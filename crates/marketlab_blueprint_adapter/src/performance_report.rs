//! Build Performance Analytics tear-sheet payloads after a finance sweep.

use std::collections::{HashMap, HashSet, VecDeque};

use graphy::GraphDescription;

use crate::asset_data::{load_finance_asset_preview, FinanceAssetPreview};
use crate::blueprint::finance_primary_output_pin;
use crate::performance_analytics::{
    align_wealth_series, compare_to_benchmark, compute_performance_bundle,
    periods_per_year_from_bar_count, FinanceBenchmarkComparison, FinancePerformanceSeriesBundle,
};
use crate::series_pins::{PERFORMANCE_BENCHMARK_PIN, performance_series_pin_id};
use crate::sweep::{FinancePortfolioSweepSummary, FinanceSweepResult};
use crate::types::{type_id, FinanceNodeKind};

#[derive(Clone, Debug, PartialEq)]
pub struct FinancePerformanceReport {
    pub node_id: String,
    pub label: String,
    pub lineage: Vec<String>,
    pub rolling_window: usize,
    pub bundle: FinancePerformanceSeriesBundle,
    pub benchmark: Option<FinanceBenchmarkComparison>,
}

pub struct FinancePerformanceBuildContext<'a> {
    pub graph: &'a GraphDescription,
    pub sweep: &'a FinanceSweepResult,
    pub prim_paths: &'a HashMap<String, String>,
    pub asset_previews: &'a HashMap<String, FinanceAssetPreview>,
    pub portfolio_by_node: &'a HashMap<String, FinancePortfolioSweepSummary>,
    pub analytics_by_node: &'a HashMap<String, Vec<f64>>,
}

pub fn build_finance_performance_reports(
    ctx: &FinancePerformanceBuildContext<'_>,
) -> HashMap<String, FinancePerformanceReport> {
    let mut reports = HashMap::new();
    for (node_id, node) in &ctx.graph.nodes {
        if node.node_type != type_id::PERFORMANCE_ANALYTICS {
            continue;
        }
        let Some(report) = build_report_for_node(node_id, ctx) else {
            continue;
        };
        reports.insert(node_id.clone(), report);
    }
    reports
}

fn build_report_for_node(
    report_node_id: &str,
    ctx: &FinancePerformanceBuildContext<'_>,
) -> Option<FinancePerformanceReport> {
    let node = ctx.graph.nodes.get(report_node_id)?;
    let label = property_string(node, "name").unwrap_or_else(|| "Performance Report".into());
    let risk_free = property_f64(node, "risk_free_rate").unwrap_or(0.0);
    let rolling_window = property_u32(node, "rolling_window").unwrap_or(63) as usize;
    let benchmark_mode =
        property_string(node, "benchmark_mode").unwrap_or_else(|| "auto".into());
    let benchmark_symbol = property_string(node, "benchmark_symbol").unwrap_or_else(|| "SPY".into());

    let primary_source = incoming_series_connection(report_node_id, 0, ctx.graph)?;
    let wealth = resolve_series(
        &primary_source.source_node,
        &primary_source.source_pin,
        ctx,
    )?;
    if wealth.len() < 2 {
        return None;
    }

    let periods_per_year = periods_per_year_from_bar_count(wealth.len());
    let bundle = compute_performance_bundle(&wealth, risk_free, rolling_window, periods_per_year)?;

    let lineage = collect_upstream_lineage(report_node_id, ctx.graph);
    let benchmark = resolve_benchmark(
        report_node_id,
        ctx,
        &wealth,
        &bundle.period_returns_pct,
        &benchmark_mode,
        &benchmark_symbol,
    );

    Some(FinancePerformanceReport {
        node_id: report_node_id.to_string(),
        label,
        lineage,
        rolling_window,
        bundle,
        benchmark,
    })
}

struct SeriesConnection {
    source_node: String,
    source_pin: String,
}

fn incoming_series_connection(
    report_node_id: &str,
    series_index: usize,
    graph: &GraphDescription,
) -> Option<SeriesConnection> {
    let target_pin = performance_series_pin_id(series_index);
    graph.connections.iter().find_map(|connection| {
        if connection.target_node != report_node_id || connection.target_pin != target_pin {
            return None;
        }
        Some(SeriesConnection {
            source_node: connection.source_node.clone(),
            source_pin: connection.source_pin.clone(),
        })
    })
}

