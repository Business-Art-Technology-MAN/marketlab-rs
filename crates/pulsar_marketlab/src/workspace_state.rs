//! Central workspace state, simulation bridge, and cross-thread messaging.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use gpui::*;
use gpui_component::input::InputState;

use crate::asset_path_input::{AssetPathInput, PathInputEvent};
use crate::graph_compiler::{
    deoverlap_canvas_columns, upstream_price_source_node_id_parts, ta_compute_for_node,
    AssetSourceType, NodeConnection, NodeType, PipelineGraphSnapshot, SharedCsvAssetPaths,
    SharedPipelineGraph, VisualNode,
};
use crate::ohlc_chart_pane::OhlcBar;
use pulsar_marketlab::trading_stage::{
    analytics_prim_path, asset_prim_path, stage_time_from_bar_date, MarketStage,
};
use pulsar_marketlab_core::ComputedAttributeStream;
use crate::asset_chart_bitmap::build_asset_chart_bitmaps;
use crate::canvas_compose::{blank_stage_usda, compose_pipeline_usda};
use crate::canvas_hydrate::hydrate_canvas_from_stage;
use crate::canvas_stage_sync::{
    apply_incremental_canvas_sync, needs_full_stage_recompose, published_node_paths,
};
use crate::session_autosave::{
    compose_session_usda, load_session_snapshot, SessionAutosaveHandle, SessionSnapshot,
};
use pulsar_marketlab::technical_analysis::{
    ta_indicator_label, MarketSeriesWindow,
};
use pulsar_marketlab_core::TimelineExecutionResult;

/// Debounce window before OTL compile, USD stage sync, and graph-engine sweeps.
pub(crate) const PIPELINE_DEBOUNCE_MS: u64 = 500;

pub const DEFAULT_CSV_ASSET_PATH: &str = "data/SPY.csv";
pub(crate) const CHART_Y_PADDING_RATIO: f32 = 0.08;
pub(crate) const CHART_Y_MIN_SPAN: f32 = 1.0;
pub(crate) const CHART_STROKE_WIDTH: f32 = 1.5;
pub(crate) const STATUS_LOG_CAP: usize = 64;
const INGESTION_POLL_INTERVAL: Duration = Duration::from_millis(16);
pub const SIM_INITIAL_CASH: f64 = 10_000.0;
const CSV_PLAYBACK_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone)]
pub enum PipelineSystemMessage {
    TickUpdate {
        tick_index: usize,
        /// When present (e.g. Yahoo `Date` column), used as `MatrixDataRow::tick` verbatim.
        tick_label: Option<String>,
        node_id: usize,
        source: String,
        value: String,
    },
    StatusAlert {
        text: String,
    },
    /// Full Date/Close series loaded from a Yahoo CSV bind or hot-swap.
    ChartSeriesPreload {
        node_id: usize,
        timestamps: Vec<String>,
        values: Vec<f32>,
        ohlc_bars: Vec<OhlcBar>,
    },
    /// Bar count for the active OHLC series (full historical length).
    ChartBarCount {
        total_bars: usize,
    },
    /// Hydrate UI `market_stage` from simulation-thread ledger or FIX bridge writes.
    StageSample {
        prim_path: String,
        attribute: String,
        time: f64,
        value: f32,
    },
}
pub(crate) fn format_tick_label(tick_index: usize) -> String {
    format!("{tick_index:04}")
}

pub(crate) fn format_multivector_scalar(value: f64) -> String {
    format!("[{value:.2}]")
}

pub(crate) fn format_percent_signed(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    format!("{:+.2}%", value * 100.0)
}

pub(crate) fn format_percent_magnitude(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    format!("{:.2}%", value * 100.0)
}

pub(crate) fn format_ratio(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.2}"),
        _ => "—".to_string(),
    }
}

pub(crate) fn format_currency(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    format!("${value:.2}")
}

#[derive(Debug, Clone)]
pub(crate) struct PortfolioDiagnosticsSnapshot {
    pub(crate) simulation_epoch: u64,
    pub(crate) tick_index: usize,
    pub(crate) tick_label: Option<String>,
    pub(crate) nav: f64,
    pub(crate) cash: f64,
    pub(crate) position_qty: f64,
    pub(crate) mark_price: f64,
    pub(crate) total_return_pct: f64,
    pub(crate) max_drawdown_pct: f64,
    pub(crate) sharpe_ratio: Option<f64>,
    pub(crate) bars_processed: usize,
    pub(crate) trade_count: u32,
    pub(crate) benchmark_return_pct: Option<f64>,
    pub(crate) excess_return_pct: Option<f64>,
    pub(crate) avg_exposure_pct: f64,
}

fn resolve_bar_ohlc(
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    close: f64,
) -> (f64, f64, f64, f64) {
    match (open, high, low) {
        (Some(open), Some(high), Some(low)) => (open, high, low, close),
        _ => (close, close, close, close),
    }
}

pub(crate) fn format_stream_indicator(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() => format_multivector_scalar(v),
        _ => "[warming up]".to_string(),
    }
}

pub(crate) fn market_window_from_yahoo_rows(rows: &[YahooCsvRow], end_exclusive: usize) -> MarketSeriesWindow {
    let mut window = MarketSeriesWindow::default();
    for row in rows.iter().take(end_exclusive) {
        let (open, high, low, close) = resolve_bar_ohlc(row.open, row.high, row.low, row.close);
        window.push_bar(open, high, low, close, row.volume.unwrap_or(0.0));
    }
    window
}

/// TA nodes whose input port is wired from a specific asset output port.
fn wired_ta_nodes_for_asset_port<'a>(
    asset_node_id: usize,
    from_port_idx: usize,
    graph: &'a PipelineGraphSnapshot,
) -> Vec<&'a VisualNode> {
    let mut wired = Vec::new();
    for connection in &graph.connections {
        if connection.from_node_id != asset_node_id || connection.from_port_idx != from_port_idx {
            continue;
        }
        let Some(ta_node) = graph.nodes.iter().find(|node| node.id == connection.to_node_id) else {
            continue;
        };
        if !ta_node.node_type.is_ta_uber_signal() {
            continue;
        }
        if wired.iter().any(|existing: &&VisualNode| existing.id == ta_node.id) {
            continue;
        }
        wired.push(ta_node);
    }
    wired
}

pub fn restart_csv_playback(
    playbacks: &mut [CsvAssetPlayback],
    tx: &Sender<PipelineSystemMessage>,
) {
    for playback in playbacks.iter_mut() {
        csv_playback_park_at_last_bar(playback);
    }
    let active_sources = playbacks
        .iter()
        .filter(|playback| !playback.rows.is_empty())
        .count();
    if let Some(playback) = playbacks.iter().find(|p| !p.rows.is_empty()) {
        send_bar_count_update(tx, playback.rows.len());
    }
    let _ = tx.send(PipelineSystemMessage::StatusAlert {
        text: format!(
            "CSV replay started — {active_sources} source(s) @ {}ms/tick",
            CSV_PLAYBACK_INTERVAL.as_millis()
        ),
    });
}

pub fn finalize_csv_playback_at_eof(
    playbacks: &mut [CsvAssetPlayback],
    tx: &Sender<PipelineSystemMessage>,
    last_label: Option<String>,
) {
    for playback in playbacks.iter_mut() {
        if !playback.rows.is_empty() {
            playback.reader_paused = true;
            let _last_index = playback.rows.len().saturating_sub(1);
            let _ = tx.send(PipelineSystemMessage::ChartBarCount {
                total_bars: playback.rows.len(),
            });
        }
    }
    let _ = tx.send(PipelineSystemMessage::StatusAlert {
        text: format!(
            "CSV playback complete — change graph to replay{}",
            last_label
                .map(|date| format!(" (last bar {date})"))
                .unwrap_or_default()
        ),
    });
}

pub fn send_bar_count_update(tx: &Sender<PipelineSystemMessage>, total_bars: usize) {
    let _ = tx.send(PipelineSystemMessage::ChartBarCount { total_bars });
}

/// Notify the UI of loaded CSV bar count (terminal-bar display uses `total_bars - 1`).
pub fn send_playhead_set_to_last_bar(tx: &Sender<PipelineSystemMessage>, total_bars: usize) {
    send_bar_count_update(tx, total_bars);
}

/// Non-realtime CSV sources stay parked on the last bar until replay is requested.
pub fn csv_playback_park_at_last_bar(playback: &mut CsvAssetPlayback) {
    if playback.rows.is_empty() {
        playback.cursor = 0;
        playback.reader_paused = true;
    } else {
        playback.cursor = playback.rows.len() - 1;
        playback.reader_paused = true;
    }
}

pub fn csv_playback_is_active(playbacks: &[CsvAssetPlayback]) -> bool {
    playbacks
        .iter()
        .any(|playback| !playback.reader_paused && !playback.rows.is_empty())
}

pub fn ta_tick_messages_for_asset(
    asset_node_id: usize,
    from_port_idx: usize,
    tick_index: usize,
    tick_label: Option<String>,
    asset_source: &str,
    window: &MarketSeriesWindow,
    graph: &PipelineGraphSnapshot,
    _price: f64,
    portfolio_ta_filter: Option<&HashSet<usize>>,
) -> Vec<PipelineSystemMessage> {
    let mut messages = Vec::new();
    for node in wired_ta_nodes_for_asset_port(asset_node_id, from_port_idx, graph) {
        if portfolio_ta_filter.is_some_and(|allowed| !allowed.contains(&node.id)) {
            continue;
        }
        let Some(indicator_id) = node.overlay_algorithm() else {
            continue;
        };
        let label = ta_indicator_label(indicator_id).unwrap_or(indicator_id);
        let value = ta_compute_for_node(node, window);
        messages.push(PipelineSystemMessage::TickUpdate {
            tick_index,
            tick_label: tick_label.clone(),
            node_id: node.id,
            source: format!("{asset_source} ({label})"),
            value: format_stream_indicator(value),
        });
    }
    messages
}
#[derive(Debug, Clone, Default)]
pub struct ChartHistoryBuffer {
    pub timestamps: Vec<String>,
    pub values: Vec<f32>,
}

impl ChartHistoryBuffer {
    pub fn replace_series(&mut self, timestamps: Vec<String>, values: Vec<f32>) {
        debug_assert_eq!(timestamps.len(), values.len());
        self.timestamps = timestamps;
        self.values = values;
    }

    pub fn push_sample(&mut self, timestamp: String, value: f32) {
        self.timestamps.push(timestamp);
        self.values.push(value);
    }
}

pub(crate) fn parse_chart_scalar_value(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    inner.trim().parse::<f32>().ok()
}

/// Map a Yahoo `Date` string (`YYYY-MM-DD`) to a monotonic chart X coordinate.
pub(crate) fn parse_chart_date_ordinal(date: &str) -> Option<f32> {
    let (year, rest) = date.trim().split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let y: f32 = year.parse().ok()?;
    let m: f32 = month.parse().ok()?;
    let d: f32 = day.parse().ok()?;
    Some(y * 372.0 + m * 31.0 + d)
}

pub fn chart_buffer_from_csv_rows(rows: &[YahooCsvRow]) -> ChartHistoryBuffer {
    ChartHistoryBuffer {
        timestamps: rows.iter().map(|row| row.date.clone()).collect(),
        values: rows.iter().map(|row| row.close as f32).collect(),
    }
}

pub fn ohlc_bars_from_csv_rows(rows: &[YahooCsvRow]) -> Vec<OhlcBar> {
    rows.iter().filter_map(YahooCsvRow::to_ohlc_bar).collect()
}

