//! Central workspace state, simulation bridge, and cross-thread messaging.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use gpui::*;

use crate::asset_path_input::{AssetPathInput, PathInputEvent};
use crate::graph_compiler::{
    csv_backed_asset_ids, portfolio_signal_port_label, portfolio_wired_ta_node_ids,
    ta_lookback_for_node, upstream_asset_for_ta_node, upstream_price_source_node_id_parts,
    ta_compute_for_node,
    AssetSourceType, NodeConnection, NodeGradeType, NodeType, PipelineGraphSnapshot, SharedCsvAssetPaths,
    SharedPipelineGraph, VisualNode,
};
use crate::ohlc_chart_pane::OhlcBar;
use pulsar_marketlab::execution_engine::{
    ExecutionEngine, SimulationTransaction, StageSimulationLedger, EXECUTION_CASH_ATTR,
    EXECUTION_CASH_PATH, position_prim_path,
};
use pulsar_marketlab::trading_stage::{
    analytics_prim_path, asset_prim_path, stage_time_from_bar_date, MarketStage,
};
use pulsar_marketlab::technical_analysis::{
    build_ta_evaluation_closure, compute_ta_at_playhead_from_stage, ta_indicator_label,
    MarketSeriesWindow, DEFAULT_TA_INDICATOR_ID,
    DEFAULT_TA_LOOKBACK,
};

