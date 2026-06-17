//! Pre-built UI caches produced off the main thread after graph-engine sweeps.

use std::collections::HashMap;
use std::sync::Arc;

use pulsar_marketlab_core::{
    ComputedAttributeStream, ComputedTokenStream, PortfolioIntegrationResult,
    TimelineExecutionResult,
};

use crate::portfolio_integrator_ledger::PortfolioIntegratorLedger;
use crate::portfolio_wealth_chart::{
    PortfolioAllocationChartSeries, PortfolioWealthChartSeries,
};
use crate::workspace_state::PortfolioDiagnosticsSnapshot;

/// Plan alias: double-buffer UI read model after graph-engine sweeps.
pub type TimelineUiSnapshot = GraphUiSnapshot;

/// Immutable UI read model swapped in after a background sweep completes.
#[derive(Clone, Debug, Default)]
pub struct GraphUiSnapshot {
    pub portfolio_timeline_cache: HashMap<String, PortfolioWealthChartSeries>,
    pub portfolio_allocation_cache: HashMap<String, PortfolioAllocationChartSeries>,
    pub portfolio_ledger_cache: HashMap<String, Arc<PortfolioIntegratorLedger>>,
    pub portfolio_diagnostics_cache: HashMap<String, PortfolioDiagnosticsSnapshot>,
    pub graph_engine_portfolio_results: HashMap<String, PortfolioIntegrationResult>,
    pub graph_engine_streams: Vec<ComputedAttributeStream>,
    pub graph_engine_token_streams: Vec<ComputedTokenStream>,
}

/// Build caches off the UI thread from a completed sweep result.
pub fn build_graph_ui_snapshot(
    result: &TimelineExecutionResult,
    build: &GraphUiSnapshotBuildInput,
) -> GraphUiSnapshot {
    use crate::portfolio_integrator_ledger::build_integrator_ledger;
    use crate::portfolio_wealth_chart::{
        build_allocation_chart_from_integration, build_allocation_chart_from_token_streams,
        build_portfolio_wealth_chart_from_streams, build_portfolio_wealth_chart_series,
    };
    use crate::portfolio_analytics::build_portfolio_diagnostics_from_integration;
    use crate::workspace_state::SIM_INITIAL_CASH;

    let bar_labels = build.bar_labels.clone();
    let mut snapshot = GraphUiSnapshot {
        graph_engine_portfolio_results: result.portfolio_results.clone(),
        graph_engine_streams: result.streams.clone(),
        graph_engine_token_streams: result.token_streams.clone(),
        ..Default::default()
    };

    for (prim_path, integration) in &result.portfolio_results {
        snapshot.portfolio_timeline_cache.insert(
            prim_path.clone(),
            build_portfolio_wealth_chart_series(integration, bar_labels.clone()),
        );
        snapshot.portfolio_allocation_cache.insert(
            prim_path.clone(),
            build_allocation_chart_from_integration(integration, bar_labels.clone()),
        );
        snapshot.portfolio_ledger_cache.insert(
            prim_path.clone(),
            Arc::new(build_integrator_ledger(integration, bar_labels.clone())),
        );
    }

    if snapshot.portfolio_timeline_cache.is_empty() {
        for (node_id, prim_path, is_portfolio) in &build.node_prim_paths {
            if !is_portfolio {
                continue;
            }
            if let Some(series) = build_portfolio_wealth_chart_from_streams(
                &result.streams,
                prim_path,
                bar_labels.clone(),
            ) {
                snapshot.portfolio_timeline_cache.insert(prim_path.clone(), series);
            }
            if let Some(allocation) = build_allocation_chart_from_token_streams(
                &result.token_streams,
                prim_path,
                bar_labels.clone(),
                |path| build.prim_labels.get(path).cloned().unwrap_or_else(|| path.to_string()),
            ) {
                snapshot
                    .portfolio_allocation_cache
                    .insert(prim_path.clone(), allocation);
            }
            let _ = node_id;
        }
    }

    let bar_index = build.terminal_bar_index;
    let tick_label = build.terminal_tick_label.clone();
    for (prim_path, integration) in &result.portfolio_results {
        let diagnostics = build_portfolio_diagnostics_from_integration(
            integration,
            bar_index,
            SIM_INITIAL_CASH,
            build.portfolio_metrics_epoch,
            Some(tick_label.clone()),
            build.benchmark_prices.as_deref(),
        );
        snapshot
            .portfolio_diagnostics_cache
            .insert(prim_path.clone(), diagnostics);
    }

    snapshot
}

/// Frozen host inputs for off-thread UI snapshot construction.
#[derive(Clone, Debug)]
pub struct GraphUiSnapshotBuildInput {
    pub bar_labels: Option<Vec<String>>,
    pub terminal_bar_index: usize,
    pub terminal_tick_label: String,
    pub portfolio_metrics_epoch: u64,
    pub benchmark_prices: Option<Vec<f64>>,
    pub node_prim_paths: Vec<(usize, String, bool)>,
    pub prim_labels: HashMap<String, String>,
}
