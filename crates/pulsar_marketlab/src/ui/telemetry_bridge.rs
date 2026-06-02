//! Centralized graph-engine metrics cache for canvas cards and inspector panels.

use std::collections::HashMap;

use gpui::{App, Global, UpdateGlobal};
use pulsar_marketlab_core::{
    DirectionalDistribution, PortfolioIntegrationResult, SymbolicOtlClosure,
};

use crate::graph_compiler::VisualNode;
use crate::workspace_state::PortfolioDiagnosticsSnapshot;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvaluatedMetrics {
    pub total_return: f64,
    pub rolling_drawdown: f64,
    pub net_exposure: f64,
    pub current_conviction: f64,
    pub trailing_trades_count: usize,
}

#[derive(Default)]
pub struct MetricsTelemetryBridge {
    pub node_metrics: HashMap<usize, EvaluatedMetrics>,
    pub global_metrics: EvaluatedMetrics,
}

impl Global for MetricsTelemetryBridge {}

impl MetricsTelemetryBridge {
    /// Ingest computed runtime closure states to hydrate the active view cache.
    pub fn push_timeline_frame(
        &mut self,
        node_id: usize,
        closures: &[SymbolicOtlClosure],
        performance_map: &HashMap<String, f64>,
    ) {
        let metrics = self.node_metrics.entry(node_id).or_default();

        if let Some(latest) = closures.last() {
            metrics.current_conviction = latest.closure_raw_weight;
            if let Some(signal) = latest.signal_series.last() {
                metrics.current_conviction = signal.abs();
            }
        }

        if let Some(&exposure) = performance_map.get("net_exposure") {
            metrics.net_exposure = exposure;
        } else if let Some(latest) = closures.last() {
            metrics.net_exposure = latest.closure_raw_weight.abs();
        }

        if let Some(&r_total) = performance_map.get("total_return") {
            metrics.total_return = r_total;
        }
        if let Some(&dd) = performance_map.get("drawdown") {
            metrics.rolling_drawdown = dd;
        }
        if let Some(&trades) = performance_map.get("trades_count") {
            metrics.trailing_trades_count = trades as usize;
        }
    }

    pub fn metrics_for_node(&self, node_id: usize) -> Option<&EvaluatedMetrics> {
        self.node_metrics.get(&node_id)
    }

    pub fn reset(&mut self) {
        self.node_metrics.clear();
        self.global_metrics = EvaluatedMetrics::default();
    }

    pub fn sync_from_graph_engine(
        &mut self,
        nodes: &[VisualNode],
        portfolio_results: &HashMap<String, PortfolioIntegrationResult>,
        diagnostics: &HashMap<String, PortfolioDiagnosticsSnapshot>,
        playhead: usize,
        resolve_prim_path: impl Fn(&VisualNode) -> Option<String>,
        selected_node_id: Option<usize>,
    ) {
        self.node_metrics.clear();

        for node in nodes {
            if !node.node_type.is_portfolio() {
                continue;
            }
            let Some(prim_path) = resolve_prim_path(node) else {
                continue;
            };
            let integration = portfolio_results.get(&prim_path);
            let closures = integration
                .map(|result| closures_at_playhead(result, playhead))
                .unwrap_or_default();
            let performance_map = diagnostics
                .get(&prim_path)
                .map(performance_map_from_diagnostics)
                .unwrap_or_default();
            self.push_timeline_frame(node.id, &closures, &performance_map);
        }

        self.global_metrics = selected_node_id
            .and_then(|node_id| self.node_metrics.get(&node_id).cloned())
            .or_else(|| self.node_metrics.values().next().cloned())
            .unwrap_or_default();
    }
}

impl From<&PortfolioDiagnosticsSnapshot> for EvaluatedMetrics {
    fn from(snapshot: &PortfolioDiagnosticsSnapshot) -> Self {
        Self {
            total_return: snapshot.total_return_pct,
            rolling_drawdown: snapshot.max_drawdown_pct,
            net_exposure: snapshot.avg_exposure_pct,
            current_conviction: 0.0,
            trailing_trades_count: snapshot.trade_count as usize,
        }
    }
}

pub fn publish_metrics_telemetry(
    cx: &mut App,
    nodes: &[VisualNode],
    portfolio_results: &HashMap<String, PortfolioIntegrationResult>,
    diagnostics: &HashMap<String, PortfolioDiagnosticsSnapshot>,
    playhead: usize,
    resolve_prim_path: impl Fn(&VisualNode) -> Option<String>,
    selected_node_id: Option<usize>,
) {
    MetricsTelemetryBridge::update_global(cx, |bridge, _| {
        bridge.sync_from_graph_engine(
            nodes,
            portfolio_results,
            diagnostics,
            playhead,
            resolve_prim_path,
            selected_node_id,
        );
    });
}

fn performance_map_from_diagnostics(
    snapshot: &PortfolioDiagnosticsSnapshot,
) -> HashMap<String, f64> {
    HashMap::from([
        ("total_return".to_string(), snapshot.total_return_pct),
        ("drawdown".to_string(), snapshot.max_drawdown_pct),
        ("net_exposure".to_string(), snapshot.avg_exposure_pct),
        ("trades_count".to_string(), snapshot.trade_count as f64),
    ])
}

fn closures_at_playhead(
    integration: &PortfolioIntegrationResult,
    playhead: usize,
) -> Vec<SymbolicOtlClosure> {
    integration
        .tracking_matrix
        .iter()
        .filter(|frame| frame.timestamp as usize == playhead)
        .map(|frame| SymbolicOtlClosure {
            asset_id: frame.asset_id.clone(),
            direction: if frame.altered_portfolio_weight >= 0.0 {
                DirectionalDistribution::MarketLong
            } else {
                DirectionalDistribution::MarketShort
            },
            closure_raw_weight: frame.closure_raw_weight,
            signal_series: vec![frame.altered_portfolio_weight],
            leg_kind: Default::default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulsar_marketlab_core::PortfolioTrackingFrame;

    #[test]
    fn push_timeline_frame_hydrates_metrics() {
        let mut bridge = MetricsTelemetryBridge::default();
        let closures = vec![SymbolicOtlClosure {
            asset_id: "SPY".into(),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 0.75,
            signal_series: vec![0.75],
            leg_kind: Default::default(),
        }];
        let performance = HashMap::from([
            ("total_return".to_string(), 0.12),
            ("drawdown".to_string(), 0.03),
            ("trades_count".to_string(), 4.0),
            ("net_exposure".to_string(), 0.95),
        ]);
        bridge.push_timeline_frame(7, &closures, &performance);
        let metrics = bridge.metrics_for_node(7).expect("node metrics");
        assert!((metrics.total_return - 0.12).abs() < f64::EPSILON);
        assert!((metrics.current_conviction - 0.75).abs() < f64::EPSILON);
        assert_eq!(metrics.trailing_trades_count, 4);
    }

    #[test]
    fn closures_at_playhead_reads_tracking_matrix() {
        let integration = PortfolioIntegrationResult {
            wealth_series: vec![10_000.0, 10_100.0],
            tracking_matrix: vec![PortfolioTrackingFrame {
                timestamp: 1,
                asset_id: "QQQ".into(),
                closure_raw_weight: 0.5,
                altered_portfolio_weight: 0.5,
                current_nominal_price: 400.0,
                calculated_units: 12.0,
                investment_return: 0.01,
            }],
        };
        let closures = closures_at_playhead(&integration, 1);
        assert_eq!(closures.len(), 1);
        assert!((closures[0].closure_raw_weight - 0.5).abs() < f64::EPSILON);
    }
}