pub fn yahoo_rows_from_ohlc_bars(bars: &[OhlcBar]) -> Vec<YahooCsvRow> {
    bars
        .iter()
        .map(|bar| YahooCsvRow {
            date: bar.date.clone(),
            open: Some(bar.open),
            high: Some(bar.high),
            low: Some(bar.low),
            close: bar.close,
            volume: None,
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct AssetCsvBundle {
    pub chart: ChartHistoryBuffer,
    pub ohlc_bars: Vec<OhlcBar>,
    pub close_series: std::sync::Arc<[f64]>,
}

pub fn close_series_from_bars(bars: &[OhlcBar]) -> std::sync::Arc<[f64]> {
    bars.iter()
        .map(|bar| bar.close)
        .collect::<Vec<_>>()
        .into_boxed_slice()
        .into()
}

pub fn load_asset_csv_bundle(path: &str) -> Result<AssetCsvBundle, String> {
    let (_, rows) = load_yahoo_finance_csv(path)?;
    let ohlc_bars = ohlc_bars_from_csv_rows(&rows);
    let close_series = close_series_from_bars(&ohlc_bars);
    Ok(AssetCsvBundle {
        chart: chart_buffer_from_csv_rows(&rows),
        ohlc_bars,
        close_series,
    })
}

pub fn preload_asset_csv_bundles_from_nodes(nodes: &[VisualNode]) -> HashMap<usize, AssetCsvBundle> {
    let mut history = HashMap::new();
    for node in nodes {
        if !node.node_type.displays_price_chart() {
            continue;
        }
        let Some(AssetSourceType::Csv { path }) = &node.asset_source else {
            continue;
        };
        if let Ok(bundle) = load_asset_csv_bundle(path) {
            history.insert(node.id, bundle);
        }
    }
    history
}

pub fn preload_asset_charts_from_nodes(nodes: &[VisualNode]) -> HashMap<usize, ChartHistoryBuffer> {
    preload_asset_csv_bundles_from_nodes(nodes)
        .into_iter()
        .map(|(id, bundle)| (id, bundle.chart))
        .collect()
}

pub fn preload_asset_ohlc_from_nodes(nodes: &[VisualNode]) -> HashMap<usize, Vec<OhlcBar>> {
    preload_asset_csv_bundles_from_nodes(nodes)
        .into_iter()
        .filter_map(|(id, bundle)| {
            if bundle.ohlc_bars.is_empty() {
                None
            } else {
                Some((id, bundle.ohlc_bars))
            }
        })
        .collect()
}

fn analytics_indicator_id(node: &VisualNode) -> String {
    node.overlay_algorithm()
        .map(str::to_string)
        .unwrap_or_else(|| format!("ta_{}", node.id))
}

fn stream_value_at_bar(
    streams: &[ComputedAttributeStream],
    prim_path: &str,
    attribute: &str,
    bar_index: usize,
) -> Option<f64> {
    streams
        .iter()
        .find(|stream| stream.prim_path == prim_path && stream.attribute == attribute)
        .and_then(|stream| stream.values.get(bar_index).copied())
}

fn build_inspector_rows_from_streams(
    streams: &[ComputedAttributeStream],
    nodes: &[VisualNode],
    bar_index: usize,
    tick: &str,
    resolve_prim: impl Fn(&VisualNode) -> Option<String>,
) -> Vec<MatrixDataRow> {
    let mut rows = Vec::new();
    for node in nodes {
        let Some(prim_path) = resolve_prim(node) else {
            continue;
        };
        match &node.node_type {
            NodeType::AssetAdaptor { .. }
                if matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) =>
            {
                let Some(close) =
                    stream_value_at_bar(streams, &prim_path, "outputs:price", bar_index)
                else {
                    continue;
                };
                rows.push(MatrixDataRow {
                    tick: tick.to_string(),
                    asset: node.name.clone(),
                    grade_type: format!("{:?}", node.grade),
                    multivector_value: format!("[{close:.2}]"),
                    associated_node_id: Some(node.id),
                });
            }
            NodeType::TaUberSignal { .. } | NodeType::OtlShader { .. } => {
                let indicator_id = analytics_indicator_id(node);
                let value = stream_value_at_bar(streams, &prim_path, "outputs:result", bar_index)
                    .or_else(|| {
                        stream_value_at_bar(streams, &prim_path, "outputs:signal", bar_index)
                    });
                let Some(value) = value else {
                    continue;
                };
                rows.push(MatrixDataRow {
                    tick: tick.to_string(),
                    asset: format!("{} ({indicator_id})", node.name),
                    grade_type: format!("{:?}", node.grade),
                    multivector_value: format!("[{value:.4}]"),
                    associated_node_id: Some(node.id),
                });
            }
            _ => {}
        }
    }
    rows
}

pub(crate) fn hydrate_market_stage_from_ohlc(
    stage: &mut MarketStage,
    ticker: &str,
    bars: &[OhlcBar],
) {
    let Ok(prim) = asset_prim_path(ticker) else {
        return;
    };
    for bar in bars {
        let Some(time) = stage_time_from_bar_date(&bar.date) else {
            continue;
        };
        let _ = stage.set_sample(&prim, "open", time, bar.open as f32);
        let _ = stage.set_sample(&prim, "high", time, bar.high as f32);
        let _ = stage.set_sample(&prim, "low", time, bar.low as f32);
        let _ = stage.set_sample(&prim, "close", time, bar.close as f32);
    }
}

pub(crate) fn hydrate_market_stage_from_workspace(
    stage: &mut MarketStage,
    nodes: &[VisualNode],
    ohlc_by_node: &HashMap<usize, Vec<OhlcBar>>,
) {
    stage.prims.clear();
    for node in nodes {
        if !node.node_type.is_asset_adaptor() {
            continue;
        }
        if !matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) {
            continue;
        }
        let Some(bars) = ohlc_by_node.get(&node.id) else {
            continue;
        };
        hydrate_market_stage_from_ohlc(stage, &node.name, bars);
    }
}

pub(crate) fn stage_time_for_bar_index(bars: &[OhlcBar], index: usize) -> Option<f64> {
    bars.get(index).and_then(|bar| stage_time_from_bar_date(&bar.date))
}

pub fn send_chart_series_preload(
    tx: &Sender<PipelineSystemMessage>,
    node_id: usize,
    rows: &[YahooCsvRow],
) {
    let timestamps: Vec<String> = rows.iter().map(|row| row.date.clone()).collect();
    let values: Vec<f32> = rows.iter().map(|row| row.close as f32).collect();
    let ohlc_bars = ohlc_bars_from_csv_rows(rows);
    let _ = tx.send(PipelineSystemMessage::ChartSeriesPreload {
        node_id,
        timestamps,
        values,
        ohlc_bars,
    });
}
#[derive(Debug, Clone)]
pub struct YahooCsvRow {
    pub date: String,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: f64,
    pub volume: Option<f64>,
}

impl YahooCsvRow {
    fn to_ohlc_bar(&self) -> Option<OhlcBar> {
        Some(OhlcBar {
            date: self.date.clone(),
            open: self.open?,
            high: self.high?,
            low: self.low?,
            close: self.close,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MatrixDataRow {
    pub tick: String,
    pub asset: String,
    pub grade_type: String,
    pub multivector_value: String,
    #[allow(dead_code)]
    pub associated_node_id: Option<usize>,
}
pub struct TradingSystemWorkspace {
    pub nodes: Vec<VisualNode>,
    pub connections: Vec<NodeConnection>,
    pub inspector_data: Vec<MatrixDataRow>,
    pub pipeline_status_log: Vec<String>,
    pub csv_path_registry: SharedCsvAssetPaths,
    pub pipeline_graph: SharedPipelineGraph,
    pub asset_chart_history: HashMap<usize, ChartHistoryBuffer>,
    pub(crate) asset_chart_bitmaps: HashMap<usize, Arc<gpui::RenderImage>>,
    pub asset_ohlc_history: HashMap<usize, Vec<OhlcBar>>,
    pub(crate) asset_close_series: HashMap<usize, std::sync::Arc<[f64]>>,
    /// Phase B Layer 1 market stage (path-addressable time-sampled attributes).
    pub(crate) market_stage: MarketStage,
    pub selected_node_id: Option<usize>,
    pub active_drag_node_id: Option<usize>,
    /// Inspector/stage selection refresh deferred until node drag completes.
    pub(crate) defer_inspector_sync_after_drag: bool,
    /// Coalesce canvas pan/drag repaints to one defer tick per frame.
    pub(crate) canvas_interaction_repaint_pending: bool,
    /// Coalesce FIX/CSV ingestion repaints to one defer tick per frame.
    pub(crate) pipeline_ingestion_repaint_pending: bool,
    /// OTL inline uniform inputs need (re)binding after graph edits.
    pub(crate) otl_shader_inputs_stale: bool,
    /// Coalesce post-sweep inspector/metrics sync to the next frame.
    pub(crate) view_window_sync_pending: bool,
    /// Debounce portfolio weight overlay writes after sweeps.
    pub(crate) portfolio_weights_overlay_pending: bool,
    pub drag_offset: Point<Pixels>,
    pub canvas_origin: Point<Pixels>,
    pub active_wire_source: Option<(usize, usize)>,
    pub active_mouse_pos: Point<Pixels>,
    pub context_menu_pos: Option<Point<Pixels>>,
    pub pan_offset: Point<Pixels>,
    pub zoom_scale: f32,
    /// Last measured node-canvas viewport (for fit-to-graph).
    pub(crate) canvas_viewport_size: Size<Pixels>,
    pub is_panning: bool,
    pub last_pan_mouse_pos: Point<Pixels>,
    /// Active category shelf tab in the TA indicator picker.
    pub(crate) ta_inspector_category: Option<String>,
    /// Latest Layer 2 portfolio diagnostics from the simulation ledger.
    pub(crate) portfolio_diagnostics: Option<PortfolioDiagnosticsSnapshot>,
    /// Ignore stale portfolio metric frames from prior CSV playback epochs.
    pub(crate) portfolio_metrics_epoch: u64,
    /// Number of OHLC bars in the active historical series.
    pub(crate) historical_bar_count: usize,
    /// Editable CSV path field for the selected asset node.
    pub(crate) asset_path_input: Entity<AssetPathInput>,
    /// Cached bounds for the TA lookback slider track (inspector sidebar).
    pub(crate) ta_lookback_slider_bounds: Option<Bounds<Pixels>>,
    /// True while dragging the TA lookback slider (USD commits deferred to mouse-up).
    pub(crate) ta_lookback_scrubbing: bool,
    /// Shared MVU context powering the stage ledger explorer grid (unified USD stage).
    pub(crate) workspace_context: Entity<pulsar_marketlab_ui::workspace::WorkspaceContext>,
    /// Lazily initialized OTL script editor bound to the selected shader node.
    pub(crate) otl_script_input: Option<Entity<InputState>>,
    pub(crate) otl_script_node_id: Option<usize>,
    /// Editable display name (`info:user_label`) for the selected canvas node.
    pub(crate) node_label_input: Option<Entity<InputState>>,
    pub(crate) node_label_node_id: Option<usize>,
    pub(crate) otl_shader_param_inputs: HashMap<(usize, String), Entity<InputState>>,
    /// Dedicated OTL editor tab buffer (commits on compile, not per keystroke).
    pub(crate) otl_editor_input: Option<Entity<InputState>>,
    pub(crate) otl_editor_binding: Option<String>,
    pub(crate) otl_compile_status: String,
    pub(crate) otl_compile_inflight: bool,
    pub(crate) active_workspace_tab: pulsar_marketlab_ui::workspace::WorkspaceTab,
    /// Persisted workstation splitter shares.
    pub(crate) split_layout: pulsar_marketlab_ui::workspace::WorkstationSplitLayout,
    pub(crate) split_container_bounds: Option<Bounds<Pixels>>,
    pub(crate) upper_row_bounds: Option<Bounds<Pixels>>,
    pub(crate) active_split_drag: Option<pulsar_marketlab_ui::workspace::SplitHandle>,
    pub(crate) stage_tree_columns: pulsar_marketlab_ui::workspace::StageTreeColumnLayout,
    pub(crate) stage_tree_header_bounds: Option<Bounds<Pixels>>,
    pub(crate) active_stage_tree_column_drag:
        Option<(pulsar_marketlab_ui::workspace::StageTreeColumnHandle, f32)>,
    pub(crate) graph_engine_last_compiled_generation: u64,
    pub(crate) graph_engine_last_compile_ms: u64,
    pub(crate) graph_engine_recompile_inflight: bool,
    pub(crate) graph_engine_recompile_pending: bool,
    pub(crate) graph_engine_asset_data_epoch: u64,
    pub(crate) graph_engine_last_swept_asset_epoch: u64,
    pub(crate) graph_engine_compile_error: Option<String>,
    /// Suppresses reactive observers while the workspace entity is still being constructed.
    pub(crate) bootstrapping: bool,
    /// On-disk path for the active USD root layer (Save / Save As target).
    pub(crate) usd_document_path: Option<std::path::PathBuf>,
    /// Sub-canvas environment tabs (root + aggregator drill-downs).
    pub(crate) canvas_tabs: Vec<pulsar_marketlab_ui::workspace::CanvasEnvironmentTab>,
    pub(crate) active_canvas_tab: usize,
    /// Detect double-clicks on aggregator node headers for sub-canvas drill-down.
    pub(crate) last_node_header_click: Option<(usize, std::time::Instant)>,
    /// Collapsed branches in the stage hierarchy tree-table.
    pub(crate) collapsed_tree_paths: HashSet<String>,
    /// Collapsible shelf state for Context Tower and Stage Composer panels.
    pub(crate) workstation_shelves: pulsar_marketlab_ui::workspace::WorkstationShelfState,
    /// Persistent topology dopesheet disclosure layout (isolated from engine sweeps).
    pub(crate) dopesheet_ui_state: pulsar_marketlab_ui::workspace::DopesheetUiState,
    /// Cached logical strategy tree — refreshed only when stage topology or sweep generation changes.
    pub(crate) topology_tree_cache: Vec<pulsar_marketlab_ui::workspace::LogicalTreeNode>,
    pub(crate) topology_tree_cache_stage_generation: u64,
    pub(crate) topology_tree_cache_sweep_generation: u64,
    /// Last observed unified selection generation from [`WorkspaceContext`].
    pub(crate) last_ui_selection_generation: u64,
    /// Inline lookback [`NumberInput`] states keyed by OTL node id.
    pub(crate) node_lookback_inputs: HashMap<usize, Entity<InputState>>,
    /// Guards one-time inline lookback input construction.
    pub(crate) node_lookback_inputs_ready: bool,
    /// Pre-computed portfolio wealth timelines keyed by USD prim path.
    pub(crate) portfolio_timeline_cache: HashMap<String, crate::portfolio_wealth_chart::PortfolioWealthChartSeries>,
    /// Stacked allocation weights keyed by portfolio prim path.
    pub(crate) portfolio_allocation_cache:
        HashMap<String, crate::portfolio_wealth_chart::PortfolioAllocationChartSeries>,
    /// Token streams (`outputs:weights`) from the last graph sweep.
    pub(crate) graph_engine_token_streams: Vec<pulsar_marketlab_core::ComputedTokenStream>,
    /// Graph-engine portfolio integration keyed by stage prim path.
    pub(crate) graph_engine_portfolio_results:
        HashMap<String, pulsar_marketlab_core::PortfolioIntegrationResult>,
    /// Pre-computed attribute streams from the last vectorized timeline execution.
    pub(crate) graph_engine_streams: Vec<ComputedAttributeStream>,
    /// Per-portfolio diagnostics derived from graph-engine sweeps.
    pub(crate) portfolio_diagnostics_cache: HashMap<String, PortfolioDiagnosticsSnapshot>,
    /// Integrator tracking matrix rows keyed by portfolio prim path.
    pub(crate) portfolio_ledger_cache: HashMap<String, Arc<crate::portfolio_integrator_ledger::PortfolioIntegratorLedger>>,
    /// Active quick-filter for the integrator ledger spreadsheet.
    pub(crate) portfolio_ledger_filter: crate::portfolio_integrator_ledger::IntegratorLedgerFilter,
    /// Overlay toggles for the portfolio analytics wealth chart.
    pub(crate) portfolio_chart_overlays: crate::portfolio_wealth_chart::PortfolioChartOverlayToggles,
    /// Debounced background session autosave writer.
    pub(crate) session_autosave: SessionAutosaveHandle,
    /// Monotonic revision bumped on graph mutations for autosave coalescing.
    pub(crate) session_autosave_revision: u64,
    /// Graph-engine metrics cache needs publishing to [`MetricsTelemetryBridge`].
    pub(crate) metrics_telemetry_dirty: bool,
    /// Resolved prim paths after the last successful canvas → USD publish.
    pub(crate) last_published_node_paths: HashMap<usize, String>,
    /// Debounce coalescing for incremental stage sync (Milestone 3).
    pub(crate) canvas_stage_sync_revision: u64,
    pub(crate) canvas_stage_sync_debounce_scheduled: bool,
    /// Debounce TA hyperparameter edits before USD/engine invalidation.
    pub(crate) ta_hyperparam_revision: u64,
    pub(crate) ta_hyperparam_debounce_scheduled: bool,
    /// Stage sync / engine sweep deferred until continuous interaction ends.
    pub(crate) pipeline_sync_deferred: bool,
    /// Timeline sweep result held until canvas or slider interaction completes.
    pub(crate) pending_timeline_result: Option<TimelineExecutionResult>,
    /// Pre-built UI snapshot paired with [`pending_timeline_result`].
    pub(crate) pending_ui_snapshot: Option<std::sync::Arc<crate::graph_ui_snapshot::GraphUiSnapshot>>,
    /// Double-buffer read model for charts, inspector, and telemetry (Arc swap after sweeps).
    pub(crate) graph_ui_snapshot: Option<std::sync::Arc<crate::graph_ui_snapshot::GraphUiSnapshot>>,
    /// Cached stage graph snapshot keyed by [`SharedPipelineGraph::revision`].
    pub(crate) stage_graph_snapshot_cache: std::sync::Mutex<Option<(u64, std::sync::Arc<pulsar_marketlab_core::StageGraphSnapshot>)>>,
    /// Cached pipeline validation snapshot keyed by graph revision.
    pub(crate) cached_pipeline_snapshot: Option<(u64, crate::graph_compiler::PipelineGraphSnapshot)>,
    /// Debounced background ledger rebuild (off hot publish path).
    pub(crate) ledger_sync_debounce_scheduled: bool,
    pub(crate) usd_commit_generation: u64,
    /// Skip rebuilding the historical timeline map when bar labels are unchanged.
    pub(crate) cached_timeline_map_key: Option<(usize, String, String)>,
    /// True while a background full-USD recompose is in flight.
    pub(crate) canvas_stage_full_recompose_inflight: bool,
    /// Reused across sweeps when graph topology is unchanged.
    pub(crate) graph_engine_cached: Option<pulsar_marketlab_core::MarketLabGraphEngine>,
    pub(crate) graph_engine_cached_generation: u64,
}

fn workspace_tab_token(tab: pulsar_marketlab_ui::workspace::WorkspaceTab) -> String {
    match tab {
        pulsar_marketlab_ui::workspace::WorkspaceTab::ParamInspector => {
            "param_inspector".to_string()
        }
        pulsar_marketlab_ui::workspace::WorkspaceTab::OtlEditor => "otl_editor".to_string(),
    }
}

fn workspace_tab_from_token(token: &str) -> pulsar_marketlab_ui::workspace::WorkspaceTab {
    match token {
        "otl_editor" => pulsar_marketlab_ui::workspace::WorkspaceTab::OtlEditor,
        _ => pulsar_marketlab_ui::workspace::WorkspaceTab::ParamInspector,
    }
}

fn blank_workspace_context() -> pulsar_marketlab_ui::workspace::WorkspaceContext {
    pulsar_marketlab_ui::workspace::WorkspaceContext::from_usda_text(&blank_stage_usda())
        .unwrap_or_default()
}

impl TradingSystemWorkspace {
    pub fn new(
        rx: Receiver<PipelineSystemMessage>,
        csv_path_registry: SharedCsvAssetPaths,
        pipeline_graph: SharedPipelineGraph,
        cx: &mut Context<Self>,
    ) -> Self {
        let asset_path_input = cx.new(|cx| AssetPathInput::new("", cx));
        cx.subscribe(
            &asset_path_input,
            |this, _, event: &PathInputEvent, cx| {
                this.on_asset_path_input_event(event, cx);
            },
        )
        .detach();


        let workspace_context = cx.new(|_| blank_workspace_context());

        let nodes: Vec<VisualNode> = Vec::new();
        let asset_ohlc_history = HashMap::new();
        let market_stage = MarketStage::new();
        let mut workspace = Self {
            nodes: nodes.clone(),
            connections: Vec::new(),
            inspector_data: Vec::new(),
            pipeline_status_log: vec![
                "Blank document — empty canvas and USD stage.".to_string(),
            ],
            csv_path_registry,
            pipeline_graph,
            asset_chart_history: HashMap::new(),
            asset_chart_bitmaps: HashMap::new(),
            asset_ohlc_history,
            asset_close_series: HashMap::new(),
            market_stage,
            selected_node_id: None,
            active_drag_node_id: None,
            defer_inspector_sync_after_drag: false,
            canvas_interaction_repaint_pending: false,
            pipeline_ingestion_repaint_pending: false,
            otl_shader_inputs_stale: true,
            view_window_sync_pending: false,
            portfolio_weights_overlay_pending: false,
            drag_offset: point(px(0.0), px(0.0)),
            canvas_origin: point(px(0.0), px(0.0)),
            active_wire_source: None,
            active_mouse_pos: point(px(0.0), px(0.0)),
            context_menu_pos: None,
            pan_offset: point(px(0.0), px(0.0)),
            zoom_scale: 1.0,
            canvas_viewport_size: size(px(960.0), px(640.0)),
            is_panning: false,
            last_pan_mouse_pos: point(px(0.0), px(0.0)),
            ta_inspector_category: None,
            portfolio_diagnostics: None,
            portfolio_metrics_epoch: 0,
            historical_bar_count: 0,
            asset_path_input,
            ta_lookback_slider_bounds: None,
            ta_lookback_scrubbing: false,
            workspace_context,
            otl_script_input: None,
            otl_script_node_id: None,
            node_label_input: None,
            node_label_node_id: None,
            otl_editor_input: None,
            otl_editor_binding: None,
            otl_shader_param_inputs: HashMap::new(),
            otl_compile_status: String::new(),
            otl_compile_inflight: false,
            active_workspace_tab: pulsar_marketlab_ui::workspace::WorkspaceTab::default(),
            split_layout: pulsar_marketlab_ui::workspace::WorkstationSplitLayout::default(),
            split_container_bounds: None,
            upper_row_bounds: None,
            active_split_drag: None,
            stage_tree_columns: pulsar_marketlab_ui::workspace::StageTreeColumnLayout::default(),
            stage_tree_header_bounds: None,
            active_stage_tree_column_drag: None,
            graph_engine_last_compiled_generation: u64::MAX,
            graph_engine_last_compile_ms: 0,
            graph_engine_recompile_inflight: false,
            graph_engine_recompile_pending: false,
            graph_engine_asset_data_epoch: 0,
            graph_engine_last_swept_asset_epoch: 0,
            graph_engine_compile_error: None,
            bootstrapping: true,
            usd_document_path: None,
            canvas_tabs: vec![pulsar_marketlab_ui::workspace::CanvasEnvironmentTab::root()],
            active_canvas_tab: 0,
            last_node_header_click: None,
            collapsed_tree_paths: HashSet::new(),
            workstation_shelves: {
                let mut shelves = pulsar_marketlab_ui::workspace::WorkstationShelfState::default();
                shelves.set_expanded(
                    pulsar_marketlab_ui::workspace::WorkstationShelfId::TowerOtlEditor,
                    false,
                );
                shelves.set_expanded(
                    pulsar_marketlab_ui::workspace::WorkstationShelfId::TowerStageComposer,
                    false,
                );
                shelves
            },
            dopesheet_ui_state: pulsar_marketlab_ui::workspace::DopesheetUiState::default(),
            topology_tree_cache: Vec::new(),
            topology_tree_cache_stage_generation: u64::MAX,
            topology_tree_cache_sweep_generation: u64::MAX,
            last_ui_selection_generation: 0,
            node_lookback_inputs: HashMap::new(),
            node_lookback_inputs_ready: false,
            portfolio_timeline_cache: HashMap::new(),
            portfolio_allocation_cache: HashMap::new(),
            graph_engine_token_streams: Vec::new(),
            graph_engine_portfolio_results: HashMap::new(),
            graph_engine_streams: Vec::new(),
            portfolio_diagnostics_cache: HashMap::new(),
            portfolio_ledger_cache: HashMap::new(),
            portfolio_ledger_filter: crate::portfolio_integrator_ledger::IntegratorLedgerFilter::default(),
            portfolio_chart_overlays: crate::portfolio_wealth_chart::PortfolioChartOverlayToggles::with_defaults(),
            session_autosave: SessionAutosaveHandle::new(),
            session_autosave_revision: 0,
            metrics_telemetry_dirty: false,
            last_published_node_paths: HashMap::new(),
            canvas_stage_sync_revision: 0,
            canvas_stage_sync_debounce_scheduled: false,
            ta_hyperparam_revision: 0,
            ta_hyperparam_debounce_scheduled: false,
            pipeline_sync_deferred: false,
            pending_timeline_result: None,
            pending_ui_snapshot: None,
            graph_ui_snapshot: None,
            stage_graph_snapshot_cache: std::sync::Mutex::new(None),
            cached_pipeline_snapshot: None,
            ledger_sync_debounce_scheduled: false,
            usd_commit_generation: 0,
            cached_timeline_map_key: None,
            canvas_stage_full_recompose_inflight: false,
            graph_engine_cached: None,
            graph_engine_cached_generation: u64::MAX,
        };

        workspace.sync_historical_bar_count();
        workspace.sync_pipeline_graph(cx);
        workspace.spawn_pipeline_ingestion_worker(rx, cx);
        pulsar_marketlab_ui::workspace::install_graph_engine_invalidation_worker(
            &workspace.workspace_context,
            cx,
        );
        cx.observe(&workspace.workspace_context, |workspace, context, cx| {
            if workspace.bootstrapping {
                return;
            }
            let ctx = context.read(cx);
            let generation = ctx.ui_selection_generation();
            if generation != workspace.last_ui_selection_generation {
                workspace.last_ui_selection_generation = generation;
                workspace.sync_canvas_selection_from_context(ctx);
                workspace.sync_inspector_from_selection(cx);
                cx.notify();
            }
        })
        .detach();
        workspace.schedule_workspace_ledger_sync_deferred(cx);
        workspace.bootstrapping = false;
        workspace.restore_session_from_autosave_if_present(cx);
        cx.on_app_quit(|workspace, cx| {
            workspace.flush_session_autosave_sync();
            cx.notify();
            async {}
        })
        .detach();
        workspace
    }

    pub(crate) fn build_session_snapshot(&self) -> SessionSnapshot {
        SessionSnapshot::new(
            self.usd_document_path.clone(),
            self.nodes.clone(),
            self.connections.clone(),
            self.selected_node_id,
            [f32::from(self.pan_offset.x), f32::from(self.pan_offset.y)],
            self.zoom_scale,
            self.split_layout.stage_share,
            self.split_layout.inspector_share,
            workspace_tab_token(self.active_workspace_tab),
        )
    }

    pub(crate) fn schedule_session_autosave(&mut self) {
        if self.bootstrapping || cfg!(test) {
            return;
        }
        self.session_autosave_revision = self.session_autosave_revision.saturating_add(1);
        let revision = self.session_autosave_revision;
        let snapshot = self.build_session_snapshot();
        self.session_autosave.schedule(revision, snapshot);
    }

    pub(crate) fn flush_session_autosave_sync(&mut self) {
        if cfg!(test) {
            return;
        }
        let snapshot = self.build_session_snapshot();
        let usda = compose_session_usda(&snapshot.nodes, &snapshot.connections);
        if let Err(error) = self.session_autosave.flush_sync(snapshot, &usda) {
            self.push_status_log(format!(
                "Session autosave flush failed ({}): {error}",
                self.session_autosave.dir().display()
            ));
        }
    }

    pub(crate) fn restore_session_from_autosave_if_present(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let dir = self.session_autosave.dir().to_path_buf();
        let Ok(Some(snapshot)) = load_session_snapshot(&dir) else {
            return;
        };
        if snapshot.nodes.is_empty() && snapshot.connections.is_empty() {
            return;
        }

        self.bootstrapping = true;
        self.nodes = snapshot.nodes;
        deoverlap_canvas_columns(&mut self.nodes);
        self.connections = snapshot.connections;
        self.selected_node_id = snapshot.selected_node_id;
        self.pan_offset = point(px(snapshot.pan_offset[0]), px(snapshot.pan_offset[1]));
        self.zoom_scale = snapshot.zoom_scale;
        self.split_layout = pulsar_marketlab_ui::workspace::WorkstationSplitLayout {
            stage_share: snapshot.stage_share,
            inspector_share: snapshot.inspector_share,
            bottom_share: pulsar_marketlab_ui::workspace::WorkstationSplitLayout::default()
                .bottom_share,
        }
        .clamp();
        self.active_workspace_tab = workspace_tab_from_token(&snapshot.active_workspace_tab);
        self.usd_document_path = snapshot
            .usd_document_path
            .map(PathBuf::from);

        self.node_lookback_inputs.clear();
        self.node_lookback_inputs_ready = false;
        self.csv_path_registry.replace_from_nodes(&self.nodes);
        self.pipeline_status_log.push(format!(
            "Restored autosaved session from `{}` ({} nodes, {} wires)",
            dir.display(),
            self.nodes.len(),
            self.connections.len()
        ));

        self.sync_historical_bar_count();
        self.sync_pipeline_graph(cx);
        self.preload_bound_csv_assets(cx);
        self.bootstrapping = false;
        cx.notify();
    }

    /// Rasterize node sparklines off the UI thread after CSV series are loaded.
    pub(crate) fn rebuild_asset_chart_bitmaps_async(&mut self, cx: &mut Context<Self>) {
        let history = self.asset_chart_history.clone();
        cx.spawn(async move |this, cx| {
            let bitmaps = cx
                .background_executor()
                .spawn(async move { build_asset_chart_bitmaps(&history) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.asset_chart_bitmaps = bitmaps;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn apply_asset_csv_bundle(
        &mut self,
        node_id: usize,
        bundle: AssetCsvBundle,
        cx: &mut Context<Self>,
    ) {
        self.asset_chart_history.insert(node_id, bundle.chart);
        if bundle.ohlc_bars.is_empty() {
            self.asset_ohlc_history.remove(&node_id);
            self.asset_close_series.remove(&node_id);
        } else {
            self.asset_ohlc_history
                .insert(node_id, bundle.ohlc_bars.clone());
            self.asset_close_series
                .insert(node_id, std::sync::Arc::clone(&bundle.close_series));
        }
        self.rebuild_asset_chart_bitmaps_async(cx);
        self.sync_historical_bar_count();
        self.request_graph_engine_sweep(cx);
        self.sync_view_window(cx);
    }

    /// Load one CSV off the UI thread and merge into caches.
    pub(crate) fn load_asset_csv_for_node_async(
        &mut self,
        node_id: usize,
        path: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let path_for_log = path.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { load_asset_csv_bundle(&path).map(|bundle| (path, bundle)) })
                .await;
            let _ = this.update(cx, |workspace, cx| match loaded {
                Ok((path, bundle)) => {
                    workspace.apply_asset_csv_bundle(node_id, bundle, cx);
                    workspace.push_status_log(format!(
                        "CSV Asset bound — node {node_id} loaded `{path}`"
                    ));
                    cx.notify();
                }
                Err(error) => {
                    workspace.push_status_log(format!(
                        "Chart reload failed for `{path_for_log}`: {error}"
                    ));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Load bound CSV files into chart/OHLC caches and refresh the graph engine sweep.
    pub(crate) fn preload_bound_csv_assets(&mut self, cx: &mut Context<Self>) {
        self.csv_path_registry.replace_from_nodes(&self.nodes);
        let nodes = self.nodes.clone();
        cx.spawn(async move |this, cx| {
            let bundles = cx
                .background_executor()
                .spawn(async move { preload_asset_csv_bundles_from_nodes(&nodes) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.asset_chart_history = bundles
                    .iter()
                    .map(|(id, bundle)| (*id, bundle.chart.clone()))
                    .collect();
                workspace.asset_ohlc_history = bundles
                    .iter()
                    .filter_map(|(id, bundle)| {
                        if bundle.ohlc_bars.is_empty() {
                            None
                        } else {
                            Some((*id, bundle.ohlc_bars.clone()))
                        }
                    })
                    .collect();
                workspace.asset_close_series = bundles
                    .iter()
                    .filter_map(|(id, bundle)| {
                        if bundle.ohlc_bars.is_empty() {
                            None
                        } else {
                            Some((*id, std::sync::Arc::clone(&bundle.close_series)))
                        }
                    })
                    .collect();
                workspace.rebuild_asset_chart_bitmaps_async(cx);

                let mut loaded = 0usize;
                let mut failed_paths = Vec::new();
                for node in &workspace.nodes {
                    let Some(AssetSourceType::Csv { path }) = &node.asset_source else {
                        continue;
                    };
                    if workspace.asset_ohlc_history.contains_key(&node.id) {
                        loaded += 1;
                    } else {
                        failed_paths.push(path.clone());
                    }
                }
                for path in &failed_paths {
                    workspace.push_status_log(format!("CSV preload failed for `{path}`"));
                }

                if loaded > 0 {
                    let failed_count = failed_paths.len();
                    workspace.sync_historical_bar_count();
                    workspace.request_graph_engine_sweep(cx);
                    workspace.push_status_log(format!(
                        "CSV preload — {loaded} asset source(s) loaded{}",
                        if failed_count == 0 {
                            String::new()
                        } else {
                            format!(", {failed_count} failed")
                        }
                    ));
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn sync_workspace_ledger(&mut self, cx: &mut Context<Self>) {
        let stage = pulsar_marketlab::stage_bridge::UsdStageBridge::borrow(
            self.workspace_context.read(cx).usd_stage(),
        );
        let entries: Vec<pulsar_marketlab_ui::workspace::StageLedgerEntry> = stage
            .stage_ledger_entries()
            .unwrap_or_default()
            .into_iter()
            .map(|row| pulsar_marketlab_ui::workspace::StageLedgerEntry {
                prim_path: row.prim_path,
                property: row.property,
                depth: row.depth,
                active: row.active,
                override_layer: row.override_layer,
                deviates_from_schema: row.deviates_from_schema,
                value_label: row.value_label,
                lineage: row.lineage,
            })
            .collect();
        self.workspace_context.update(cx, |context, cx| {
            context.replace_ledger_entries(entries);
            cx.notify();
        });
    }

    pub(crate) fn register_asset_prim_in_usd_stage(
        &mut self,
        prim_path: &str,
        cx: &mut Context<Self>,
    ) {
        self.workspace_context.update(cx, |context, cx| {
            context.usd_stage().set_prim_active(prim_path, true);
            cx.notify();
        });
    }

    pub(crate) fn stage_prim_path_for_node(node: &VisualNode) -> Option<String> {
        crate::canvas_compose::stage_prim_path_for_node(node)
    }

    pub(crate) fn stage_prim_path_for_node_in_graph(
        &self,
        node: &VisualNode,
    ) -> Option<String> {
        crate::canvas_compose::stage_prim_path_for_node_resolved(
            node,
            &self.nodes,
            &self.connections,
        )
    }

    pub(crate) fn timeline_bar_labels(&self) -> Option<Vec<String>> {
        let timeline_len = self.historical_bar_count.max(
            self.asset_ohlc_history
                .values()
                .map(|bars| bars.len())
                .max()
                .unwrap_or(0),
        );
        if timeline_len == 0 {
            return None;
        }
        self.asset_ohlc_history.values().find_map(|bars| {
            if bars.is_empty() {
                return None;
            }
            Some(
                bars.iter()
                    .take(timeline_len)
                    .map(|bar| bar.date.clone())
                    .collect(),
            )
        })
    }

    pub(crate) fn refresh_portfolio_wealth_chart_cache(
        &mut self,
        result: &pulsar_marketlab_core::TimelineExecutionResult,
    ) {
        use crate::portfolio_integrator_ledger::build_integrator_ledger;
        use crate::portfolio_wealth_chart::{
            build_allocation_chart_from_integration, build_allocation_chart_from_token_streams,
            build_portfolio_wealth_chart_from_streams, build_portfolio_wealth_chart_series,
        };

        let bar_labels = self.timeline_bar_labels();
        self.portfolio_timeline_cache.clear();
        self.portfolio_allocation_cache.clear();
        self.portfolio_ledger_cache.clear();
        self.graph_engine_portfolio_results = result.portfolio_results.clone();
        self.graph_engine_streams = result.streams.clone();
        self.graph_engine_token_streams = result.token_streams.clone();
        self.refresh_portfolio_diagnostics_cache();

        for (prim_path, integration) in &result.portfolio_results {
            self.portfolio_timeline_cache.insert(
                prim_path.clone(),
                build_portfolio_wealth_chart_series(integration, bar_labels.clone()),
            );
            self.portfolio_allocation_cache.insert(
                prim_path.clone(),
                build_allocation_chart_from_integration(integration, bar_labels.clone()),
            );
            self.portfolio_ledger_cache.insert(
                prim_path.clone(),
                Arc::new(build_integrator_ledger(integration, bar_labels.clone())),
            );
        }

        if self.portfolio_timeline_cache.is_empty() {
            for node in &self.nodes {
                if !node.node_type.is_portfolio() {
                    continue;
                }
                let Some(prim_path) = self.stage_prim_path_for_node_in_graph(node) else {
                    continue;
                };
                if let Some(series) = build_portfolio_wealth_chart_from_streams(
                    &result.streams,
                    &prim_path,
                    bar_labels.clone(),
                ) {
                    self.portfolio_timeline_cache.insert(prim_path.clone(), series);
                }
                if let Some(allocation) = build_allocation_chart_from_token_streams(
                    &result.token_streams,
                    &prim_path,
                    bar_labels.clone(),
                    |path| self.prim_display_label_for_path(path),
                ) {
                    self.portfolio_allocation_cache.insert(prim_path.clone(), allocation);
                }
                if !self.portfolio_ledger_cache.contains_key(&prim_path) {
                    self.portfolio_ledger_cache.insert(
                        prim_path,
                        Arc::new(build_integrator_ledger(
                            &pulsar_marketlab_core::PortfolioIntegrationResult {
                                wealth_series: Vec::new(),
                                tracking_matrix: Vec::new(),
                            },
                            bar_labels.clone(),
                        )),
                    );
                }
            }
        }
    }

    pub(crate) fn graph_engine_analytics_active(&self) -> bool {
        self.ui_read_snapshot()
            .map(|snapshot| !snapshot.graph_engine_portfolio_results.is_empty())
            .unwrap_or(false)
    }

    pub(crate) fn graph_engine_vectorized_active(&self) -> bool {
        self.ui_read_snapshot()
            .map(|snapshot| !snapshot.graph_engine_streams.is_empty())
            .unwrap_or(false)
    }

    pub(crate) fn refresh_portfolio_diagnostics_cache(&mut self) {
        use crate::portfolio_analytics::build_portfolio_diagnostics_from_integration;

        self.portfolio_diagnostics_cache.clear();
        let bar_index = self.terminal_bar_index();
        let tick_label = self.terminal_tick_label();
        let benchmark = self.primary_asset_benchmark_prices();

        for (prim_path, integration) in &self.graph_engine_portfolio_results {
            let snapshot = build_portfolio_diagnostics_from_integration(
                integration,
                bar_index,
                SIM_INITIAL_CASH,
                self.portfolio_metrics_epoch,
                Some(tick_label.clone()),
                benchmark.as_deref(),
            );
            self.portfolio_diagnostics_cache
                .insert(prim_path.clone(), snapshot);
        }
        self.metrics_telemetry_dirty = true;
    }

    pub(crate) fn publish_metrics_telemetry_bridge(&mut self, cx: &mut Context<Self>) {
        use crate::ui::telemetry_bridge::{publish_metrics_telemetry, MetricsTelemetryBridge};

        if !self.graph_engine_analytics_active() {
            MetricsTelemetryBridge::update_global(cx, |bridge, _| bridge.reset());
            self.metrics_telemetry_dirty = false;
            return;
        }

        let bar_index = self.terminal_bar_index();
        let nodes = self.nodes.clone();
        let Some(snapshot) = self.graph_ui_snapshot.clone() else {
            self.metrics_telemetry_dirty = false;
            return;
        };
        let portfolio_results = &snapshot.graph_engine_portfolio_results;
        let diagnostics = &snapshot.portfolio_diagnostics_cache;
        let selected_node_id = self.selected_node_id;
        publish_metrics_telemetry(
            cx,
            &nodes,
            portfolio_results,
            diagnostics,
            bar_index,
            |node| self.stage_prim_path_for_node_in_graph(node),
            selected_node_id,
        );
        self.metrics_telemetry_dirty = false;
    }

    pub(crate) fn flush_metrics_telemetry_if_dirty(&mut self, cx: &mut Context<Self>) {
        if self.metrics_telemetry_dirty {
            self.publish_metrics_telemetry_bridge(cx);
        }
    }

    fn primary_asset_benchmark_prices(&self) -> Option<Vec<f64>> {
        let primary = self
            .nodes
            .iter()
            .find(|node| node.node_type.is_asset_adaptor())?;
        let bars = self.asset_ohlc_history.get(&primary.id)?;
        Some(bars.iter().map(|bar| bar.close).collect())
    }

    pub(crate) fn portfolio_diagnostics_for_node(
        &self,
        node_id: usize,
    ) -> Option<&PortfolioDiagnosticsSnapshot> {
        let node = self.nodes.iter().find(|node| node.id == node_id)?;
        if !node.node_type.is_portfolio() {
            return None;
        }
        let prim_path = self.stage_prim_path_for_node_in_graph(node)?;
        self.ui_read_snapshot()?
            .portfolio_diagnostics_cache
            .get(&prim_path)
    }

    pub(crate) fn portfolio_diagnostics_for_selection(&self) -> Option<&PortfolioDiagnosticsSnapshot> {
        let node = self.selected_portfolio_node()?;
        let prim_path = self.stage_prim_path_for_node_in_graph(node)?;
        self.ui_read_snapshot()?
            .portfolio_diagnostics_cache
            .get(&prim_path)
    }

    pub(crate) fn portfolio_wealth_chart_for_selection(
        &self,
    ) -> Option<&crate::portfolio_wealth_chart::PortfolioWealthChartSeries> {
        let selected_id = self.selected_node_id?;
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == selected_id && node.node_type.is_portfolio())?;
        let prim_path = self.stage_prim_path_for_node_in_graph(node)?;
        self.ui_read_snapshot()?
            .portfolio_timeline_cache
            .get(&prim_path)
    }

    pub(crate) fn portfolio_allocation_chart_for_selection(
        &self,
    ) -> Option<&crate::portfolio_wealth_chart::PortfolioAllocationChartSeries> {
        let node = self.selected_portfolio_node()?;
        let prim_path = self.stage_prim_path_for_node_in_graph(node)?;
        self.ui_read_snapshot()?
            .portfolio_allocation_cache
            .get(&prim_path)
    }

    pub(crate) fn portfolio_weights_encoded_at_terminal_bar(&self, prim_path: &str) -> Option<String> {
        let bar = self.terminal_bar_index() as f64;
        let snapshot = self.ui_read_snapshot()?;
        let stream = snapshot.graph_engine_token_streams.iter().find(|stream| {
            stream.prim_path == prim_path && stream.attribute == "outputs:weights"
        })?;
        stream
            .samples
            .iter()
            .find(|(sample_bar, _)| (*sample_bar - bar).abs() < f64::EPSILON)
            .or_else(|| stream.samples.last())
            .map(|(_, encoded)| encoded.clone())
    }

    pub(crate) fn prim_display_label_for_path(&self, prim_path: &str) -> String {
        if let Some(node) = self.nodes.iter().find(|node| {
            self.stage_prim_path_for_node_in_graph(node)
                .is_some_and(|path| path == prim_path)
        }) {
            return node.name.clone();
        }
        let leaf = prim_path.rsplit('/').next().unwrap_or(prim_path);
        pulsar_marketlab_core::prim_display_label(leaf, None)
    }

    pub(crate) fn portfolio_integrator_ledger_for_selection(
        &self,
    ) -> Option<Arc<crate::portfolio_integrator_ledger::PortfolioIntegratorLedger>> {
        let selected_id = self.selected_node_id?;
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == selected_id && node.node_type.is_portfolio())?;
        let prim_path = self.stage_prim_path_for_node_in_graph(node)?;
        self.ui_read_snapshot()?
            .portfolio_ledger_cache
            .get(&prim_path)
            .cloned()
    }

    pub(crate) fn node_id_for_stage_path(&self, prim_path: &str) -> Option<usize> {
        let resolved = crate::canvas_compose::resolve_node_stage_paths(
            &self.nodes,
            &self.connections,
        );
        self.nodes.iter().find_map(|node| {
            resolved
                .get(&node.id)
                .filter(|path| path.as_str() == prim_path)
                .map(|_| node.id)
        })
    }

    pub(crate) fn resolved_stage_path_for_node(&self, node: &VisualNode) -> Option<String> {
        self.stage_prim_path_for_node_in_graph(node)
    }

    /// Unified selection entry point: tree-table and node canvas both route here.
    pub(crate) fn select_stage_path(
        &mut self,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.selected_node_id = path
            .as_deref()
            .and_then(|prim_path| self.node_id_for_stage_path(prim_path));
        self.sync_inspector_from_selection(cx);
        cx.notify();

        let workspace_context = self.workspace_context.clone();
        cx.defer(move |cx| {
            workspace_context.update(cx, |ws, cx| {
                ws.set_selected_path(path, cx);
            });
        });
    }

    pub(crate) fn sync_canvas_selection_from_context(
        &mut self,
        context: &pulsar_marketlab_ui::workspace::WorkspaceContext,
    ) {
        let selected_path = context.selected_path().map(str::to_string);
        self.selected_node_id = selected_path
            .as_deref()
            .and_then(|prim_path| self.node_id_for_stage_path(prim_path));
    }

    pub(crate) fn canvas_interaction_active(&self) -> bool {
        self.active_drag_node_id.is_some() || self.is_panning
    }

    /// True while continuous pointer interaction should block OTL compile and USD writes.
    pub(crate) fn pipeline_interaction_active(&self) -> bool {
        self.canvas_interaction_active() || self.ta_lookback_scrubbing
    }

    /// Flush debounced pipeline work after drag, pan, or slider scrub completes.
    pub(crate) fn on_pipeline_interaction_ended(&mut self, cx: &mut Context<Self>) {
        if self.pipeline_sync_deferred {
            self.pipeline_sync_deferred = false;
            self.schedule_canvas_stage_sync(cx);
        }
        if let Some(result) = self.pending_timeline_result.take() {
            let ui_snapshot = self.pending_ui_snapshot.take();
            self.apply_timeline_result_now(&result, ui_snapshot, cx);
        }
        let entity = cx.entity();
        let needs_sweep = self.graph_engine_recompile_pending
            || self
                .workspace_context
                .read(cx)
                .is_engine_cache_dirty(self.graph_engine_last_compiled_generation);
        if needs_sweep {
            self.graph_engine_recompile_pending = false;
            pulsar_marketlab_ui::workspace::begin_graph_engine_timeline_sweep(self, entity, cx);
        }
        cx.notify();
    }

    pub(crate) fn apply_timeline_result_now(
        &mut self,
        result: &TimelineExecutionResult,
        prebuilt: Option<std::sync::Arc<crate::graph_ui_snapshot::GraphUiSnapshot>>,
        cx: &mut Context<Self>,
    ) {
        self.graph_engine_compile_error = None;
        if let Some(ui) = prebuilt {
            self.apply_graph_ui_snapshot(ui);
            self.schedule_deferred_view_window_sync(cx);
            return;
        }
        let build_input = self.graph_ui_snapshot_build_input();
        let result = result.clone();
        cx.spawn(async move |this, cx| {
            let ui_snapshot =
                crate::graph_ui_snapshot::build_graph_ui_snapshot(&result, &build_input);
            let _ = this.update(cx, |workspace, cx| {
                workspace.apply_graph_ui_snapshot(std::sync::Arc::new(ui_snapshot));
                workspace.schedule_deferred_view_window_sync(cx);
            });
        })
        .detach();
    }

    /// Defer inspector/metrics sync to the next frame so sweep completion stays off the hot path.
    pub(crate) fn schedule_deferred_view_window_sync(&mut self, cx: &mut Context<Self>) {
        if self.view_window_sync_pending {
            return;
        }
        self.view_window_sync_pending = true;
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let Some(entity) = view.upgrade() else {
                return;
            };
            entity.update(cx, |workspace, cx| {
                workspace.view_window_sync_pending = false;
                workspace.sync_view_window(cx);
            });
        });
    }

    pub(crate) fn cached_stage_graph_snapshot(
        &self,
    ) -> std::sync::Arc<pulsar_marketlab_core::StageGraphSnapshot> {
        let revision = self.pipeline_graph.revision();
        if let Ok(guard) = self.stage_graph_snapshot_cache.lock() {
            if let Some((cached_rev, snapshot)) = guard.as_ref() {
                if *cached_rev == revision {
                    return std::sync::Arc::clone(snapshot);
                }
            }
        }
        let snapshot = std::sync::Arc::new(
            crate::canvas_graph_snapshot::build_stage_graph_snapshot_from_graph(
                &self.nodes,
                &self.connections,
            ),
        );
        if let Ok(mut guard) = self.stage_graph_snapshot_cache.lock() {
            *guard = Some((revision, std::sync::Arc::clone(&snapshot)));
        }
        snapshot
    }

    pub(crate) fn store_stage_graph_snapshot_cache(
        &self,
        revision: u64,
        snapshot: std::sync::Arc<pulsar_marketlab_core::StageGraphSnapshot>,
    ) {
        if let Ok(mut guard) = self.stage_graph_snapshot_cache.lock() {
            *guard = Some((revision, snapshot));
        }
    }

    pub(crate) fn pipeline_graph_snapshot_cached(
        &mut self,
    ) -> &crate::graph_compiler::PipelineGraphSnapshot {
        let revision = self.pipeline_graph.revision();
        if self
            .cached_pipeline_snapshot
            .as_ref()
            .map(|(rev, _)| *rev)
            != Some(revision)
        {
            self.cached_pipeline_snapshot =
                Some((revision, self.pipeline_graph.snapshot()));
        }
        &self.cached_pipeline_snapshot.as_ref().unwrap().1
    }

    pub(crate) fn ui_read_snapshot(&self) -> Option<&crate::graph_ui_snapshot::GraphUiSnapshot> {
        self.graph_ui_snapshot.as_deref()
    }

    pub(crate) fn graph_ui_snapshot_build_input(&self) -> crate::graph_ui_snapshot::GraphUiSnapshotBuildInput {
        use crate::graph_ui_snapshot::GraphUiSnapshotBuildInput;

        let mut node_prim_paths = Vec::new();
        let mut prim_labels = std::collections::HashMap::new();
        for node in &self.nodes {
            let Some(prim_path) = self.stage_prim_path_for_node_in_graph(node) else {
                continue;
            };
            prim_labels.insert(prim_path.clone(), node.name.clone());
            node_prim_paths.push((node.id, prim_path, node.node_type.is_portfolio()));
        }

        GraphUiSnapshotBuildInput {
            bar_labels: self.timeline_bar_labels(),
            terminal_bar_index: self.terminal_bar_index(),
            terminal_tick_label: self.terminal_tick_label(),
            portfolio_metrics_epoch: self.portfolio_metrics_epoch,
            benchmark_prices: self.primary_asset_benchmark_prices(),
            node_prim_paths,
            prim_labels,
        }
    }

    pub(crate) fn apply_graph_ui_snapshot(
        &mut self,
        ui: std::sync::Arc<crate::graph_ui_snapshot::GraphUiSnapshot>,
    ) {
        self.graph_ui_snapshot = Some(ui);
        self.metrics_telemetry_dirty = true;
    }

    /// Batch node-canvas pan/drag updates into a single UI frame.
    pub(crate) fn schedule_canvas_interaction_repaint(&mut self, cx: &mut Context<Self>) {
        if self.canvas_interaction_repaint_pending {
            return;
        }
        self.canvas_interaction_repaint_pending = true;
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let Some(entity) = view.upgrade() else {
                return;
            };
            entity.update(cx, |workspace, cx| {
                workspace.canvas_interaction_repaint_pending = false;
                cx.notify();
            });
        });
    }

    /// Batch FIX/CSV ingestion updates into a single UI frame.
    pub(crate) fn schedule_pipeline_ingestion_repaint(&mut self, cx: &mut Context<Self>) {
        if self.pipeline_ingestion_repaint_pending {
            return;
        }
        self.pipeline_ingestion_repaint_pending = true;
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let Some(entity) = view.upgrade() else {
                return;
            };
            entity.update(cx, |workspace, cx| {
                workspace.pipeline_ingestion_repaint_pending = false;
                workspace.synchronize_inspector_view();
                cx.notify();
            });
        });
    }

    pub(crate) fn mark_otl_shader_inputs_stale(&mut self) {
        self.otl_shader_inputs_stale = true;
    }

    pub(crate) fn toggle_node_collapsed(&mut self, node_id: usize, cx: &mut Context<Self>) {
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
            node.collapsed = !node.collapsed;
            cx.notify();
        }
    }

    pub(crate) fn ensure_node_lookback_inputs(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.node_lookback_inputs_ready {
            return;
        }
        self.node_lookback_inputs_ready = true;

        for node in &self.nodes {
            if !node.node_type.is_ta_uber_signal() {
                continue;
            }
            if self.node_lookback_inputs.contains_key(&node.id) {
                continue;
            }
            let _node_id = node.id;
            let _period = node.overlay_period();
            // Uber-signal hyperparameters are edited in the sidebar inspector only.
        }
    }

    /// Bind an upstream output prim onto a downstream input slot after a wire drop.
    pub(crate) fn connect_primitives(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
        cx: &mut Context<Self>,
    ) {
        use pulsar_marketlab_ui::workspace::{
            compile_relationship_directive, execution_slot_for_target_prim,
            execution_slot_for_target_type,
        };
        let slot = self
            .workspace_context
            .read(cx)
            .usd_stage()
            .prim_type_name(target_prim_path)
            .as_deref()
            .and_then(execution_slot_for_target_type)
            .or_else(|| execution_slot_for_target_prim(target_prim_path));
        let Some(slot) = slot else {
            self.push_status_log(format!(
                "connect_primitives: unknown target prim `{target_prim_path}`"
            ));
            return;
        };
        let directive = compile_relationship_directive(target_prim_path, source_prim_path, slot);
        if let Err(err) = self.market_stage.set_relationship(
            &directive.target_prim_path,
            &directive.relationship,
            &directive.source_prim_path,
        ) {
            self.push_status_log(format!("connect_primitives failed: {err}"));
            return;
        }
        self.push_status_log(format!(
            "connect_primitives(\"{source_prim_path}\", \"{target_prim_path}\") → {}",
            directive.as_stage_instruction()
        ));
        self.workspace_context.update(cx, |context, cx| {
            context.connect_primitives(source_prim_path, target_prim_path, cx);
        });
        cx.notify();
    }

    /// Remove a USD relationship edge after a canvas wire disconnect.
    pub(crate) fn disconnect_primitives(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
        cx: &mut Context<Self>,
    ) {
        use pulsar_marketlab_ui::workspace::{
            compile_relationship_directive, execution_slot_for_target_prim,
            execution_slot_for_target_type,
        };
        let slot = self
            .workspace_context
            .read(cx)
            .usd_stage()
            .prim_type_name(target_prim_path)
            .as_deref()
            .and_then(execution_slot_for_target_type)
            .or_else(|| execution_slot_for_target_prim(target_prim_path));
        let Some(slot) = slot else {
            return;
        };
        let directive = compile_relationship_directive(target_prim_path, source_prim_path, slot);
        let _ = self.market_stage.remove_relationship(
            &directive.target_prim_path,
            &directive.relationship,
            &directive.source_prim_path,
        );
        self.push_status_log(format!(
            "disconnect_primitives(\"{source_prim_path}\", \"{target_prim_path}\")"
        ));
        cx.notify();
    }

    /// Reset canvas, pipeline, and USD stage to a fresh blank document.
    pub(crate) fn reset_to_new_document(&mut self, cx: &mut Context<Self>) {
        self.nodes.clear();
        self.connections.clear();
        self.selected_node_id = None;
        self.active_drag_node_id = None;
        self.active_wire_source = None;
        self.context_menu_pos = None;
        self.ta_inspector_category = None;
        self.pan_offset = point(px(0.0), px(0.0));
        self.zoom_scale = 1.0;
        self.usd_document_path = None;
        self.canvas_tabs = vec![pulsar_marketlab_ui::workspace::CanvasEnvironmentTab::root()];
        self.active_canvas_tab = 0;
        self.last_node_header_click = None;
        self.inspector_data.clear();
        self.portfolio_diagnostics = None;
        self.portfolio_timeline_cache.clear();
        self.portfolio_ledger_cache.clear();
        self.portfolio_allocation_cache.clear();
        self.graph_engine_portfolio_results.clear();
        self.graph_engine_streams.clear();
        self.graph_engine_token_streams.clear();
        self.cached_timeline_map_key = None;
        self.portfolio_diagnostics_cache.clear();
        self.historical_bar_count = 0;
        self.pipeline_status_log =
            vec!["New document — empty canvas and blank USD stage.".to_string()];
        self.node_lookback_inputs.clear();
        self.node_lookback_inputs_ready = false;
        self.otl_shader_param_inputs.clear();
        self.last_ui_selection_generation = 0;
        self.last_published_node_paths.clear();
        self.canvas_stage_sync_revision = 0;
        self.canvas_stage_sync_debounce_scheduled = false;
        self.canvas_stage_full_recompose_inflight = false;

        self.csv_path_registry.replace_from_nodes(&[]);
        self.asset_ohlc_history.clear();
        self.asset_close_series.clear();
        self.asset_chart_history.clear();
        self.asset_chart_bitmaps.clear();
        self.market_stage = MarketStage::new();

        self.asset_path_input.update(cx, |input, cx| {
            input.set_content(String::new(), cx);
        });

        self.workspace_context.update(cx, |context, cx| {
            *context = blank_workspace_context();
            cx.notify();
        });

        self.otl_script_input = None;
        self.otl_script_node_id = None;
        self.reset_otl_editor_input();
        self.otl_compile_status.clear();
        self.otl_compile_inflight = false;
        self.sync_historical_bar_count();
        self.sync_pipeline_graph(cx);
        self.sync_view_window(cx);
        cx.notify();
        self.schedule_session_autosave();
    }

    /// Copy a portfolio `.usda` into the active project stack and hydrate the node canvas.
    pub(crate) fn import_portfolio_layer_from_disk(
        &mut self,
        source: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let filename = match self.workspace_context.update(cx, |ctx, cx| {
            ctx.import_portfolio_layer_into_workspace(&source, cx)
        }) {
            Ok(name) => name,
            Err(err) => {
                self.push_status_log(format!("Portfolio import failed: {err}"));
                return;
            }
        };

        let stage_path = PathBuf::from(
            self.workspace_context
                .read(cx)
                .usd_stage()
                .root_layer_path(),
        );
        let hydrated = match pulsar_marketlab::stage_bridge::UsdStageBridge::open(&stage_path) {
            Ok(stage) => hydrate_canvas_from_stage(&stage),
            Err(error) => {
                self.push_status_log(format!(
                    "Portfolio `{filename}` imported but canvas hydration failed: {error}"
                ));
                self.topology_tree_cache_stage_generation = u64::MAX;
                self.request_graph_engine_sweep(cx);
                cx.notify();
                return;
            }
        };

        self.nodes = hydrated.nodes;
        self.connections = hydrated.connections;
        self.selected_node_id = None;
        self.csv_path_registry.replace_from_nodes(&self.nodes);
        self.pipeline_graph
            .replace(self.nodes.clone(), self.connections.clone());
        self.topology_tree_cache_stage_generation = u64::MAX;
        self.sync_historical_bar_count();
        self.preload_bound_csv_assets(cx);
        self.sync_view_window(cx);
        self.request_graph_engine_sweep(cx);
        self.schedule_canvas_stage_sync(cx);
        self.push_status_log(format!(
            "Imported portfolio layer `{filename}` from `{}` ({} nodes hydrated)",
            source.display(),
            self.nodes.len()
        ));
        cx.notify();
        self.schedule_session_autosave();
    }

    pub(crate) fn sync_pipeline_graph(&mut self, cx: &mut Context<Self>) {
        use crate::graph_compiler::{
            portfolio_ensure_spare_input_port, sync_portfolio_input_ports_from_connections,
        };
        sync_portfolio_input_ports_from_connections(&mut self.nodes, &self.connections);
        let portfolio_ids: Vec<usize> = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_portfolio())
            .map(|node| node.id)
            .collect();
        for portfolio_id in portfolio_ids {
            portfolio_ensure_spare_input_port(&mut self.nodes, &self.connections, portfolio_id);
        }
        self.pipeline_graph
            .replace(self.nodes.clone(), self.connections.clone());
        self.mark_otl_shader_inputs_stale();
        self.schedule_canvas_stage_sync(cx);
    }

    /// Update graph caches immediately; re-sweep without OpenUSD commit until save/topology idle.
    pub(crate) fn commit_ta_uber_parameter_change(&mut self, cx: &mut Context<Self>) {
        self.pipeline_graph
            .replace(self.nodes.clone(), self.connections.clone());
        self.ta_hyperparam_revision = self.ta_hyperparam_revision.wrapping_add(1);
        if self.bootstrapping {
            self.schedule_canvas_stage_sync(cx);
        } else {
            self.schedule_ta_param_resweep(cx);
        }
        cx.notify();
    }

    /// Coalesce rapid TA hyperparameter edits into one engine re-sweep (no USD write).
    pub(crate) fn schedule_ta_param_resweep(&mut self, cx: &mut Context<Self>) {
        if self.ta_hyperparam_debounce_scheduled {
            return;
        }
        self.ta_hyperparam_debounce_scheduled = true;
        let target_revision = self.ta_hyperparam_revision;
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(PIPELINE_DEBOUNCE_MS))
                .await;
            let _ = cx.update(|cx| {
                let Some(entity) = view.upgrade() else {
                    return;
                };
                entity.update(cx, |workspace, cx| {
                    workspace.ta_hyperparam_debounce_scheduled = false;
                    if workspace.ta_hyperparam_revision != target_revision {
                        workspace.schedule_ta_param_resweep(cx);
                        return;
                    }
                    if workspace.pipeline_interaction_active() {
                        workspace.pipeline_sync_deferred = true;
                        workspace.graph_engine_recompile_pending = true;
                        return;
                    }
                    workspace.graph_engine_cached = None;
                    workspace.request_graph_engine_sweep(cx);
                });
            });
        })
        .detach();
    }

    /// Coalesce rapid TA hyperparameter edits into one stage sync + engine sweep.
    #[allow(dead_code)]
    pub(crate) fn schedule_ta_hyperparam_sync(&mut self, cx: &mut Context<Self>) {
        if self.bootstrapping {
            self.schedule_canvas_stage_sync(cx);
            return;
        }
        if self.ta_hyperparam_debounce_scheduled {
            return;
        }
        self.ta_hyperparam_debounce_scheduled = true;
        let target_revision = self.ta_hyperparam_revision;
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(PIPELINE_DEBOUNCE_MS))
                .await;
            let _ = cx.update(|cx| {
                let Some(entity) = view.upgrade() else {
                    return;
                };
                entity.update(cx, |workspace, cx| {
                    workspace.ta_hyperparam_debounce_scheduled = false;
                    if workspace.ta_hyperparam_revision != target_revision {
                        workspace.schedule_ta_hyperparam_sync(cx);
                        return;
                    }
                    if workspace.pipeline_interaction_active() {
                        workspace.pipeline_sync_deferred = true;
                        return;
                    }
                    workspace.schedule_canvas_stage_sync(cx);
                });
            });
        })
        .detach();
    }

    /// Coalesce rapid canvas edits into a single stage sync after a short debounce window.
    pub(crate) fn schedule_canvas_stage_sync(&mut self, cx: &mut Context<Self>) {
        self.canvas_stage_sync_revision = self.canvas_stage_sync_revision.wrapping_add(1);
        let target_revision = self.canvas_stage_sync_revision;
        if self.canvas_stage_sync_debounce_scheduled {
            return;
        }
        self.canvas_stage_sync_debounce_scheduled = true;
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(PIPELINE_DEBOUNCE_MS))
                .await;
            let _ = cx.update(|cx| {
                let Some(entity) = view.upgrade() else {
                    return;
                };
                entity.update(cx, |workspace, cx| {
                    workspace.canvas_stage_sync_debounce_scheduled = false;
                    if workspace.canvas_stage_sync_revision != target_revision {
                        workspace.schedule_canvas_stage_sync(cx);
                        return;
                    }
                    if workspace.pipeline_interaction_active() {
                        workspace.pipeline_sync_deferred = true;
                        return;
                    }
                    workspace.sync_graph_after_topology_edit(cx);
                });
            });
        })
        .detach();
    }

    /// Invalidate topology caches and re-run graph engine sweep without publishing to USD.
    pub(crate) fn sync_graph_after_topology_edit(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.pipeline_graph_snapshot_cached().clone();
        if !snapshot.nodes.is_empty() && (!snapshot.wiring_valid || !snapshot.dag_valid) {
            let mut reasons: Vec<String> = snapshot
                .wiring_errors
                .iter()
                .map(|error| error.message.clone())
                .collect();
            if !snapshot.dag_valid {
                reasons.push("dependency cycle detected in canvas graph".to_string());
            }
            self.push_status_log(format!(
                "Graph sync blocked — fix {} validation issue(s) before engine sweep.",
                reasons.len()
            ));
            for reason in reasons.iter().take(3) {
                self.push_status_log(format!("  • {reason}"));
            }
            cx.notify();
            return;
        }

        if let Ok(mut guard) = self.stage_graph_snapshot_cache.lock() {
            *guard = None;
        }
        self.graph_engine_last_compiled_generation = u64::MAX;
        self.workspace_context.update(cx, |ctx, cx| {
            ctx.invalidate_engine_topology_cache(cx);
        });

        if let Some(path) = self
            .workspace_context
            .read(cx)
            .selected_path()
            .map(str::to_string)
        {
            self.selected_node_id = self.node_id_for_stage_path(&path);
        }
        self.schedule_workspace_ledger_sync_deferred(cx);
        if self.pipeline_interaction_active() {
            self.pipeline_sync_deferred = true;
            self.graph_engine_recompile_pending = true;
            self.schedule_session_autosave();
            cx.notify();
            return;
        }
        let entity = cx.entity();
        pulsar_marketlab_ui::workspace::begin_graph_engine_timeline_sweep(self, entity, cx);
        self.schedule_session_autosave();
        cx.notify();
    }

    /// Sync canvas graph to the unified USD stage (incremental overlays or background full recompose).
    pub(crate) fn publish_canvas_to_usd_stage(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.pipeline_graph_snapshot_cached().clone();
        if !snapshot.nodes.is_empty() && (!snapshot.wiring_valid || !snapshot.dag_valid) {
            let mut reasons: Vec<String> = snapshot
                .wiring_errors
                .iter()
                .map(|error| error.message.clone())
                .collect();
            if !snapshot.dag_valid {
                reasons.push("dependency cycle detected in canvas graph".to_string());
            }
            self.push_status_log(format!(
                "USD compose blocked — fix {} validation issue(s) before stage sync.",
                reasons.len()
            ));
            for reason in reasons.iter().take(3) {
                self.push_status_log(format!("  • {reason}"));
            }
            cx.notify();
            return;
        }

        let stage = self.workspace_context.read(cx).usd_stage().clone();
        let nodes = self.nodes.clone();
        let connections = self.connections.clone();
        let last_paths = self.last_published_node_paths.clone();

        if !needs_full_stage_recompose(&stage, &nodes, &connections, &last_paths) {
            let report = apply_incremental_canvas_sync(&stage, &nodes, &connections);
            let paths = published_node_paths(&nodes, &connections);
            self.last_published_node_paths = paths.clone();
            let topology_changed = report.relationships_updated > 0;
            self.workspace_context.update(cx, |ctx, cx| {
                for prim_path in paths.values() {
                    ctx.execution_engine_mut().dirty_graph_node(prim_path);
                }
                if topology_changed {
                    ctx.invalidate_engine_topology_cache(cx);
                }
                cx.notify();
            });
            if !topology_changed {
                self.graph_engine_cached = None;
            }
            self.usd_commit_generation = self.usd_commit_generation.wrapping_add(1);
            self.push_status_log(format!(
                "USD incremental sync — {} prim(s), {} relationship(s), {} attribute(s)",
                report.prims_touched, report.relationships_updated, report.attributes_updated
            ));
            self.finish_canvas_stage_publish(cx, false, topology_changed);
            return;
        }

        if self.canvas_stage_full_recompose_inflight {
            self.canvas_stage_sync_revision = self.canvas_stage_sync_revision.wrapping_add(1);
            return;
        }

        let preserved_selection = self
            .workspace_context
            .read(cx)
            .selected_path()
            .map(str::to_string);
        let preserved_edit_target = self
            .workspace_context
            .read(cx)
            .edit_target_layer()
            .map(str::to_string);
        let preserved_overlays = self
            .workspace_context
            .read(cx)
            .usd_stage()
            .snapshot_runtime_overlays();

        self.canvas_stage_full_recompose_inflight = true;
        let context_handle = self.workspace_context.clone();
        let nodes_for_publish = nodes.clone();
        let connections_for_publish = connections.clone();
        cx.spawn(async move |this, cx| {
            let context = cx
                .background_executor()
                .spawn(async move {
                    let usda = if nodes.is_empty() {
                        blank_stage_usda()
                    } else {
                        compose_pipeline_usda(&nodes, &connections)
                    };
                    pulsar_marketlab_ui::workspace::WorkspaceContext::from_usda_text(&usda)
                        .unwrap_or_else(|_| blank_workspace_context())
                })
                .await;
            let _ = cx.update(|cx| {
                context_handle.update(cx, |ctx, cx| {
                    *ctx = context;
                    ctx.restore_runtime_overlays_from(preserved_overlays);
                    if let Some(path) = preserved_selection.as_deref() {
                        if ctx.usd_stage().prim_exists(path) {
                            ctx.set_selected_path(Some(path.to_string()), cx);
                        }
                    }
                    if let Some(layer) = preserved_edit_target {
                        if ctx.layer_identifiers().iter().any(|id| id == &layer) {
                            ctx.set_edit_target_layer(Some(layer), cx);
                        }
                    }
                    ctx.invalidate_engine_topology_cache(cx);
                    cx.notify();
                });

                if let Some(workspace) = this.upgrade() {
                    workspace.update(cx, |ws, cx| {
                        ws.canvas_stage_full_recompose_inflight = false;
                        ws.last_published_node_paths =
                            published_node_paths(&nodes_for_publish, &connections_for_publish);
                        ws.usd_commit_generation = ws.usd_commit_generation.wrapping_add(1);
                        ws.push_status_log("USD full stage recompose complete".to_string());
                        ws.finish_canvas_stage_publish(cx, true, true);
                    });
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_canvas_stage_publish(
        &mut self,
        cx: &mut Context<Self>,
        full_recompose: bool,
        topology_invalidated: bool,
    ) {
        let _ = full_recompose;
        if topology_invalidated {
            self.graph_engine_last_compiled_generation = u64::MAX;
        }
        if let Some(path) = self
            .workspace_context
            .read(cx)
            .selected_path()
            .map(str::to_string)
        {
            self.selected_node_id = self.node_id_for_stage_path(&path);
        }
        self.schedule_workspace_ledger_sync_deferred(cx);
        if self.pipeline_interaction_active() {
            self.pipeline_sync_deferred = true;
            self.graph_engine_recompile_pending = true;
            self.schedule_session_autosave();
            cx.notify();
            return;
        }
        let entity = cx.entity();
        if topology_invalidated || full_recompose {
            pulsar_marketlab_ui::workspace::begin_graph_engine_timeline_sweep(self, entity, cx);
        } else {
            self.graph_engine_cached = None;
            self.request_graph_engine_sweep(cx);
        }
        self.schedule_session_autosave();
        cx.notify();
    }

    /// Debounced off-thread ledger rebuild keyed on USD commit generation.
    pub(crate) fn schedule_workspace_ledger_sync_deferred(&mut self, cx: &mut Context<Self>) {
        if self.ledger_sync_debounce_scheduled {
            return;
        }
        self.ledger_sync_debounce_scheduled = true;
        let target_generation = self.usd_commit_generation;
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(PIPELINE_DEBOUNCE_MS))
                .await;
            let _ = cx.update(|cx| {
                let Some(entity) = view.upgrade() else {
                    return;
                };
                entity.update(cx, |workspace, cx| {
                    workspace.ledger_sync_debounce_scheduled = false;
                    if workspace.usd_commit_generation != target_generation {
                        workspace.schedule_workspace_ledger_sync_deferred(cx);
                        return;
                    }
                    workspace.sync_workspace_ledger(cx);
                });
            });
        })
        .detach();
    }

    /// Re-run the background timeline sweep after asset OHLC data changes without a full USD recompose.
    pub(crate) fn request_graph_engine_sweep(&mut self, cx: &mut Context<Self>) {
        self.graph_engine_asset_data_epoch = self.graph_engine_asset_data_epoch.wrapping_add(1);
        let entity = cx.entity();
        pulsar_marketlab_ui::workspace::begin_graph_engine_timeline_sweep(self, entity, cx);
    }

    pub(crate) fn bump_graph_engine_asset_data_epoch(&mut self) {
        self.graph_engine_asset_data_epoch = self.graph_engine_asset_data_epoch.wrapping_add(1);
    }

    pub(crate) fn asset_quote_symbol_for_node(&self, node: &VisualNode) -> String {
        if let Some(path) = self.stage_prim_path_for_node_in_graph(node) {
            if let Some(leaf) = path.rsplit('/').next().filter(|leaf| !leaf.is_empty()) {
                return leaf.trim_end_matches(".csv").to_string();
            }
        }
        if let NodeType::AssetAdaptor { prim_path } = &node.node_type {
            if let Some(leaf) = prim_path.rsplit('/').next().filter(|leaf| !leaf.is_empty()) {
                return leaf.to_string();
            }
        }
        node.name.trim_end_matches(".csv").to_string()
    }

    pub(crate) fn portfolio_graph_engine_status_label(&self, cx: &App) -> String {
        if self.graph_engine_recompile_inflight {
            return "Graph engine · compiling portfolio sweep…".to_string();
        }
        if let Some(error) = &self.graph_engine_compile_error {
            return format!("Graph engine · compile failed · {error}");
        }
        let workspace = self.workspace_context.read(cx);
        if workspace.is_engine_cache_dirty(self.graph_engine_last_compiled_generation) {
            return "Graph engine · pending recompile".to_string();
        }
        if self.graph_engine_timeline_len() == 0 {
            return "Graph engine · waiting for OHLC bars (bind CSV on asset node)".to_string();
        }
        if self.graph_engine_last_compile_ms > 0 {
            let portfolio_streams = workspace
                .computed_streams()
                .iter()
                .filter(|stream| stream.attribute == "outputs:portfolio_wealth")
                .count();
            return format!(
                "Graph engine · ready ({} ms) · {} streams · {} portfolio wealth",
                self.graph_engine_last_compile_ms,
                workspace.computed_streams().len(),
                portfolio_streams
            );
        }
        "Graph engine · idle".to_string()
    }

    fn graph_engine_timeline_len(&self) -> usize {
        self.historical_bar_count.max(
            self.asset_ohlc_history
                .values()
                .map(|bars| bars.len())
                .max()
                .unwrap_or(0),
        )
    }

    pub(crate) fn chart_bars_for_selection(&self) -> Vec<OhlcBar> {
        let Some(node_id) = self.selected_node_id else {
            return self
                .asset_ohlc_history
                .values()
                .next()
                .cloned()
                .unwrap_or_default();
        };
        if let Some(node) = self.nodes.iter().find(|node| node.id == node_id) {
            match &node.node_type {
                NodeType::AssetAdaptor { .. }
                    if matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) => {
                    return self
                        .asset_ohlc_history
                        .get(&node_id)
                        .cloned()
                        .unwrap_or_default();
                }
                NodeType::TaUberSignal { .. } | NodeType::OtlShader { .. } => {
                    if let Some(asset_id) = upstream_price_source_node_id_parts(
                        node_id,
                        0,
                        &self.nodes,
                        &self.connections,
                    ) {
                        return self
                            .asset_ohlc_history
                            .get(&asset_id)
                            .cloned()
                            .unwrap_or_default();
                    }
                }
                _ => {}
            }
        }
        self.asset_ohlc_history
            .values()
            .next()
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn sync_historical_bar_count(&mut self) {
        let bars = self.chart_bars_for_selection();
        self.historical_bar_count = bars.len();
    }

    pub(crate) fn terminal_bar_index(&self) -> usize {
        self.historical_bar_count.saturating_sub(1)
    }

    pub(crate) fn terminal_tick_label(&self) -> String {
        let bars = self.chart_bars_for_selection();
        if bars.is_empty() {
            return "—".to_string();
        }
        let index = self.terminal_bar_index().min(bars.len() - 1);
        format!(
            "{}/{} · {}",
            index + 1,
            bars.len(),
            bars[index].date
        )
    }

    pub(crate) fn take_cached_graph_engine(
        &mut self,
        generation: u64,
    ) -> Option<pulsar_marketlab_core::MarketLabGraphEngine> {
        if self.graph_engine_cached_generation != generation {
            self.graph_engine_cached = None;
            return None;
        }
        self.graph_engine_cached.take()
    }

    pub(crate) fn store_cached_graph_engine(
        &mut self,
        generation: u64,
        engine: pulsar_marketlab_core::MarketLabGraphEngine,
    ) {
        self.graph_engine_cached_generation = generation;
        self.graph_engine_cached = Some(engine);
    }

    /// Rebuild the global inspector register from pre-computed vectorized streams.
    pub(crate) fn synchronize_inspector_view(&mut self) {
        let Some(snapshot) = self.ui_read_snapshot() else {
            return;
        };
        if self.historical_bar_count == 0 || snapshot.graph_engine_streams.is_empty() {
            return;
        }
        let bar_index = self.terminal_bar_index();
        let tick = self.terminal_tick_label();
        let rows = build_inspector_rows_from_streams(
            &snapshot.graph_engine_streams,
            &self.nodes,
            bar_index,
            &tick,
            |node| self.stage_prim_path_for_node_in_graph(node),
        );
        if !rows.is_empty() {
            self.inspector_data = rows;
        }
    }

    /// Push terminal-bar portfolio weights onto USD overlay (debounced; never blocks sweeps).
    pub(crate) fn schedule_portfolio_weights_overlay_sync(&mut self, cx: &mut Context<Self>) {
        if self.portfolio_weights_overlay_pending {
            return;
        }
        self.portfolio_weights_overlay_pending = true;
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let Some(entity) = view.upgrade() else {
                return;
            };
            entity.update(cx, |workspace, cx| {
                workspace.portfolio_weights_overlay_pending = false;
                workspace.sync_portfolio_weights_stage_overlay(cx);
            });
        });
    }

    /// Push terminal-bar `outputs:weights` onto the session USD overlay for inspector/stage tree.
    pub(crate) fn sync_portfolio_weights_stage_overlay(&mut self, cx: &mut Context<Self>) {
        use openusd::sdf::Value;

        let Some(node) = self.selected_portfolio_node() else {
            return;
        };
        let Some(prim_path) = self.stage_prim_path_for_node_in_graph(node) else {
            return;
        };
        let Some(encoded) = self.portfolio_weights_encoded_at_terminal_bar(&prim_path) else {
            return;
        };
        let property_path = format!("{prim_path}.outputs:weights");
        self.workspace_context.update(cx, |workspace, cx| {
            let stage = workspace.usd_stage();
            if stage
                .field(&property_path, "default")
                .as_ref()
                .is_some_and(|value| matches!(value, Value::String(existing) if existing == &encoded))
            {
                return;
            }
            stage.set_field(&property_path, "default", Value::String(encoded));
            cx.notify();
        });
    }

    /// Passive view-window sync: slice pre-computed buffers at the current bar index.
    pub(crate) fn sync_historical_timeline_map(&mut self, cx: &mut Context<Self>) {
        let Some(labels) = self.timeline_bar_labels() else {
            return;
        };
        let len = labels.len();
        let first = labels.first().cloned().unwrap_or_default();
        let last = labels.last().cloned().unwrap_or_default();
        let key = (len, first, last);
        if self.cached_timeline_map_key.as_ref() == Some(&key) {
            return;
        }
        let map = pulsar_marketlab_core::HistoricalTimelineMap::from_dates(labels);
        self.workspace_context.update(cx, |ctx, _cx| {
            ctx.set_historical_timeline_map(map);
        });
        self.cached_timeline_map_key = Some(key);
    }

    pub(crate) fn sync_view_window(&mut self, cx: &mut Context<Self>) {
        if self.historical_bar_count == 0 {
            return;
        }
        self.sync_historical_timeline_map(cx);
        self.synchronize_inspector_view();

        if self.graph_engine_analytics_active() {
            self.schedule_portfolio_weights_overlay_sync(cx);
        }
        self.portfolio_diagnostics = self
            .portfolio_diagnostics_for_selection()
            .cloned()
            .or_else(|| {
                self.ui_read_snapshot()?
                    .portfolio_diagnostics_cache
                    .values()
                    .next()
                    .cloned()
            });

        if self.graph_engine_analytics_active() {
            self.publish_metrics_telemetry_bridge(cx);
        }
        cx.notify();
    }

    pub(crate) fn record_stage_sample_for_tick(
        &mut self,
        node_id: usize,
        source: &str,
        tick_index: usize,
        attribute: &str,
        value: f32,
    ) {
        let Some(bars) = self.asset_ohlc_history.get(&node_id) else {
            return;
        };
        let Some(time) = stage_time_for_bar_index(bars, tick_index) else {
            return;
        };
        if let Ok(prim) = asset_prim_path(source) {
            let _ = self
                .market_stage
                .set_sample(&prim, attribute, time, value);
        }
    }

    pub(crate) fn record_stage_analytics_sample(
        &mut self,
        node: &VisualNode,
        tick_index: usize,
        value: f32,
    ) {
        let bars = self
            .chart_bars_for_selection();
        let Some(time) = stage_time_for_bar_index(&bars, tick_index) else {
            return;
        };
        let indicator_id = analytics_indicator_id(node);
        if let Ok(prim) = analytics_prim_path(&indicator_id) {
            let _ = self
                .market_stage
                .set_sample(&prim, "value", time, value);
        }
    }

    pub(crate) fn push_status_log(&mut self, text: String) {
        self.pipeline_status_log.push(text);
        if self.pipeline_status_log.len() > STATUS_LOG_CAP {
            let overflow = self.pipeline_status_log.len() - STATUS_LOG_CAP;
            self.pipeline_status_log.drain(0..overflow);
        }
    }

    fn spawn_pipeline_ingestion_worker(
        &self,
        rx: Receiver<PipelineSystemMessage>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                let mut drained = false;
                while let Ok(message) = rx.try_recv() {
                    drained = true;
                    match message {
                        PipelineSystemMessage::TickUpdate {
                            tick_index,
                            tick_label,
                            node_id,
                            source,
                            value,
                        } => {
                            let csv_date = tick_label.clone();
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        let ta_node = workspace
                                            .nodes
                                            .iter()
                                            .find(|node| node.id == node_id)
                                            .filter(|node| node.node_type.is_ta_uber_signal())
                                            .cloned();
                                        if let Some(node) = ta_node {
                                            if let Some(value) =
                                                parse_chart_scalar_value(&value)
                                            {
                                                workspace.record_stage_analytics_sample(
                                                    &node,
                                                    tick_index,
                                                    value,
                                                );
                                            }
                                        }

                                        if let Some(close) = parse_chart_scalar_value(&value) {
                                            workspace.record_stage_sample_for_tick(
                                                node_id,
                                                &source,
                                                tick_index,
                                                "close",
                                                close,
                                            );
                                            if let Some(date) = csv_date.as_deref() {
                                                if let Some(time) = stage_time_from_bar_date(date)
                                                {
                                                    if let Ok(prim) = asset_prim_path(&source) {
                                                        let _ = workspace.market_stage.set_sample(
                                                            &prim,
                                                            "close",
                                                            time,
                                                            close,
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        if let (Some(date), Some(close)) =
                                            (csv_date, parse_chart_scalar_value(&value))
                                        {
                                            let is_chart_node = workspace.nodes.iter().any(|node| {
                                                node.id == node_id
                                                    && node.node_type.displays_price_chart()
                                            });
                                            if is_chart_node {
                                                let buffer = workspace
                                                    .asset_chart_history
                                                    .entry(node_id)
                                                    .or_default();
                                                let is_new_date = buffer
                                                    .timestamps
                                                    .last()
                                                    .map(|last| date.as_str() > last.as_str())
                                                    .unwrap_or(true);
                                                if is_new_date {
                                                    buffer.push_sample(date, close);
                                                }
                                            }
                                        }
                                        workspace.schedule_pipeline_ingestion_repaint(cx);
                                    });
                                }
                            });
                        }
                        PipelineSystemMessage::ChartSeriesPreload {
                            node_id,
                            timestamps,
                            values,
                            ohlc_bars,
                        } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        workspace
                                            .asset_chart_history
                                            .entry(node_id)
                                            .or_default()
                                            .replace_series(timestamps, values);
                                        workspace.rebuild_asset_chart_bitmaps_async(cx);
                                        if ohlc_bars.is_empty() {
                                            workspace.asset_ohlc_history.remove(&node_id);
                                            workspace.asset_close_series.remove(&node_id);
                                        } else {
                                            workspace
                                                .asset_ohlc_history
                                                .insert(node_id, ohlc_bars.clone());
                                            workspace.asset_close_series.insert(
                                                node_id,
                                                close_series_from_bars(&ohlc_bars),
                                            );
                                        }
                                        workspace.sync_historical_bar_count();
                                        workspace.request_graph_engine_sweep(cx);
                                        workspace.schedule_pipeline_ingestion_repaint(cx);
                                    });
                                }
                            });
                        }
                        PipelineSystemMessage::StatusAlert { text } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        workspace.push_status_log(text);
                                        cx.notify();
                                    });
                                }
                            });
                        }
                        PipelineSystemMessage::ChartBarCount { total_bars } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        workspace.historical_bar_count = total_bars;
                                        workspace.schedule_pipeline_ingestion_repaint(cx);
                                    });
                                }
                            });
                        }
                        PipelineSystemMessage::StageSample {
                            prim_path,
                            attribute,
                            time,
                            value,
                        } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        let _ = workspace
                                            .market_stage
                                            .set_sample(&prim_path, &attribute, time, value);
                                        workspace.schedule_pipeline_ingestion_repaint(cx);
                                    });
                                }
                            });
                        }
                    }
                }

                if !drained {
                    cx.background_executor()
                        .timer(INGESTION_POLL_INTERVAL)
                        .await;
                }
            }
        })
        .detach();
    }
}

