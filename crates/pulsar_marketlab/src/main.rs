//! MarketLab integrated UI pipeline binary entry point.

use std::path::Path as StdPath;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::Duration;

use gpui::*;

use gpui_component::Root;
use gpui_component::{Theme, ThemeMode};
use pulsar_marketlab::fix_engine::{spawn_mock_fix_bridge, FixPlayheadClock};

use ui::telemetry_bridge::MetricsTelemetryBridge;

mod asset_path_input;
mod canvas_compose;
mod canvas_hydrate;
mod graph_compiler;
mod ohlc_chart_pane;
mod portfolio_analytics;
mod portfolio_integrator_ledger;
mod portfolio_wealth_chart;
mod session_autosave;
mod ui;
mod workspace_state;

use graph_compiler::{portfolio_wired_ta_node_ids, SharedCsvAssetPaths, SharedPipelineGraph};
use workspace_state::{
    csv_playback_is_active, finalize_csv_playback_at_eof, format_multivector_scalar,
    hot_swap_csv_playback, init_csv_playback_from_path, market_window_from_yahoo_rows,
    restart_csv_playback, send_chart_series_preload, send_playhead_set, send_playhead_set_to_last_bar,
    ta_tick_messages_for_asset, CsvAssetPlayback, PipelineSystemMessage,
    TradingSystemWorkspace, ticker_from_csv_path,
};

const CSV_PLAYBACK_INTERVAL: Duration = Duration::from_millis(400);

struct NoAssets;