pub const DEFAULT_CSV_ASSET_PATH: &str = "data/SPY.csv";
pub(crate) const CHART_Y_PADDING_RATIO: f32 = 0.08;
pub(crate) const CHART_Y_MIN_SPAN: f32 = 1.0;
pub(crate) const CHART_STROKE_WIDTH: f32 = 1.5;
pub(crate) const STATUS_LOG_CAP: usize = 64;
const INGESTION_POLL_INTERVAL: Duration = Duration::from_millis(16);
const TA_RSI_OVERBOUGHT: f64 = 70.0;
const TA_RSI_OVERSOLD: f64 = 30.0;
pub const SIM_DEPLOY_FRACTION: f64 = 0.95;
pub const SIM_INITIAL_CASH: f64 = 10_000.0;
pub const SHARPE_ANNUALIZATION: f64 = 252.0;
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
    /// Layer 2 simulation ledger metrics for Portfolio inspector + node cards.
    PortfolioMetrics {
        simulation_epoch: u64,
        tick_index: usize,
        tick_label: Option<String>,
        nav: f64,
        cash: f64,
        position_qty: f64,
        mark_price: f64,
        total_return_pct: f64,
        max_drawdown_pct: f64,
        sharpe_ratio: Option<f64>,
        bars_processed: usize,
        trade_count: u32,
        benchmark_return_pct: Option<f64>,
        excess_return_pct: Option<f64>,
        avg_exposure_pct: f64,
    },
    /// Clear UI portfolio diagnostics and reset Layer 2 ledger baselines at CSV EOF.
    ResetSimulation {
        simulation_epoch: u64,
    },
    /// Global synchronized chart playhead index (0-based bar into the active OHLC series).
    PlayheadSet {
        index: usize,
        total_bars: usize,
        #[allow(dead_code)]
        tick_label: Option<String>,
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

fn mirror_ledger_transaction(
    tx: &Sender<PipelineSystemMessage>,
    stage: &MarketStage,
    transaction: &SimulationTransaction,
) {
    let cash = StageSimulationLedger::cash_at(stage, transaction.time);
    let _ = tx.send(PipelineSystemMessage::StageSample {
        prim_path: EXECUTION_CASH_PATH.to_string(),
        attribute: EXECUTION_CASH_ATTR.to_string(),
        time: transaction.time,
        value: cash as f32,
    });
    for (ticker, _) in &transaction.position_deltas {
        if let Ok(path) = position_prim_path(ticker) {
            let shares = StageSimulationLedger::shares_at(stage, ticker, transaction.time);
            let _ = tx.send(PipelineSystemMessage::StageSample {
                prim_path: path,
                attribute: "shares".to_string(),
                time: transaction.time,
                value: shares as f32,
            });
        }
    }
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

fn sim_buy_qty(cash: f64, price: f64) -> f64 {
    if price <= f64::EPSILON || cash <= f64::EPSILON {
        return 0.0;
    }
    ((cash * SIM_DEPLOY_FRACTION) / price).floor()
}

fn nav_at_time(stage: &MarketStage, t: f64, tickers: &[&str]) -> Option<f64> {
    if !t.is_finite() {
        return None;
    }
    Some(StageSimulationLedger::nav_at_time(stage, t, tickers))
}

fn resolve_mark_price_at_time(
    stage: &MarketStage,
    ticker: &str,
    t: f64,
    fallback: f64,
) -> f64 {
    StageSimulationLedger::mark_price_at(stage, ticker, t).unwrap_or_else(|| {
        if fallback.is_finite() && fallback > 0.0 {
            fallback
        } else {
            fallback.max(0.0)
        }
    })
}

fn compute_portfolio_diagnostics(
    nav_history: &[f64],
    mark_prices: &[f64],
    exposure_samples: &[f64],
    trade_count: u32,
    simulation_epoch: u64,
    tick_index: usize,
    tick_label: Option<String>,
    mark_price: f64,
    cash: f64,
    position_qty: f64,
    initial_cash: f64,
) -> PortfolioDiagnosticsSnapshot {
    let mut sanitized_marks = mark_prices.to_vec();
    sanitize_mark_price_series(&mut sanitized_marks);
    compute_metrics_from_nav_history(
        nav_history,
        &sanitized_marks,
        exposure_samples,
        trade_count,
        simulation_epoch,
        tick_index,
        tick_label,
        mark_price,
        cash,
        position_qty,
        initial_cash,
    )
}

fn sanitize_mark_price_series(mark_prices: &mut [f64]) {
    let mut last_valid = mark_prices
        .iter()
        .copied()
        .find(|price| price.is_finite() && *price > 0.0)
        .unwrap_or(0.0);
    for price in mark_prices.iter_mut() {
        if price.is_finite() && *price > 0.0 {
            last_valid = *price;
        } else {
            *price = last_valid;
        }
    }
}

fn compute_metrics_from_nav_history(
    nav_history: &[f64],
    mark_prices: &[f64],
    exposure_samples: &[f64],
    trade_count: u32,
    simulation_epoch: u64,
    tick_index: usize,
    tick_label: Option<String>,
    mark_price: f64,
    cash: f64,
    position_qty: f64,
    initial_cash: f64,
) -> PortfolioDiagnosticsSnapshot {
    let nav = nav_history.last().copied().unwrap_or(initial_cash);
    let bars_processed = nav_history.len();
    let total_return_pct = if initial_cash.abs() > f64::EPSILON {
        (nav - initial_cash) / initial_cash
    } else {
        0.0
    };

    let benchmark_return_pct = if mark_prices.len() >= 2 {
        let first = mark_prices[0];
        let last = *mark_prices.last().unwrap_or(&first);
        if first.abs() > f64::EPSILON {
            Some((last / first) - 1.0)
        } else {
            None
        }
    } else {
        None
    };
    let excess_return_pct = benchmark_return_pct.map(|benchmark| total_return_pct - benchmark);

    let mut peak_nav = f64::NEG_INFINITY;
    let mut max_drawdown_pct: f64 = 0.0;
    for sample in nav_history {
        peak_nav = peak_nav.max(*sample);
        if peak_nav > f64::EPSILON {
            max_drawdown_pct = max_drawdown_pct.max((peak_nav - sample) / peak_nav);
        }
    }

    let sharpe_ratio = if nav_history.len() >= 3 {
        let returns: Vec<f64> = nav_history
            .windows(2)
            .filter_map(|pair| {
                if pair[0].abs() > f64::EPSILON {
                    Some((pair[1] / pair[0]) - 1.0)
                } else {
                    None
                }
            })
            .collect();
        if returns.len() >= 2 {
            let mean = returns.iter().sum::<f64>() / returns.len() as f64;
            let variance = returns
                .iter()
                .map(|sample| {
                    let diff = sample - mean;
                    diff * diff
                })
                .sum::<f64>()
                / returns.len() as f64;
            let std_dev = variance.sqrt();
            if std_dev > f64::EPSILON {
                Some((mean / std_dev) * SHARPE_ANNUALIZATION.sqrt())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let avg_exposure_pct = if exposure_samples.is_empty() {
        0.0
    } else {
        exposure_samples.iter().sum::<f64>() / exposure_samples.len() as f64
    };

    PortfolioDiagnosticsSnapshot {
        simulation_epoch,
        tick_index,
        tick_label,
        nav,
        cash,
        position_qty,
        mark_price,
        total_return_pct,
        max_drawdown_pct,
        sharpe_ratio,
        bars_processed,
        trade_count,
        benchmark_return_pct,
        excess_return_pct,
        avg_exposure_pct,
    }
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
fn baseline_portfolio_snapshot(
    playhead: usize,
    bars_len: usize,
    tick_label: Option<String>,
) -> PortfolioDiagnosticsSnapshot {
    let tick_index = playhead.min(bars_len.saturating_sub(1));
    compute_metrics_from_nav_history(
        &[],
        &[],
        &[],
        0,
        0,
        tick_index,
        tick_label.or_else(|| {
            if bars_len == 0 {
                None
            } else {
                Some(format!("baseline · bar {}/{}", tick_index + 1, bars_len))
            }
        }),
        0.0,
        SIM_INITIAL_CASH,
        0.0,
        SIM_INITIAL_CASH,
    )
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
        if ta_node.node_type != NodeType::TechnicalAnalysis {
            continue;
        }
        if wired.iter().any(|existing: &&VisualNode| existing.id == ta_node.id) {
            continue;
        }
        wired.push(ta_node);
    }
    wired
}

pub(crate) struct TaExecutionBridge {
    prior_values: HashMap<usize, f64>,
    /// NAV samples for the current CSV playback epoch only (cleared at EOF).
    nav_history: Vec<f64>,
    /// Mark prices aligned with each NAV sample for buy-and-hold benchmark.
    mark_prices: Vec<f64>,
    /// Position value / NAV at each simulation step.
    exposure_samples: Vec<f64>,
    trade_count: u32,
    /// Monotonic tick within the current epoch; resets to 0 on replay.
    simulation_tick: usize,
    /// Incremented on every replay so UI can ignore stale metric frames.
    simulation_epoch: u64,
    /// Continuous-time execution ledger for the CSV simulation thread.
    simulation_stage: MarketStage,
}

impl TaExecutionBridge {
    pub(crate) fn new() -> Self {
        let mut bridge = Self {
            prior_values: HashMap::new(),
            nav_history: Vec::new(),
            mark_prices: Vec::new(),
            exposure_samples: Vec::new(),
            trade_count: 0,
            simulation_tick: 0,
            simulation_epoch: 0,
            simulation_stage: MarketStage::new(),
        };
        bridge.reset_simulation_ledger();
        bridge
    }

    pub(crate) fn simulation_stage_mut(&mut self) -> &mut MarketStage {
        &mut self.simulation_stage
    }

    pub(crate) fn reset_simulation_ledger(&mut self) {
        StageSimulationLedger::reset_execution_paths(&mut self.simulation_stage);
        let _ = StageSimulationLedger::seed_initial_cash(&mut self.simulation_stage, SIM_INITIAL_CASH);
    }

    fn simulation_epoch(&self) -> u64 {
        self.simulation_epoch
    }

    pub(crate) fn begin_new_epoch(&mut self) {
        self.simulation_epoch = self.simulation_epoch.saturating_add(1);
        self.prior_values.clear();
        self.nav_history.clear();
        self.mark_prices.clear();
        self.exposure_samples.clear();
        self.trade_count = 0;
        self.simulation_tick = 0;
        self.reset_simulation_ledger();
    }

    pub(crate) fn clear_ta_signal_slot(&mut self, ta_node_id: usize, ui_stage: &mut MarketStage) {
        self.prior_values.remove(&ta_node_id);
        let indicator_id = format!("ta_{ta_node_id}");
        if let Ok(path) = analytics_prim_path(&indicator_id) {
            ui_stage.prims.remove(&path);
            self.simulation_stage.prims.remove(&path);
        }
    }

    pub(crate) fn record_market_price(
        stage: &mut MarketStage,
        ticker: &str,
        bar_time: f64,
        price: f64,
    ) {
        if !price.is_finite() || price <= 0.0 || !bar_time.is_finite() {
            return;
        }
        if let Ok(prim) = asset_prim_path(ticker) {
            let _ = stage.set_sample(&prim, "close", bar_time, price as f32);
        }
    }

    fn ingest_ta_sample(
        &mut self,
        node: &VisualNode,
        bar_time: f64,
        value: Option<f64>,
        price: f64,
        asset_label: &str,
        tx: &Sender<PipelineSystemMessage>,
    ) {
        let Some(value) = value else {
            return;
        };
        if !value.is_finite() || !bar_time.is_finite() {
            return;
        }

        let indicator_id = analytics_indicator_id(node);
        if let Ok(prim) = analytics_prim_path(&indicator_id) {
            let _ = self
                .simulation_stage
                .set_sample(&prim, "value", bar_time, value as f32);
        }
        Self::record_market_price(&mut self.simulation_stage, asset_label, bar_time, price);

        if node.ta_indicator_id.as_deref() == Some("rsi") {
            self.evaluate_rsi_crossing(
                node.id,
                bar_time,
                value,
                price,
                asset_label,
                tx,
            );
        }
    }

    fn evaluate_rsi_crossing(
        &mut self,
        ta_node_id: usize,
        bar_time: f64,
        value: f64,
        price: f64,
        asset_label: &str,
        tx: &Sender<PipelineSystemMessage>,
    ) {
        let prior = self.prior_values.insert(ta_node_id, value);
        let Some(prior) = prior.filter(|sample| sample.is_finite()) else {
            return;
        };

        if prior <= TA_RSI_OVERSOLD && value > TA_RSI_OVERSOLD {
            let cash = StageSimulationLedger::cash_at(&self.simulation_stage, bar_time);
            let qty = sim_buy_qty(cash, price);
            if qty <= f64::EPSILON {
                return;
            }
            let cost = price * qty;
            let transaction = SimulationTransaction {
                time: bar_time,
                cash_delta: -cost,
                position_deltas: vec![(asset_label.to_string(), qty)],
            };
            if ExecutionEngine::apply_transaction(&mut self.simulation_stage, &transaction).is_ok() {
                mirror_ledger_transaction(tx, &self.simulation_stage, &transaction);
                self.trade_count = self.trade_count.saturating_add(1);
                let _ = tx.send(PipelineSystemMessage::StatusAlert {
                    text: format!(
                        "SIM BUY — {qty:.0} {asset_label} @ {price:.2} (TA node {ta_node_id} RSI {value:.1} crossed above {TA_RSI_OVERSOLD:.0})"
                    ),
                });
            }
        } else if prior >= TA_RSI_OVERBOUGHT && value < TA_RSI_OVERBOUGHT {
            let qty = StageSimulationLedger::shares_at(&self.simulation_stage, asset_label, bar_time);
            if qty <= f64::EPSILON {
                return;
            }
            let proceeds = price * qty;
            let transaction = SimulationTransaction {
                time: bar_time,
                cash_delta: proceeds,
                position_deltas: vec![(asset_label.to_string(), -qty)],
            };
            if ExecutionEngine::apply_transaction(&mut self.simulation_stage, &transaction).is_ok() {
                mirror_ledger_transaction(tx, &self.simulation_stage, &transaction);
                self.trade_count = self.trade_count.saturating_add(1);
                let _ = tx.send(PipelineSystemMessage::StatusAlert {
                    text: format!(
                        "SIM SELL — {qty:.0} {asset_label} @ {price:.2} (TA node {ta_node_id} RSI {value:.1} crossed below {TA_RSI_OVERBOUGHT:.0})"
                    ),
                });
            }
        }
    }

    fn metrics_inputs(&self) -> (&[f64], &[f64], &[f64], u32) {
        (
            &self.nav_history,
            &self.mark_prices,
            &self.exposure_samples,
            self.trade_count,
        )
    }

    /// Append NAV for the current simulation step (CSV feeder live alerts path).
    pub(crate) fn finish_simulation_tick(&mut self, bar_time: f64, tickers: &[&str], mark_price: f64) {
        let resolved_mark = resolve_mark_price_at_time(&self.simulation_stage, tickers[0], bar_time, mark_price);
        if let Some(nav) = nav_at_time(&self.simulation_stage, bar_time, tickers) {
            let position_qty = tickers
                .first()
                .map(|ticker| StageSimulationLedger::shares_at(&self.simulation_stage, ticker, bar_time))
                .unwrap_or(0.0);
            let exposure = if nav.abs() > f64::EPSILON {
                (position_qty * resolved_mark) / nav
            } else {
                0.0
            };
            self.nav_history.push(nav);
            self.mark_prices.push(resolved_mark);
            self.exposure_samples.push(exposure);
        }
        self.simulation_tick += 1;
    }

    pub(crate) fn publish_baseline(&self, tx: &Sender<PipelineSystemMessage>) {
        let metrics = compute_metrics_from_nav_history(
            &[],
            &[],
            &[],
            0,
            self.simulation_epoch,
            0,
            Some("baseline".to_string()),
            0.0,
            SIM_INITIAL_CASH,
            0.0,
            SIM_INITIAL_CASH,
        );
        let _ = tx.send(PipelineSystemMessage::PortfolioMetrics {
            simulation_epoch: self.simulation_epoch,
            tick_index: metrics.tick_index,
            tick_label: metrics.tick_label,
            nav: metrics.nav,
            cash: metrics.cash,
            position_qty: metrics.position_qty,
            mark_price: metrics.mark_price,
            total_return_pct: metrics.total_return_pct,
            max_drawdown_pct: metrics.max_drawdown_pct,
            sharpe_ratio: metrics.sharpe_ratio,
            bars_processed: metrics.bars_processed,
            trade_count: metrics.trade_count,
            benchmark_return_pct: metrics.benchmark_return_pct,
            excess_return_pct: metrics.excess_return_pct,
            avg_exposure_pct: metrics.avg_exposure_pct,
        });
    }
}

pub fn restart_csv_playback(
    playbacks: &mut [CsvAssetPlayback],
    ta_execution: &mut TaExecutionBridge,
    tx: &Sender<PipelineSystemMessage>,
) {
    ta_execution.begin_new_epoch();
    for playback in playbacks.iter_mut() {
        playback.cursor = 0;
        playback.reader_paused = playback.rows.is_empty();
    }
    let epoch = ta_execution.simulation_epoch();
    let _ = tx.send(PipelineSystemMessage::ResetSimulation { simulation_epoch: epoch });
    ta_execution.publish_baseline(tx);
    let active_sources = playbacks
        .iter()
        .filter(|playback| !playback.rows.is_empty())
        .count();
    if let Some(playback) = playbacks.iter().find(|p| !p.rows.is_empty()) {
        send_playhead_set(tx, 0, playback.rows.len(), None);
    }
    let _ = tx.send(PipelineSystemMessage::StatusAlert {
        text: format!(
            "CSV replay started — epoch {epoch}, {active_sources} source(s) @ {}ms/tick",
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
            let last_index = playback.rows.len().saturating_sub(1);
            let _ = tx.send(PipelineSystemMessage::PlayheadSet {
                index: last_index,
                total_bars: playback.rows.len(),
                tick_label: last_label.clone(),
            });
        }
    }
    let _ = tx.send(PipelineSystemMessage::StatusAlert {
        text: format!(
            "CSV playback complete — scrub playhead or change graph to replay{}",
            last_label
                .map(|date| format!(" (last bar {date})"))
                .unwrap_or_default()
        ),
    });
}

pub fn send_playhead_set(
    tx: &Sender<PipelineSystemMessage>,
    index: usize,
    total_bars: usize,
    tick_label: Option<String>,
) {
    let _ = tx.send(PipelineSystemMessage::PlayheadSet {
        index,
        total_bars,
        tick_label,
    });
}

pub fn csv_playback_is_active(playbacks: &[CsvAssetPlayback]) -> bool {
    playbacks
        .iter()
        .any(|playback| !playback.reader_paused && !playback.rows.is_empty())
}

type TaExecutionSideEffects<'a> = (
    &'a mut TaExecutionBridge,
    &'a Sender<PipelineSystemMessage>,
);

pub fn ta_tick_messages_for_asset(
    asset_node_id: usize,
    from_port_idx: usize,
    tick_index: usize,
    tick_label: Option<String>,
    asset_source: &str,
    window: &MarketSeriesWindow,
    graph: &PipelineGraphSnapshot,
    price: f64,
    execution: Option<TaExecutionSideEffects<'_>>,
    portfolio_ta_filter: Option<&HashSet<usize>>,
) -> Vec<PipelineSystemMessage> {
    let mut messages = Vec::new();
    let wired_nodes = wired_ta_nodes_for_asset_port(asset_node_id, from_port_idx, graph);

    let push_messages = |messages: &mut Vec<PipelineSystemMessage>,
                         wired_nodes: &[&VisualNode],
                         execution: Option<TaExecutionSideEffects<'_>>| {
        match execution {
            Some((bridge, tx)) => {
                for node in wired_nodes {
                    if portfolio_ta_filter.is_some_and(|allowed| !allowed.contains(&node.id)) {
                        continue;
                    }
                    let Some(indicator_id) = node.ta_indicator_id.as_deref() else {
                        continue;
                    };
                    let label = ta_indicator_label(indicator_id).unwrap_or(indicator_id);
                    let value = ta_compute_for_node(node, window);
                    let bar_time = tick_label
                        .as_deref()
                        .and_then(stage_time_from_bar_date)
                        .unwrap_or(tick_index as f64);
                    bridge.ingest_ta_sample(
                        node,
                        bar_time,
                        value,
                        price,
                        asset_source,
                        tx,
                    );
                    messages.push(PipelineSystemMessage::TickUpdate {
                        tick_index,
                        tick_label: tick_label.clone(),
                        node_id: node.id,
                        source: format!("{asset_source} ({label})"),
                        value: format_stream_indicator(value),
                    });
                }
            }
            None => {
                for node in wired_nodes {
                    if portfolio_ta_filter.is_some_and(|allowed| !allowed.contains(&node.id)) {
                        continue;
                    }
                    let Some(indicator_id) = node.ta_indicator_id.as_deref() else {
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
            }
        }
    };

    push_messages(&mut messages, &wired_nodes, execution);
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
/// Replay simulation from bar 0 through `playhead` using the compiled DAG execution order.
fn evaluate_portfolio_at_playhead(
    ohlc_by_asset: &HashMap<usize, Vec<OhlcBar>>,
    playhead: usize,
    graph: &PipelineGraphSnapshot,
) -> Option<PortfolioDiagnosticsSnapshot> {
    let csv_assets: Vec<usize> = csv_backed_asset_ids(graph).into_iter().collect();
    if csv_assets.is_empty() {
        return None;
    }

    let primary_asset = *csv_assets.first()?;
    let primary_bars = ohlc_by_asset.get(&primary_asset)?;
    if primary_bars.is_empty() {
        return None;
    }
    let end = playhead.min(primary_bars.len().saturating_sub(1));
    let tick_label = primary_bars.get(end).map(|bar| {
        format!(
            "bar {}/{} · {}",
            end + 1,
            primary_bars.len(),
            bar.date
        )
    });

    if !graph.dag_valid {
        return Some(baseline_portfolio_snapshot(
            end,
            primary_bars.len(),
            Some("invalid graph cycle — execution halted".to_string()),
        ));
    }

    let portfolio_ta_filter = portfolio_wired_ta_node_ids(graph);
    if portfolio_ta_filter.is_empty() {
        return Some(baseline_portfolio_snapshot(
            end,
            primary_bars.len(),
            tick_label,
        ));
    }

    let mut yahoo_by_asset: HashMap<usize, Vec<YahooCsvRow>> = HashMap::new();
    for asset_id in &csv_assets {
        let Some(bars) = ohlc_by_asset.get(asset_id) else {
            continue;
        };
        if bars.is_empty() {
            continue;
        }
        yahoo_by_asset.insert(*asset_id, yahoo_rows_from_ohlc_bars(bars));
    }
    if yahoo_by_asset.is_empty() {
        return None;
    }

    let mut bridge = TaExecutionBridge::new();
    let sink = mpsc::channel::<PipelineSystemMessage>().0;
    let primary_ticker = graph
        .nodes
        .iter()
        .find(|node| node.id == primary_asset)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| "SPY".to_string());
    let tickers = [primary_ticker.as_str()];

    let nodes_by_id: HashMap<usize, VisualNode> = graph
        .nodes
        .iter()
        .map(|node| (node.id, node.clone()))
        .collect();

    for tick in 0..=end {
        let bar_time = stage_time_from_bar_date(&primary_bars[tick].date)?;
        let mut last_close = primary_bars
            .get(tick)
            .map(|bar| bar.close)
            .filter(|price| price.is_finite() && *price > 0.0)
            .unwrap_or(0.0);

        for &node_id in &graph.execution_order {
            let Some(node) = nodes_by_id.get(&node_id) else {
                continue;
            };
            match node.node_type {
                NodeType::Asset => {
                    let Some(rows) = yahoo_by_asset.get(&node_id) else {
                        continue;
                    };
                    if tick >= rows.len() {
                        continue;
                    }
                    let row = &rows[tick];
                    if row.close.is_finite() && row.close > 0.0 {
                        TaExecutionBridge::record_market_price(
                            bridge.simulation_stage_mut(),
                            &node.name,
                            bar_time,
                            row.close,
                        );
                        last_close = row.close;
                    }
                }
                NodeType::TechnicalAnalysis => {
                    if !portfolio_ta_filter.contains(&node_id) {
                        continue;
                    }
                    let Some(asset_id) = upstream_asset_for_ta_node(node_id, graph) else {
                        continue;
                    };
                    let Some(rows) = yahoo_by_asset.get(&asset_id) else {
                        continue;
                    };
                    if tick >= rows.len() {
                        continue;
                    }
                    let row = &rows[tick];
                    let window = market_window_from_yahoo_rows(rows, tick + 1);
                    let price = row.close;
                    if !(price.is_finite() && price > 0.0) {
                        continue;
                    }
                    last_close = price;
                    let ticker = graph
                        .nodes
                        .iter()
                        .find(|entry| entry.id == asset_id)
                        .map(|entry| entry.name.clone())
                        .unwrap_or_else(|| format!("asset {asset_id}"));
                    let Some(indicator_id) = node.ta_indicator_id.as_deref() else {
                        continue;
                    };
                    let closure = build_ta_evaluation_closure(
                        indicator_id.to_string(),
                        window.clone(),
                    );
                    let value = (closure.run)(tick, ta_lookback_for_node(node))
                        .map(f64::from);
                    bridge.ingest_ta_sample(
                        node,
                        bar_time,
                        value,
                        price,
                        &ticker,
                        &sink,
                    );
                }
                NodeType::Portfolio => {}
            }
        }

        bridge.finish_simulation_tick(bar_time, &tickers, last_close);
    }

    let playhead_time = stage_time_from_bar_date(&primary_bars[end].date)?;
    let bar_close = primary_bars.get(end).map(|bar| bar.close).unwrap_or(0.0);
    let mark_price = resolve_mark_price_at_time(
        &bridge.simulation_stage,
        &primary_ticker,
        playhead_time,
        bar_close,
    );
    let cash = StageSimulationLedger::cash_at(&bridge.simulation_stage, playhead_time);
    let position_qty =
        StageSimulationLedger::shares_at(&bridge.simulation_stage, &primary_ticker, playhead_time);
    let (nav_history, mark_prices, exposure_samples, trade_count) = bridge.metrics_inputs();
    Some(compute_portfolio_diagnostics(
        nav_history,
        mark_prices,
        exposure_samples,
        trade_count,
        0,
        end,
        tick_label,
        mark_price,
        cash,
        position_qty,
        SIM_INITIAL_CASH,
    ))
}

pub fn preload_asset_charts_from_nodes(nodes: &[VisualNode]) -> HashMap<usize, ChartHistoryBuffer> {
    let mut history = HashMap::new();
    for node in nodes {
        if !node.node_type.displays_price_chart() {
            continue;
        }
        let Some(AssetSourceType::Csv { path }) = &node.asset_source else {
            continue;
        };
        if let Ok((_, rows)) = load_yahoo_finance_csv(path) {
            history.insert(node.id, chart_buffer_from_csv_rows(&rows));
        }
    }
    history
}

pub fn preload_asset_ohlc_from_nodes(nodes: &[VisualNode]) -> HashMap<usize, Vec<OhlcBar>> {
    let mut history = HashMap::new();
    for node in nodes {
        if !node.node_type.displays_price_chart() {
            continue;
        }
        let Some(AssetSourceType::Csv { path }) = &node.asset_source else {
            continue;
        };
        if let Ok((_, rows)) = load_yahoo_finance_csv(path) {
            let bars = ohlc_bars_from_csv_rows(&rows);
            if !bars.is_empty() {
                history.insert(node.id, bars);
            }
        }
    }
    history
}

fn analytics_indicator_id(node: &VisualNode) -> String {
    node.ta_indicator_id
        .clone()
        .unwrap_or_else(|| format!("ta_{}", node.id))
}

fn refresh_ta_samples_at_playhead(
    market_stage: &mut MarketStage,
    nodes: &[VisualNode],
    graph: &PipelineGraphSnapshot,
    playhead_time: f64,
) {
    let portfolio_ta = portfolio_wired_ta_node_ids(graph);
    for node in nodes {
        if node.node_type != NodeType::TechnicalAnalysis || !portfolio_ta.contains(&node.id) {
            continue;
        }
        let Some(asset_id) = upstream_asset_for_ta_node(node.id, graph) else {
            continue;
        };
        let Some(asset) = graph.nodes.iter().find(|entry| entry.id == asset_id) else {
            continue;
        };
        let Some(indicator_id) = node.ta_indicator_id.as_deref() else {
            continue;
        };
        let lookback = ta_lookback_for_node(node);
        let Some(value) = compute_ta_at_playhead_from_stage(
            market_stage,
            &asset.name,
            indicator_id,
            playhead_time,
            lookback,
        ) else {
            continue;
        };
        let indicator_key = analytics_indicator_id(node);
        if let Ok(prim) = analytics_prim_path(&indicator_key) {
            let _ = market_stage.set_sample(&prim, "value", playhead_time, value as f32);
        }
    }
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
        if !matches!(node.node_type, NodeType::Asset) {
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

fn stage_time_for_bar_index(bars: &[OhlcBar], index: usize) -> Option<f64> {
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
    pub asset_ohlc_history: HashMap<usize, Vec<OhlcBar>>,
    /// Phase B Layer 1 market stage (path-addressable time-sampled attributes).
    pub(crate) market_stage: MarketStage,
    pub selected_node_id: Option<usize>,
    pub active_drag_node_id: Option<usize>,
    pub drag_offset: Point<Pixels>,
    pub canvas_origin: Point<Pixels>,
    pub active_wire_source: Option<(usize, usize)>,
    pub active_mouse_pos: Point<Pixels>,
    pub context_menu_pos: Option<Point<Pixels>>,
    pub pan_offset: Point<Pixels>,
    pub zoom_scale: f32,
    pub is_panning: bool,
    pub last_pan_mouse_pos: Point<Pixels>,
    /// Active category shelf tab in the TA indicator picker.
    pub(crate) ta_inspector_category: Option<String>,
    /// Latest Layer 2 portfolio diagnostics from the simulation ledger.
    pub(crate) portfolio_diagnostics: Option<PortfolioDiagnosticsSnapshot>,
    /// Ignore stale portfolio metric frames from prior CSV playback epochs.
    pub(crate) portfolio_metrics_epoch: u64,
    /// Global synchronized OHLC playhead (0-based bar index).
    pub(crate) playhead_current: usize,
    /// Continuous stage coordinate for the active playhead (derived from bar date).
    pub(crate) playhead_time: f64,
    pub(crate) playhead_total_bars: usize,
    pub(crate) playhead_scrubbing: bool,
    pub(crate) ohlc_chart_bounds: Option<Bounds<Pixels>>,
    /// Cached `(playhead_time, graph_revision)` for playhead evaluation short-circuit.
    pub(crate) last_calculated_state: (f64, u64),
    /// Editable CSV path field for the selected asset node.
    pub(crate) asset_path_input: Entity<AssetPathInput>,
    /// Cached bounds for the TA lookback slider track (inspector sidebar).
    pub(crate) ta_lookback_slider_bounds: Option<Bounds<Pixels>>,
}
pub fn default_pipeline_nodes() -> Vec<VisualNode> {
    vec![
        VisualNode {
            id: 1,
            name: csv_node_label_from_path(DEFAULT_CSV_ASSET_PATH),
            node_type: NodeType::Asset,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: None,
            ta_lookback_period: DEFAULT_TA_LOOKBACK as u32,
            dsl_formula: None,
            asset_source: Some(AssetSourceType::Csv {
                path: DEFAULT_CSV_ASSET_PATH.to_string(),
            }),
            x: 60.0,
            y: 70.0,
            inputs: vec![],
            outputs: vec!["Close Out".to_string()],
        },
        VisualNode {
            id: 2,
            name: ta_indicator_label(DEFAULT_TA_INDICATOR_ID)
                .unwrap_or("RSI")
                .to_string(),
            node_type: NodeType::TechnicalAnalysis,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: Some(DEFAULT_TA_INDICATOR_ID.to_string()),
            ta_lookback_period: DEFAULT_TA_LOOKBACK as u32,
            dsl_formula: None,
            asset_source: None,
            x: 320.0,
            y: 130.0,
            inputs: vec!["Price In".to_string()],
            outputs: vec!["TA Out".to_string()],
        },
        VisualNode {
            id: 4,
            name: "Sim Portfolio".to_string(),
            node_type: NodeType::Portfolio,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: None,
            ta_lookback_period: DEFAULT_TA_LOOKBACK as u32,
            dsl_formula: None,
            asset_source: None,
            x: 600.0,
            y: 130.0,
            inputs: vec![portfolio_signal_port_label(0)],
            outputs: vec!["NAV Out".to_string()],
        },
    ]
}

pub fn default_pipeline_connections() -> Vec<NodeConnection> {
    vec![
        NodeConnection {
            from_node_id: 1,
            from_port_idx: 0,
            to_node_id: 2,
            to_port_idx: 0,
        },
        NodeConnection {
            from_node_id: 2,
            from_port_idx: 0,
            to_node_id: 4,
            to_port_idx: 0,
        },
    ]
}
impl TradingSystemWorkspace {
    pub fn new(
        rx: Receiver<PipelineSystemMessage>,
        csv_path_registry: SharedCsvAssetPaths,
        pipeline_graph: SharedPipelineGraph,
        cx: &mut Context<Self>,
    ) -> Self {
        let asset_path_input = cx.new(|cx| AssetPathInput::new(DEFAULT_CSV_ASSET_PATH, cx));
        cx.subscribe(
            &asset_path_input,
            |this, _, event: &PathInputEvent, cx| {
                this.on_asset_path_input_event(event, cx);
            },
        )
        .detach();

        let nodes = default_pipeline_nodes();
        let asset_ohlc_history = preload_asset_ohlc_from_nodes(&nodes);
        let mut market_stage = MarketStage::new();
        hydrate_market_stage_from_workspace(&mut market_stage, &nodes, &asset_ohlc_history);
        let mut workspace = Self {
            nodes: nodes.clone(),
            connections: default_pipeline_connections(),
            inspector_data: Vec::new(),
            pipeline_status_log: vec![
                "Pipeline status console ready.".to_string(),
            ],
            csv_path_registry,
            pipeline_graph,
            asset_chart_history: preload_asset_charts_from_nodes(&nodes),
            asset_ohlc_history,
            market_stage,
            selected_node_id: None,
            active_drag_node_id: None,
            drag_offset: point(px(0.0), px(0.0)),
            canvas_origin: point(px(0.0), px(0.0)),
            active_wire_source: None,
            active_mouse_pos: point(px(0.0), px(0.0)),
            context_menu_pos: None,
            pan_offset: point(px(0.0), px(0.0)),
            zoom_scale: 1.0,
            is_panning: false,
            last_pan_mouse_pos: point(px(0.0), px(0.0)),
            ta_inspector_category: None,
            portfolio_diagnostics: None,
            portfolio_metrics_epoch: 0,
            playhead_current: 0,
            playhead_time: 0.0,
            playhead_total_bars: 0,
            playhead_scrubbing: false,
            ohlc_chart_bounds: None,
            last_calculated_state: (f64::NAN, u64::MAX),
            asset_path_input,
            ta_lookback_slider_bounds: None,
        };

        workspace.sync_playhead_bounds();
        workspace.sync_playhead_time_from_index();
        workspace.synchronize_inspector_view();
        workspace.recompute_playhead_diagnostics();
        workspace.sync_pipeline_graph();
        workspace.spawn_pipeline_ingestion_worker(rx, cx);
        workspace
    }

    pub(crate) fn sync_pipeline_graph(&self) {
        self.pipeline_graph
            .replace(self.nodes.clone(), self.connections.clone());
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
            match node.node_type {
                NodeType::Asset if matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) => {
                    return self
                        .asset_ohlc_history
                        .get(&node_id)
                        .cloned()
                        .unwrap_or_default();
                }
                NodeType::TechnicalAnalysis => {
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

    pub(crate) fn sync_playhead_bounds(&mut self) {
        let bars = self.chart_bars_for_selection();
        self.playhead_total_bars = bars.len();
        if self.playhead_total_bars == 0 {
            self.playhead_current = 0;
        } else {
            self.playhead_current = self
                .playhead_current
                .min(self.playhead_total_bars - 1);
        }
        self.sync_playhead_time_from_index();
    }

    pub(crate) fn sync_playhead_time_from_index(&mut self) {
        let bars = self.chart_bars_for_selection();
        if bars.is_empty() {
            self.playhead_time = 0.0;
            return;
        }
        let index = self.playhead_current.min(bars.len() - 1);
        self.playhead_time =
            stage_time_for_bar_index(&bars, index).unwrap_or(self.playhead_current as f64);
    }

    pub(crate) fn playhead_tick_label(&self) -> String {
        let bars = self.chart_bars_for_selection();
        if bars.is_empty() {
            return format!("t={:.2}", self.playhead_time);
        }
        let index = self.playhead_current.min(bars.len() - 1);
        format!(
            "{}/{} · {}",
            index + 1,
            bars.len(),
            bars[index].date
        )
    }

    /// Rebuild the global inspector register from stage lookups at [`playhead_time`].
    pub(crate) fn synchronize_inspector_view(&mut self) {
        if self.playhead_total_bars == 0 {
            return;
        }
        let t = self.playhead_time;
        let graph = self.pipeline_graph.snapshot();
        refresh_ta_samples_at_playhead(&mut self.market_stage, &self.nodes, &graph, t);
        let tick = self.playhead_tick_label();
        let mut rows = Vec::new();

        for node in &self.nodes {
            match node.node_type {
                NodeType::Asset if matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) => {
                    let Ok(prim) = asset_prim_path(&node.name) else {
                        continue;
                    };
                    let Some(close) = self.market_stage.resolve_attribute_at(&prim, "close", t)
                    else {
                        continue;
                    };
                    rows.push(MatrixDataRow {
                        tick: tick.clone(),
                        asset: node.name.clone(),
                        grade_type: format!("{:?}", node.grade),
                        multivector_value: format!("[{close:.2}]"),
                        associated_node_id: Some(node.id),
                    });
                }
                NodeType::TechnicalAnalysis => {
                    let indicator_id = analytics_indicator_id(node);
                    let Ok(prim) = analytics_prim_path(&indicator_id) else {
                        continue;
                    };
                    let Some(value) = self.market_stage.resolve_attribute_at(&prim, "value", t)
                    else {
                        continue;
                    };
                    rows.push(MatrixDataRow {
                        tick: tick.clone(),
                        asset: format!("{} ({})", node.name, indicator_id),
                        grade_type: format!("{:?}", node.grade),
                        multivector_value: format!("[{value:.4}]"),
                        associated_node_id: Some(node.id),
                    });
                }
                _ => {}
            }
        }

        if rows.is_empty() {
            return;
        }
        self.inspector_data = rows;
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

    pub(crate) fn recompute_playhead_diagnostics(&mut self) {
        if self.playhead_total_bars < 1 {
            self.portfolio_diagnostics = None;
            return;
        }
        let graph_revision = self.pipeline_graph.revision();
        if self.playhead_time == self.last_calculated_state.0
            && graph_revision == self.last_calculated_state.1
        {
            return;
        }
        self.sync_playhead_time_from_index();
        self.synchronize_inspector_view();
        let graph = self.pipeline_graph.snapshot();
        self.portfolio_diagnostics = evaluate_portfolio_at_playhead(
            &self.asset_ohlc_history,
            self.playhead_current,
            &graph,
        );
        self.last_calculated_state = (self.playhead_time, graph_revision);
    }

    pub(crate) fn invalidate_playhead_evaluation_cache(&mut self) {
        self.last_calculated_state = (f64::NAN, u64::MAX);
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
                                            .filter(|node| node.node_type == NodeType::TechnicalAnalysis)
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
                                        workspace.synchronize_inspector_view();
                                        cx.notify();
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
                                        if ohlc_bars.is_empty() {
                                            workspace.asset_ohlc_history.remove(&node_id);
                                        } else {
                                            workspace
                                                .asset_ohlc_history
                                                .insert(node_id, ohlc_bars.clone());
                                            if let Some(node) =
                                                workspace.nodes.iter().find(|node| node.id == node_id)
                                            {
                                                hydrate_market_stage_from_ohlc(
                                                    &mut workspace.market_stage,
                                                    &node.name,
                                                    &ohlc_bars,
                                                );
                                            }
                                        }
                                        workspace.playhead_current = 0;
                                        workspace.sync_playhead_bounds();
                                        workspace.sync_playhead_time_from_index();
                                        workspace.synchronize_inspector_view();
                                        workspace.recompute_playhead_diagnostics();
                                        cx.notify();
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
                        PipelineSystemMessage::PortfolioMetrics {
                            simulation_epoch,
                            tick_index,
                            tick_label,
                            nav,
                            cash,
                            position_qty,
                            mark_price,
                            total_return_pct,
                            max_drawdown_pct,
                            sharpe_ratio,
                            bars_processed,
                            trade_count,
                            benchmark_return_pct,
                            excess_return_pct,
                            avg_exposure_pct,
                        } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        if simulation_epoch < workspace.portfolio_metrics_epoch {
                                            return;
                                        }
                                        let incoming_is_baseline = tick_label
                                            .as_deref()
                                            .is_some_and(|label| label == "baseline");
                                        if incoming_is_baseline {
                                            if let Some(existing) = &workspace.portfolio_diagnostics {
                                                if existing.simulation_epoch == simulation_epoch
                                                    && existing.tick_label.as_deref().is_some_and(
                                                        |label| label.starts_with("epoch closed"),
                                                    )
                                                {
                                                    return;
                                                }
                                            }
                                        }
                                        workspace.portfolio_metrics_epoch = simulation_epoch;
                                        workspace.portfolio_diagnostics =
                                            Some(PortfolioDiagnosticsSnapshot {
                                                simulation_epoch,
                                                tick_index,
                                                tick_label,
                                                nav,
                                                cash,
                                                position_qty,
                                                mark_price,
                                                total_return_pct,
                                                max_drawdown_pct,
                                                sharpe_ratio,
                                                bars_processed,
                                                trade_count,
                                                benchmark_return_pct,
                                                excess_return_pct,
                                                avg_exposure_pct,
                                            });
                                        cx.notify();
                                    });
                                }
                            });
                        }
                        PipelineSystemMessage::ResetSimulation { simulation_epoch } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        if simulation_epoch < workspace.portfolio_metrics_epoch {
                                            return;
                                        }
                                        workspace.portfolio_metrics_epoch = simulation_epoch;
                                        cx.notify();
                                    });
                                }
                            });
                        }
                        PipelineSystemMessage::PlayheadSet {
                            index,
                            total_bars,
                            tick_label: _,
                        } => {
                            let _ = cx.update(|cx| {
                                if let Some(view) = this.upgrade() {
                                    view.update(cx, |workspace, cx| {
                                        workspace.playhead_total_bars = total_bars;
                                        workspace.playhead_current = if total_bars == 0 {
                                            0
                                        } else {
                                            index.min(total_bars - 1)
                                        };
                                        workspace.sync_playhead_time_from_index();
                                        workspace.synchronize_inspector_view();
                                        workspace.recompute_playhead_diagnostics();
                                        cx.notify();
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
                                        workspace.synchronize_inspector_view();
                                        cx.notify();
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
    Ok(CsvAssetPlayback {
        node_id,
        ticker,
        rows,
        cursor: 0,
        current_active_path: path.to_string(),
        reader_paused: false,
    })
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
            playback.cursor = 0;
            playback.current_active_path = new_path.to_string();
            playback.reader_paused = false;
            send_chart_series_preload(tx, playback.node_id, &playback.rows);
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