#[derive(Debug)]
pub(crate) struct CsvAssetPlayback {
    pub(crate) node_id: usize,
    pub(crate) ticker: String,
    pub(crate) rows: Vec<YahooCsvRow>,
    pub(crate) cursor: usize,
    pub(crate) current_active_path: String,
    pub(crate) reader_paused: bool,
}

pub fn init_csv_playback_from_path(node_id: usize, path: &str) -> Result<CsvAssetPlayback, String> {
    let (ticker, rows) = load_yahoo_finance_csv(path)?;
    let mut playback = CsvAssetPlayback {
        node_id,
        ticker,
        rows,
        cursor: 0,
        current_active_path: path.to_string(),
        reader_paused: true,
    };
    csv_playback_park_at_last_bar(&mut playback);
    Ok(playback)
}
pub fn hot_swap_csv_playback(
    playback: &mut CsvAssetPlayback,
    new_path: &str,
    tx: &Sender<PipelineSystemMessage>,
) -> bool {
    if new_path == playback.current_active_path {
        return false;
    }

    let previous_path = playback.current_active_path.clone();
    let _ = tx.send(PipelineSystemMessage::StatusAlert {
        text: format!(
            "CSV hot-swap — rebinding node {} from `{previous_path}` → `{new_path}`",
            playback.node_id
        ),
    });

    playback.rows.clear();
    playback.cursor = 0;

    match load_yahoo_finance_csv(new_path) {
        Ok((ticker, rows)) => {
            let row_count = rows.len();
            playback.ticker = ticker;
            playback.rows = rows;
            playback.current_active_path = new_path.to_string();
            csv_playback_park_at_last_bar(playback);
            send_chart_series_preload(tx, playback.node_id, &playback.rows);
            send_playhead_set_to_last_bar(tx, row_count);
            let _ = tx.send(PipelineSystemMessage::StatusAlert {
                text: format!(
                    "CSV source bound — node {} streaming `{new_path}` ({row_count} rows)",
                    playback.node_id
                ),
            });
            true
        }
        Err(error) => {
            playback.reader_paused = true;
            playback.current_active_path = new_path.to_string();
            let _ = tx.send(PipelineSystemMessage::StatusAlert {
                text: format!(
                    "CSV file warning — node {} path `{new_path}`: {error}",
                    playback.node_id
                ),
            });
            false
        }
    }
}