impl AssetSource for NoAssets {
    fn load(&self, _path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

fn inspect_csv_path_hot_swaps(
    playbacks: &mut Vec<CsvAssetPlayback>,
    path_registry: &SharedCsvAssetPaths,
    tx: &Sender<PipelineSystemMessage>,
) -> bool {
    let ui_paths = path_registry.snapshot();
    let mut replay_required = false;
    for playback in &mut *playbacks {
        if let Some(ui_path) = ui_paths.get(&playback.node_id) {
            if hot_swap_csv_playback(playback, ui_path, tx) {
                replay_required = true;
            }
        }
    }

    for (node_id, ui_path) in ui_paths {
        if playbacks.iter().any(|playback| playback.node_id == node_id) {
            continue;
        }
        match init_csv_playback_from_path(node_id, &ui_path) {
            Ok(playback) => {
                send_chart_series_preload(tx, node_id, &playback.rows);
                let _ = tx.send(PipelineSystemMessage::StatusAlert {
                    text: format!(
                        "CSV asset feeder attached — node {node_id} bound to `{ui_path}`"
                    ),
                });
                playbacks.push(playback);
                replay_required = true;
            }
            Err(error) => {
                let _ = tx.send(PipelineSystemMessage::StatusAlert {
                    text: format!(
                        "CSV file warning — node {node_id} path `{ui_path}`: {error}"
                    ),
                });
                playbacks.push(CsvAssetPlayback {
                    node_id,
                    ticker: ticker_from_csv_path(StdPath::new(&ui_path)),
                    rows: Vec::new(),
                    cursor: 0,
                    current_active_path: ui_path,
                    reader_paused: true,
                });
            }
        }
    }

    replay_required
}

fn init_csv_asset_playbacks(
    path_registry: &SharedCsvAssetPaths,
) -> Result<Vec<CsvAssetPlayback>, String> {
    let mut playbacks = Vec::new();
    for (node_id, path) in path_registry.snapshot() {
        playbacks.push(init_csv_playback_from_path(node_id, &path)?);
    }
    Ok(playbacks)
}

/// Sequential Yahoo CSV playback at [`CSV_PLAYBACK_INTERVAL`]; pauses at EOF and replays on graph/CSV change.
fn spawn_csv_asset_feeder(
    tx: Sender<PipelineSystemMessage>,
    path_registry: SharedCsvAssetPaths,
    pipeline_graph: SharedPipelineGraph,
    fix_clock: Arc<FixPlayheadClock>,
) {
    std::thread::spawn(move || {
        let mut playbacks: Vec<CsvAssetPlayback> = match init_csv_asset_playbacks(&path_registry) {
            Ok(playbacks) => playbacks,
            Err(error) => {
                let _ = tx.send(PipelineSystemMessage::StatusAlert { text: error });
                Vec::new()
            }
        };

        for playback in &playbacks {
            send_chart_series_preload(&tx, playback.node_id, &playback.rows);
        }
        if let Some(playback) = playbacks.iter().find(|p| !p.rows.is_empty()) {
            send_playhead_set_to_last_bar(&tx, playback.rows.len());
        }
        if !playbacks.is_empty() {
            let _ = tx.send(PipelineSystemMessage::StatusAlert {
                text: format!(
                    "CSV asset feeder armed — {} Yahoo source(s) @ {}ms/tick; pauses at EOF, replays on change",
                    playbacks.len(),
                    CSV_PLAYBACK_INTERVAL.as_millis()
                ),
            });
        }

        let mut last_graph_revision = pipeline_graph.revision();
        let mut last_paths_revision = path_registry.revision();
        let mut armed_announced = !playbacks.is_empty();

        loop {
            std::thread::sleep(CSV_PLAYBACK_INTERVAL);

            let graph_revision = pipeline_graph.revision();
            let paths_revision = path_registry.revision();
            let config_changed =
                graph_revision != last_graph_revision || paths_revision != last_paths_revision;
            let registry_changed = inspect_csv_path_hot_swaps(&mut playbacks, &path_registry, &tx);

            if registry_changed && !playbacks.is_empty() && !armed_announced {
                armed_announced = true;
                let _ = tx.send(PipelineSystemMessage::StatusAlert {
                    text: format!(
                        "CSV asset feeder armed — {} Yahoo source(s) @ {}ms/tick; pauses at EOF, replays on change",
                        playbacks.len(),
                        CSV_PLAYBACK_INTERVAL.as_millis()
                    ),
                });
            }

            if config_changed || registry_changed {
                last_graph_revision = graph_revision;
                last_paths_revision = paths_revision;
                restart_csv_playback(&mut playbacks, &tx);
            }

            if !csv_playback_is_active(&playbacks) {
                continue;
            }

            let graph = pipeline_graph.snapshot();
            let portfolio_ta_filter = portfolio_wired_ta_node_ids(&graph);
            let mut epoch_end = false;
            let mut last_label = None::<String>;

            for playback in &mut playbacks {
                if playback.reader_paused || playback.rows.is_empty() {
                    continue;
                }

                let row = &playback.rows[playback.cursor];
                last_label = Some(row.date.clone());
                let window =
                    market_window_from_yahoo_rows(&playback.rows, playback.cursor + 1);
                let bar_time = pulsar_marketlab::stage_time_from_bar_date(&row.date)
                    .unwrap_or(playback.cursor as f64);
                fix_clock.set_bar_epoch(bar_time);
                let mut messages = vec![PipelineSystemMessage::TickUpdate {
                    tick_index: playback.cursor,
                    tick_label: Some(row.date.clone()),
                    node_id: playback.node_id,
                    source: playback.ticker.clone(),
                    value: format_multivector_scalar(row.close),
                }];
                messages.extend(ta_tick_messages_for_asset(
                    playback.node_id,
                    0,
                    playback.cursor,
                    Some(row.date.clone()),
                    &playback.ticker,
                    &window,
                    &graph,
                    row.close,
                    Some(&portfolio_ta_filter),
                ));
                for message in messages {
                    if tx.send(message).is_err() {
                        return;
                    }
                }

                send_playhead_set(
                    &tx,
                    playback.cursor,
                    playback.rows.len(),
                    Some(row.date.clone()),
                );

                let next_cursor = playback.cursor + 1;
                if next_cursor >= playback.rows.len() {
                    epoch_end = true;
                } else {
                    playback.cursor = next_cursor;
                }
            }

            if epoch_end {
                finalize_csv_playback_at_eof(&mut playbacks, &tx, last_label);
            }
        }
    });
}

fn main() {
    println!("Starting MarketLab Integrated UI Pipeline...");

    let csv_path_registry = SharedCsvAssetPaths::from_nodes(&[]);
    let pipeline_graph = SharedPipelineGraph::new(Vec::new(), Vec::new());

    let (pipeline_tx, pipeline_rx) = mpsc::channel::<PipelineSystemMessage>();
    let fix_clock = Arc::new(FixPlayheadClock::new());
    spawn_mock_fix_bridge(
        {
            let tx = pipeline_tx.clone();
            move |write| {
                let _ = tx.send(PipelineSystemMessage::StageSample {
                    prim_path: write.prim_path,
                    attribute: write.attribute,
                    time: write.time,
                    value: write.value,
                });
            }
        },
        fix_clock.clone(),
    );
    spawn_csv_asset_feeder(
        pipeline_tx.clone(),
        csv_path_registry.clone(),
        pipeline_graph.clone(),
        fix_clock,
    );

    Application::new().with_assets(NoAssets).run(|cx: &mut App| {
        gpui_component::init(cx);
        MetricsTelemetryBridge::set_global(cx, MetricsTelemetryBridge::default());

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(50.0), px(50.0)),
                size: size(px(1280.0), px(800.0)),
            })),
            titlebar: Some(TitlebarOptions {
                title: Some("MarketLab // Signal Generation & Node Causal Engine".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.open_window(options, move |window, cx| {
            Theme::change(ThemeMode::Dark, Some(window), cx);
            let workspace = cx.new(|cx| {
                TradingSystemWorkspace::new(
                    pipeline_rx,
                    csv_path_registry.clone(),
                    pipeline_graph.clone(),
                    cx,
                )
            });
            let workspace_for_sweep = workspace.clone();
            cx.defer(move |cx| {
                workspace_for_sweep.update(cx, |host, cx| {
                    pulsar_marketlab_ui::workspace::begin_graph_engine_timeline_sweep(
                        host,
                        workspace_for_sweep.clone(),
                        cx,
                    );
                });
            });
            cx.new(|cx| Root::new(workspace, window, cx))
        })
        .unwrap();

        println!("UI window spawned successfully. Application loop active.");
    });
}