fn resolve_series(
    node_id: &str,
    source_pin: &str,
    ctx: &FinancePerformanceBuildContext<'_>,
) -> Option<Vec<f64>> {
    let node = ctx.graph.nodes.get(node_id)?;
    match node.node_type.as_str() {
        type_id::FINANCIAL_ASSET | type_id::FINANCIAL_RETURN_ASSET => {
            let symbol = property_string(node, "symbol").unwrap_or_else(|| "SPY".into());
            let csv = property_string(node, "csv_path");
            let preview = ctx
                .asset_previews
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| {
                    crate::asset_data::load_finance_asset_preview_for_node(
                        &node.node_type,
                        &symbol,
                        csv.as_deref(),
                    )
                });
            let mut series = preview.close_series();
            trim_to_timeline(&mut series, ctx.sweep.timeline_len);
            Some(series)
        }
        type_id::PORTFOLIO_INTEGRATOR => ctx
            .portfolio_by_node
            .get(node_id)
            .map(|summary| summary.wealth_series.clone())
            .or_else(|| {
                ctx.prim_paths.get(node_id).and_then(|path| {
                    ctx.sweep
                        .portfolios
                        .iter()
                        .find(|portfolio| portfolio.prim_path == *path)
                        .map(|portfolio| portfolio.wealth_series.clone())
                })
            }),
        other if FinanceNodeKind::from_graphy_type_id(other).is_some() => ctx
            .analytics_by_node
            .get(node_id)
            .cloned()
            .or_else(|| {
                ctx.prim_paths.get(node_id).and_then(|path| {
                    ctx.sweep.analytics_signals.get(path).cloned()
                })
            }),
        _ => None,
    }
    .map(|mut series| {
        let _ = source_pin;
        trim_to_timeline(&mut series, ctx.sweep.timeline_len);
        series
    })
}

fn resolve_benchmark(
    report_node_id: &str,
    ctx: &FinancePerformanceBuildContext<'_>,
    strategy_wealth: &[f64],
    strategy_returns_pct: &[f64],
    benchmark_mode: &str,
    benchmark_symbol: &str,
) -> Option<FinanceBenchmarkComparison> {
    if let Some(wired) = wired_benchmark_wealth(report_node_id, ctx) {
        let (_strategy, benchmark) = align_wealth_series(strategy_wealth, &wired.1);
        let returns = crate::performance_analytics::wealth_to_period_returns(&benchmark);
        let len = strategy_returns_pct.len().min(returns.len());
        return compare_to_benchmark(
            &strategy_returns_pct[..len],
            &benchmark,
            &wired.0,
        );
    }

    if benchmark_mode == "wired" {
        return None;
    }

    let (label, benchmark_wealth) = if benchmark_mode == "symbol" {
        let preview = load_finance_asset_preview(benchmark_symbol, None);
        (
            format!("Buy & Hold {benchmark_symbol}"),
            preview.close_series(),
        )
    } else {
        auto_buy_and_hold(report_node_id, ctx)?
    };

    let (_strategy, benchmark) = align_wealth_series(strategy_wealth, &benchmark_wealth);
    let returns = crate::performance_analytics::wealth_to_period_returns(&benchmark);
    let len = strategy_returns_pct.len().min(returns.len());
    compare_to_benchmark(&strategy_returns_pct[..len], &benchmark, &label)
}

fn wired_benchmark_wealth(
    report_node_id: &str,
    ctx: &FinancePerformanceBuildContext<'_>,
) -> Option<(String, Vec<f64>)> {
    let connection = ctx.graph.connections.iter().find(|connection| {
        connection.target_node == report_node_id
            && connection.target_pin == PERFORMANCE_BENCHMARK_PIN
    })?;
    let label = ctx
        .graph
        .nodes
        .get(&connection.source_node)
        .and_then(|node| property_string(node, "symbol"))
        .or_else(|| property_string(ctx.graph.nodes.get(&connection.source_node)?, "name"))
        .unwrap_or_else(|| "Benchmark".into());
    let wealth = resolve_series(&connection.source_node, &connection.source_pin, ctx)?;
    Some((label, wealth))
}