pub fn resolve_csv_path(path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_file() {
        return candidate;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

pub fn ticker_from_csv_path(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("ASSET")
        .to_ascii_uppercase()
}

/// Node header label from a CSV path (e.g. `data/SPY.csv` → `SPY.csv`).
pub fn csv_node_label_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("asset.csv")
        .to_string()
}

fn csv_header_index(headers: &csv::StringRecord, candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|header| {
        let trimmed = header.trim();
        candidates
            .iter()
            .any(|candidate| trimmed.eq_ignore_ascii_case(candidate))
    })
}

fn parse_close_field(raw: &str) -> Result<f64, String> {
    raw.trim()
        .parse::<f64>()
        .map_err(|error| format!("invalid Close value `{raw}`: {error}"))
}

fn parse_optional_price_field(record: &csv::StringRecord, idx: usize) -> Option<f64> {
    record.get(idx).and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.parse().ok()
        }
    })
}

fn looks_like_iso_date(value: &str) -> bool {
    let value = value.trim();
    let Some((year, rest)) = value.split_once('-') else {
        return false;
    };
    let Some((month, day)) = rest.split_once('-') else {
        return false;
    };
    year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && year.chars().all(|ch| ch.is_ascii_digit())
        && month.chars().all(|ch| ch.is_ascii_digit())
        && day.chars().all(|ch| ch.is_ascii_digit())
}

fn parse_yahoo_csv_row(
    record: &csv::StringRecord,
    date_idx: usize,
    close_idx: usize,
    open_idx: Option<usize>,
    high_idx: Option<usize>,
    low_idx: Option<usize>,
    volume_idx: Option<usize>,
    line_no: usize,
    path: &Path,
) -> Result<Option<YahooCsvRow>, String> {
    let date = record
        .get(date_idx)
        .ok_or_else(|| format!("CSV row {} missing Date field", line_no))?
        .trim();
    if !looks_like_iso_date(date) {
        return Ok(None);
    }
    let close_raw = record
        .get(close_idx)
        .ok_or_else(|| format!("CSV row {} missing Close field", line_no))?;
    if close_raw.trim().is_empty() {
        return Ok(None);
    }
    let close = parse_close_field(close_raw).map_err(|error| {
        format!(
            "CSV row {} in `{}`: {error}",
            line_no,
            path.display()
        )
    })?;
    let open = open_idx.and_then(|idx| parse_optional_price_field(record, idx));
    let high = high_idx.and_then(|idx| parse_optional_price_field(record, idx));
    let low = low_idx.and_then(|idx| parse_optional_price_field(record, idx));
    let volume = volume_idx.and_then(|idx| parse_optional_price_field(record, idx));
    Ok(Some(YahooCsvRow {
        date: date.to_string(),
        open,
        high,
        low,
        close,
        volume,
    }))
}