fn auto_buy_and_hold(
    report_node_id: &str,
    ctx: &FinancePerformanceBuildContext<'_>,
) -> Option<(String, Vec<f64>)> {
    if let Some(asset_id) = first_upstream_asset(report_node_id, ctx.graph) {
        let node = ctx.graph.nodes.get(&asset_id)?;
        let symbol = property_string(node, "symbol").unwrap_or_else(|| "Asset".into());
        let wealth = resolve_series(&asset_id, "close", ctx)?;
        return Some((format!("Buy & Hold {symbol}"), wealth));
    }

    let assets = upstream_financial_assets(report_node_id, ctx.graph);
    if assets.is_empty() {
        return None;
    }
    let mut blended: Option<Vec<f64>> = None;
    let mut count = 0usize;
    for asset_id in &assets {
        let Some(series) = resolve_series(asset_id, "close", ctx) else {
            continue;
        };
        count += 1;
        blended = Some(match blended {
            None => series,
            Some(existing) => existing
                .iter()
                .zip(series.iter())
                .map(|(left, right)| left + right)
                .collect(),
        });
    }
    blended.map(|values| {
        let divisor = count.max(1) as f64;
        (
            "Equal-weight Buy & Hold".to_string(),
            values.iter().map(|value| value / divisor).collect(),
        )
    })
}

fn collect_upstream_lineage(report_node_id: &str, graph: &GraphDescription) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([report_node_id.to_string()]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if id != report_node_id {
            if let Some(node) = graph.nodes.get(&id) {
                labels.push(lineage_label(node));
            }
        }
        for connection in graph
            .connections
            .iter()
            .filter(|connection| connection.target_node == id)
        {
            queue.push_back(connection.source_node.clone());
        }
    }
    labels
}

fn lineage_label(node: &graphy::NodeInstance) -> String {
    if node.node_type == type_id::FINANCIAL_ASSET || node.node_type == type_id::FINANCIAL_RETURN_ASSET {
        return property_string(node, "symbol").unwrap_or_else(|| "Asset".into());
    }
    if node.node_type == type_id::PORTFOLIO_INTEGRATOR {
        return property_string(node, "name").unwrap_or_else(|| "Portfolio".into());
    }
    if node.node_type == type_id::PERFORMANCE_ANALYTICS {
        return property_string(node, "name").unwrap_or_else(|| "Performance Report".into());
    }
    finance_primary_output_pin(&node.node_type)
        .map(|pin| format!("{} ({pin})", node.node_type.rsplit('.').next().unwrap_or("node")))
        .unwrap_or_else(|| node.node_type.clone())
}

fn first_upstream_asset(report_node_id: &str, graph: &GraphDescription) -> Option<String> {
    upstream_financial_assets(report_node_id, graph).into_iter().next()
}

fn upstream_financial_assets(report_node_id: &str, graph: &GraphDescription) -> Vec<String> {
    let mut assets = Vec::new();
    let mut queue = VecDeque::from([report_node_id.to_string()]);
    let mut seen = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(node) = graph.nodes.get(&id) {
            if node.node_type == type_id::FINANCIAL_ASSET || node.node_type == type_id::FINANCIAL_RETURN_ASSET {
                assets.push(id);
                continue;
            }
        }
        for connection in graph
            .connections
            .iter()
            .filter(|connection| connection.target_node == id)
        {
            queue.push_back(connection.source_node.clone());
        }
    }
    assets.sort();
    assets.dedup();
    assets
}

fn trim_to_timeline(series: &mut Vec<f64>, timeline_len: usize) {
    if timeline_len == 0 {
        return;
    }
    series.truncate(timeline_len);
    if series.len() < timeline_len {
        let last = *series.last().unwrap_or(&0.0);
        series.resize(timeline_len, last);
    }
}

fn property_string(node: &graphy::NodeInstance, key: &str) -> Option<String> {
    node.properties.get(key).and_then(|value| match value {
        graphy::JsonValue::String(text) => Some(text.clone()),
        graphy::JsonValue::Number(number) => Some(number.to_string()),
        graphy::JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

fn property_f64(node: &graphy::NodeInstance, key: &str) -> Option<f64> {
    property_string(node, key)?.parse().ok()
}

fn property_u32(node: &graphy::NodeInstance, key: &str) -> Option<u32> {
    property_string(node, key)?.parse().ok()
}