/// Classic Yahoo export: `Date,Open,High,Low,Close,Adj Close,Volume`
pub fn load_yahoo_finance_csv_classic(
    resolved: PathBuf,
    fallback_ticker: String,
    headers: csv::StringRecord,
    mut reader: csv::Reader<std::fs::File>,
) -> Result<(String, Vec<YahooCsvRow>), String> {
    let date_idx = csv_header_index(&headers, &["Date"])
        .ok_or_else(|| format!("CSV missing Date column in `{}`", resolved.display()))?;
    let close_idx = csv_header_index(&headers, &["Adj Close", "Close"]).ok_or_else(|| {
        format!(
            "CSV missing Close / Adj Close column in `{}`",
            resolved.display()
        )
    })?;
    let open_idx = csv_header_index(&headers, &["Open"]);
    let high_idx = csv_header_index(&headers, &["High"]);
    let low_idx = csv_header_index(&headers, &["Low"]);
    let volume_idx = csv_header_index(&headers, &["Volume"]);

    let mut rows = Vec::new();
    for (offset, record) in reader.records().enumerate() {
        let record = record.map_err(|error| {
            format!(
                "CSV row {} parse failed in `{}`: {error}",
                offset + 2,
                resolved.display()
            )
        })?;
        if let Some(row) = parse_yahoo_csv_row(
            &record,
            date_idx,
            close_idx,
            open_idx,
            high_idx,
            low_idx,
            volume_idx,
            offset + 2,
            &resolved,
        )? {
            rows.push(row);
        }
    }

    if rows.is_empty() {
        return Err(format!("CSV `{}` contains no data rows", resolved.display()));
    }

    Ok((fallback_ticker, rows))
}

/// Modern Yahoo export:
/// ```text
/// Price,Close,High,Low,Open,Volume
/// Ticker,SPY,SPY,SPY,SPY,SPY
/// Date,,,,,
/// 2026-04-22,711.21,...
/// ```
pub fn load_yahoo_finance_csv_modern(
    resolved: PathBuf,
    fallback_ticker: String,
    headers: csv::StringRecord,
    mut reader: csv::Reader<std::fs::File>,
) -> Result<(String, Vec<YahooCsvRow>), String> {
    let close_idx = csv_header_index(&headers, &["Close", "Adj Close"]).ok_or_else(|| {
        format!(
            "CSV missing Close column in `{}`",
            resolved.display()
        )
    })?;
    let open_idx = csv_header_index(&headers, &["Open"]);
    let high_idx = csv_header_index(&headers, &["High"]);
    let low_idx = csv_header_index(&headers, &["Low"]);
    let volume_idx = csv_header_index(&headers, &["Volume"]);
    // Data rows place the trading date in column zero (header cell reads "Price").
    let date_idx = 0;

    let mut rows = Vec::new();
    let mut ticker = fallback_ticker;
    for (offset, record) in reader.records().enumerate() {
        let record = record.map_err(|error| {
            format!(
                "CSV row {} parse failed in `{}`: {error}",
                offset + 2,
                resolved.display()
            )
        })?;
        let first = record.get(0).unwrap_or("").trim();
        if first.eq_ignore_ascii_case("Ticker") {
            if let Some(symbol) = record.get(1).map(str::trim).filter(|s| !s.is_empty()) {
                ticker = symbol.to_ascii_uppercase();
            }
            continue;
        }
        if let Some(row) = parse_yahoo_csv_row(
            &record,
            date_idx,
            close_idx,
            open_idx,
            high_idx,
            low_idx,
            volume_idx,
            offset + 2,
            &resolved,
        )? {
            rows.push(row);
        }
    }

    if rows.is_empty() {
        return Err(format!("CSV `{}` contains no data rows", resolved.display()));
    }

    Ok((ticker, rows))
}

/// Load a Yahoo Finance CSV export (classic or modern layout).
pub fn load_yahoo_finance_csv(path: &str) -> Result<(String, Vec<YahooCsvRow>), String> {
    let resolved = resolve_csv_path(path);
    if !resolved.is_file() {
        return Err(format!(
            "Yahoo CSV asset not found at `{}` (resolved `{}`)",
            path,
            resolved.display()
        ));
    }

    let fallback_ticker = ticker_from_csv_path(&resolved);
    let mut reader = csv::Reader::from_path(&resolved)
        .map_err(|error| format!("CSV reader open failed for `{}`: {error}", resolved.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("CSV header read failed: {error}"))?
        .clone();

    if csv_header_index(&headers, &["Date"]).is_some() {
        load_yahoo_finance_csv_classic(resolved, fallback_ticker, headers, reader)
    } else if csv_header_index(&headers, &["Close"]).is_some()
        || csv_header_index(&headers, &["Price"]).is_some()
    {
        load_yahoo_finance_csv_modern(resolved, fallback_ticker, headers, reader)
    } else {
        Err(format!(
            "Unrecognized Yahoo CSV layout in `{}` — expected Date/Close or Price/Close headers",
            resolved.display()
        ))
    }
}
