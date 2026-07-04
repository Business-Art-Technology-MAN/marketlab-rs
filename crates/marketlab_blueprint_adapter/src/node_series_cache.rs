//! Node-indexed series cache for EventGraph sparklines and Hydra inspection.

use std::collections::HashMap;

use graphy::GraphDescription;
use pulsar_marketlab_core::StageGraphSnapshot;

use crate::asset_data::{load_finance_asset_preview_for_node, FinanceAssetPreview};
use crate::blueprint::{
    finance_is_analytics_node, finance_primary_output_pin, FINANCE_STREAM_INPUT_PINS,
};
use crate::sweep::{FinancePortfolioSweepSummary, FinanceSweepResult};
use crate::types::{type_id, FinanceNodeKind};

/// Semantic kind for finance timeline series visualization.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FinanceSeriesKind {
    Price,
    Indicator,
    Gate,
    Wealth,
}

/// Compact stats for Details / Hydra scrub readouts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeValueSummary {
    pub min: f64,
    pub max: f64,
    pub last: f64,
    pub long_pct: Option<f64>,
    pub flat_pct: Option<f64>,
    pub short_pct: Option<f64>,
}

/// Per-node input + output series bundle after a finance sweep.
#[derive(Clone, Debug, PartialEq)]
pub struct FinanceNodeSeriesBundle {
    pub node_id: String,
    pub prim_path: String,
    pub node_kind: FinanceNodeKind,
    pub outputs: HashMap<String, Vec<f64>>,
    pub inputs: HashMap<String, Vec<f64>>,
    pub series_kinds: HashMap<String, FinanceSeriesKind>,
    pub primary_output: Option<String>,
    pub primary_series: Vec<f64>,
    pub primary_kind: FinanceSeriesKind,
    pub summary: NodeValueSummary,
}

/// Resolved upstream series for Hydra compare / gate modes.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFinanceSeries {
    pub node_id: String,
    pub pin: String,
    pub label: String,
    pub series: Vec<f64>,
    pub kind: FinanceSeriesKind,
}

pub struct FinanceNodeSeriesBuildContext<'a> {
    pub graph: &'a GraphDescription,
    pub sweep: &'a FinanceSweepResult,
    pub snapshot: &'a StageGraphSnapshot,
    pub prim_paths: &'a HashMap<String, String>,
    pub asset_previews: &'a HashMap<String, FinanceAssetPreview>,
    pub portfolio_by_node: &'a HashMap<String, FinancePortfolioSweepSummary>,
}

pub struct FinanceSeriesResolveContext<'a> {
    pub graph: &'a GraphDescription,
    pub sweep: &'a FinanceSweepResult,
    pub prim_paths: &'a HashMap<String, String>,
    pub asset_previews: &'a HashMap<String, FinanceAssetPreview>,
    pub portfolio_by_node: &'a HashMap<String, FinancePortfolioSweepSummary>,
    pub node_series: &'a HashMap<String, FinanceNodeSeriesBundle>,
}

const ANALYTICS_OUTPUT_PRIORITY: &[&str] = &[
    "outputs:result",
    "outputs:signal",
    "outputs:portfolio_wealth",
];

pub fn build_finance_node_series_cache(
    ctx: &FinanceNodeSeriesBuildContext<'_>,
) -> HashMap<String, FinanceNodeSeriesBundle> {
    let mut cache = HashMap::new();
    for (node_id, node) in &ctx.graph.nodes {
        let Some(node_kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        let prim_path = ctx
            .prim_paths
            .get(node_id)
            .cloned()
            .unwrap_or_default();
        let mut bundle = FinanceNodeSeriesBundle {
            node_id: node_id.clone(),
            prim_path: prim_path.clone(),
            node_kind,
            outputs: HashMap::new(),
            inputs: HashMap::new(),
            series_kinds: HashMap::new(),
            primary_output: None,
            primary_series: Vec::new(),
            primary_kind: FinanceSeriesKind::Indicator,
            summary: NodeValueSummary::default(),
        };

        populate_outputs(&mut bundle, ctx);
        populate_inputs(&mut bundle, ctx, &cache);
        select_primary_series(&mut bundle);
        bundle.summary = summarize_series(&bundle.primary_series, bundle.primary_kind);
        cache.insert(node_id.clone(), bundle);
    }

    // Second pass: inputs may reference upstream bundles built in the same sweep.
    for (node_id, bundle) in cache.clone().iter() {
        if bundle.inputs.is_empty() {
            let mut updated = bundle.clone();
            populate_inputs(&mut updated, ctx, &cache);
            if let Some(entry) = cache.get_mut(node_id) {
                *entry = updated;
                entry.summary = summarize_series(&entry.primary_series, entry.primary_kind);
            }
        }
    }
    cache
}

pub fn resolve_upstream_series(
    node_id: &str,
    ctx: &FinanceSeriesResolveContext<'_>,
) -> Option<ResolvedFinanceSeries> {
    let connection = primary_inbound_stream_connection(node_id, ctx.graph)?;
    let source_id = connection.source_node.clone();
    let source_pin = connection.source_pin.clone();
    let series = resolve_node_output_series(&source_id, &source_pin, ctx)?;
    let label = node_label(ctx.graph, &source_id);
    let kind = ctx
        .node_series
        .get(&source_id)
        .map(|bundle| bundle.primary_kind)
        .unwrap_or_else(|| infer_series_kind_for_node(ctx.graph, &source_id, &series));
    Some(ResolvedFinanceSeries {
        node_id: source_id,
        pin: source_pin,
        label,
        series,
        kind,
    })
}

pub fn value_at_bar(series: &[f64], bar_index: usize) -> Option<f64> {
    series.get(bar_index).copied()
}

pub fn bundle_scrub_readout(
    bundle: &FinanceNodeSeriesBundle,
    bar_index: usize,
) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Some(value) = value_at_bar(&bundle.primary_series, bar_index) {
        rows.push((
            "Output".to_string(),
            format_series_value(value, bundle.primary_kind),
        ));
    }
    for (pin, series) in &bundle.inputs {
        let kind = bundle
            .series_kinds
            .get(&format!("input:{pin}"))
            .copied()
            .unwrap_or(FinanceSeriesKind::Indicator);
        if let Some(value) = value_at_bar(series, bar_index) {
            rows.push((
                format!("Input · {pin}"),
                format_series_value(value, kind),
            ));
        }
    }
    rows
}

fn populate_outputs(bundle: &mut FinanceNodeSeriesBundle, ctx: &FinanceNodeSeriesBuildContext<'_>) {
    match bundle.node_kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => {
            let preview = ctx.asset_previews.get(&bundle.node_id).cloned().unwrap_or_else(|| {
                let node = ctx.graph.nodes.get(&bundle.node_id).expect("node");
                let symbol = property_string(node, "symbol").unwrap_or_else(|| "SPY".into());
                let csv = property_string(node, "csv_path");
                load_finance_asset_preview_for_node(&node.node_type, &symbol, csv.as_deref())
            });
            let mut series = preview.close_series();
            trim_to_timeline(&mut series, ctx.sweep.timeline_len);
            bundle.outputs.insert("close".to_string(), series);
            bundle
                .series_kinds
                .insert("close".to_string(), FinanceSeriesKind::Price);
        }
        FinanceNodeKind::PortfolioIntegrator => {
            if let Some(summary) = ctx.portfolio_by_node.get(&bundle.node_id) {
                let mut series = summary.wealth_series.clone();
                trim_to_timeline(&mut series, ctx.sweep.timeline_len);
                bundle.outputs.insert("wealth".to_string(), series);
                bundle
                    .series_kinds
                    .insert("wealth".to_string(), FinanceSeriesKind::Wealth);
            }
        }
        FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => {
            if let Some(attrs) = ctx.sweep.attribute_streams.get(&bundle.prim_path) {
                for (attribute, values) in attrs {
                    let pin = analytics_attribute_to_pin(attribute);
                    let mut series = values.clone();
                    trim_to_timeline(&mut series, ctx.sweep.timeline_len);
                    let kind = classify_series_kind(bundle.node_kind, &pin, &series);
                    bundle.series_kinds.insert(pin.clone(), kind);
                    bundle.outputs.insert(pin, series);
                }
            }
            if bundle.outputs.is_empty() {
                if let Some(series) = ctx.sweep.analytics_signals.get(&bundle.prim_path) {
                    let mut values = series.clone();
                    trim_to_timeline(&mut values, ctx.sweep.timeline_len);
                    let kind = classify_series_kind(bundle.node_kind, "result", &values);
                    bundle.outputs.insert("result".to_string(), values);
                    bundle.series_kinds.insert("result".to_string(), kind);
                }
            }
        }
        FinanceNodeKind::PerformanceAnalytics => {
            if let Some(summary) = resolve_wired_primary_wealth(&bundle.node_id, ctx) {
                let mut series = summary;
                trim_to_timeline(&mut series, ctx.sweep.timeline_len);
                bundle.outputs.insert("series".to_string(), series);
                bundle
                    .series_kinds
                    .insert("series".to_string(), FinanceSeriesKind::Wealth);
            }
        }
    }
}

fn populate_inputs(
    bundle: &mut FinanceNodeSeriesBundle,
    ctx: &FinanceNodeSeriesBuildContext<'_>,
    cache: &HashMap<String, FinanceNodeSeriesBundle>,
) {
    let Some(node_type) = ctx
        .graph
        .nodes
        .get(&bundle.node_id)
        .map(|node| node.node_type.as_str())
    else {
        return;
    };
    if !finance_is_analytics_node(node_type) {
        return;
    }

    let resolve_ctx = FinanceSeriesResolveContext {
        graph: ctx.graph,
        sweep: ctx.sweep,
        prim_paths: ctx.prim_paths,
        asset_previews: ctx.asset_previews,
        portfolio_by_node: ctx.portfolio_by_node,
        node_series: cache,
    };
    let Some(connection) = primary_inbound_stream_connection(&bundle.node_id, ctx.graph) else {
        return;
    };
    if let Some(upstream) = resolve_upstream_series(&bundle.node_id, &resolve_ctx) {
        bundle
            .inputs
            .insert(connection.target_pin.clone(), upstream.series.clone());
        bundle.series_kinds.insert(
            format!("input:{}", connection.target_pin),
            upstream.kind,
        );
    }
}

fn select_primary_series(bundle: &mut FinanceNodeSeriesBundle) {
    let node_type = bundle.node_kind.graphy_type_id();
    if let Some(pin) = finance_primary_output_pin(node_type) {
        if let Some(series) = bundle.outputs.get(pin) {
            bundle.primary_output = Some(pin.to_string());
            bundle.primary_series = series.clone();
            bundle.primary_kind = bundle
                .series_kinds
                .get(pin)
                .copied()
                .unwrap_or_else(|| classify_series_kind(bundle.node_kind, pin, series));
            return;
        }
    }

    for attribute in ANALYTICS_OUTPUT_PRIORITY {
        let pin = analytics_attribute_to_pin(attribute);
        if let Some(series) = bundle.outputs.get(&pin) {
            bundle.primary_kind = bundle
                .series_kinds
                .get(&pin)
                .copied()
                .unwrap_or_else(|| classify_series_kind(bundle.node_kind, &pin, series));
            bundle.primary_output = Some(pin);
            bundle.primary_series = series.clone();
            return;
        }
    }

    if let Some((pin, series)) = bundle.outputs.iter().next() {
        bundle.primary_output = Some(pin.clone());
        bundle.primary_series = series.clone();
        bundle.primary_kind = bundle
            .series_kinds
            .get(pin)
            .copied()
            .unwrap_or_else(|| classify_series_kind(bundle.node_kind, pin, series));
    }
}

pub fn classify_series_kind(
    node_kind: FinanceNodeKind,
    pin: &str,
    values: &[f64],
) -> FinanceSeriesKind {
    match node_kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => {
            FinanceSeriesKind::Price
        }
        FinanceNodeKind::PortfolioIntegrator | FinanceNodeKind::PerformanceAnalytics => {
            FinanceSeriesKind::Wealth
        }
        FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => {
            if pin.contains("wealth") {
                FinanceSeriesKind::Wealth
            } else if looks_like_gate_series(values) {
                FinanceSeriesKind::Gate
            } else {
                FinanceSeriesKind::Indicator
            }
        }
    }
}

fn looks_like_gate_series(values: &[f64]) -> bool {
    if values.is_empty() {
        return false;
    }
    let mut gate_like = 0usize;
    for value in values {
        if value.abs() <= f64::EPSILON
            || (*value - 1.0).abs() <= 0.05
            || (*value + 1.0).abs() <= 0.05
        {
            gate_like += 1;
        }
    }
    gate_like * 100 / values.len() >= 85
}

pub fn summarize_series(values: &[f64], kind: FinanceSeriesKind) -> NodeValueSummary {
    if values.is_empty() {
        return NodeValueSummary::default();
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let last = *values.last().unwrap_or(&0.0);
    let (long_pct, flat_pct, short_pct) = if kind == FinanceSeriesKind::Gate {
        let total = values.len().max(1) as f64;
        let long = values.iter().filter(|value| **value > 0.25).count() as f64;
        let short = values.iter().filter(|value| **value < -0.25).count() as f64;
        let flat = total - long - short;
        (
            Some(long * 100.0 / total),
            Some(flat * 100.0 / total),
            Some(short * 100.0 / total),
        )
    } else {
        (None, None, None)
    };
    NodeValueSummary {
        min,
        max,
        last,
        long_pct,
        flat_pct,
        short_pct,
    }
}

fn resolve_node_output_series(
    node_id: &str,
    source_pin: &str,
    ctx: &FinanceSeriesResolveContext<'_>,
) -> Option<Vec<f64>> {
    if let Some(bundle) = ctx.node_series.get(node_id) {
        if let Some(series) = bundle.outputs.get(source_pin) {
            return Some(series.clone());
        }
        if !bundle.primary_series.is_empty() {
            return Some(bundle.primary_series.clone());
        }
    }

    let node = ctx.graph.nodes.get(node_id)?;
    let mut series = match node.node_type.as_str() {
        type_id::FINANCIAL_ASSET | type_id::FINANCIAL_RETURN_ASSET => {
            let symbol = property_string(node, "symbol").unwrap_or_else(|| "SPY".into());
            let csv = property_string(node, "csv_path");
            let preview = ctx
                .asset_previews
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| {
                    load_finance_asset_preview_for_node(&node.node_type, &symbol, csv.as_deref())
                });
            preview.close_series()
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
            })?,
        other if FinanceNodeKind::from_graphy_type_id(other).is_some() => ctx
            .prim_paths
            .get(node_id)
            .and_then(|path| ctx.sweep.analytics_signals.get(path).cloned())
            .or_else(|| ctx.sweep.analytics_signals.values().next().cloned())
            .unwrap_or_default(),
        _ => return None,
    };
    trim_to_timeline(&mut series, ctx.sweep.timeline_len);
    Some(series)
}

fn primary_inbound_stream_connection(
    node_id: &str,
    graph: &GraphDescription,
) -> Option<graphy::Connection> {
    let mut inbound: Vec<_> = graph
        .connections
        .iter()
        .filter(|connection| connection.target_node == node_id)
        .cloned()
        .collect();
    inbound.sort_by(|left, right| left.target_pin.cmp(&right.target_pin));
    inbound
        .into_iter()
        .find(|connection| FINANCE_STREAM_INPUT_PINS.contains(&connection.target_pin.as_str()))
        .or_else(|| {
            graph
                .connections
                .iter()
                .find(|connection| connection.target_node == node_id)
                .cloned()
        })
}

fn resolve_wired_primary_wealth(
    node_id: &str,
    ctx: &FinanceNodeSeriesBuildContext<'_>,
) -> Option<Vec<f64>> {
    let connection = ctx
        .graph
        .connections
        .iter()
        .find(|connection| connection.target_node == node_id && connection.target_pin == "series")?;
    let resolve_ctx = FinanceSeriesResolveContext {
        graph: ctx.graph,
        sweep: ctx.sweep,
        prim_paths: ctx.prim_paths,
        asset_previews: ctx.asset_previews,
        portfolio_by_node: ctx.portfolio_by_node,
        node_series: &HashMap::new(),
    };
    resolve_node_output_series(&connection.source_node, &connection.source_pin, &resolve_ctx)
}

fn infer_series_kind_for_node(
    graph: &GraphDescription,
    node_id: &str,
    series: &[f64],
) -> FinanceSeriesKind {
    let Some(node) = graph.nodes.get(node_id) else {
        return FinanceSeriesKind::Indicator;
    };
    let node_kind = FinanceNodeKind::from_graphy_type_id(&node.node_type)
        .unwrap_or(FinanceNodeKind::OtlTaUberSignal);
    classify_series_kind(node_kind, "series", series)
}

fn analytics_attribute_to_pin(attribute: &str) -> String {
    attribute
        .strip_prefix("outputs:")
        .unwrap_or(attribute)
        .to_string()
}

fn node_label(graph: &GraphDescription, node_id: &str) -> String {
    graph
        .nodes
        .get(node_id)
        .and_then(|node| property_string(node, "name"))
        .or_else(|| property_string(graph.nodes.get(node_id)?, "symbol"))
        .unwrap_or_else(|| node_id.to_string())
}

fn property_string(node: &graphy::NodeInstance, key: &str) -> Option<String> {
    node.properties.get(key).and_then(|value| match value {
        graphy::JsonValue::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        graphy::JsonValue::Number(number) => Some(number.to_string()),
        graphy::JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

fn trim_to_timeline(series: &mut Vec<f64>, timeline_len: usize) {
    if timeline_len == 0 {
        series.clear();
        return;
    }
    if series.len() > timeline_len {
        series.truncate(timeline_len);
    }
}

fn format_series_value(value: f64, kind: FinanceSeriesKind) -> String {
    match kind {
        FinanceSeriesKind::Gate => {
            if value > 0.25 {
                "Long".to_string()
            } else if value < -0.25 {
                "Short".to_string()
            } else {
                "Flat".to_string()
            }
        }
        FinanceSeriesKind::Wealth => format!("${value:.2}"),
        FinanceSeriesKind::Price => format!("{value:.4}"),
        FinanceSeriesKind::Indicator => format!("{value:.4}"),
    }
}

#[cfg(test)]
mod tests {
    use graphy::{Connection, ConnectionType, NodeInstance, Position};

    use super::*;
    use crate::snapshot::graph_description_to_stage_snapshot;
    use crate::types::type_id;

    fn build_ctx<'a>(
        graph: &'a GraphDescription,
        sweep: &'a FinanceSweepResult,
        snapshot: &'a StageGraphSnapshot,
        paths: &'a HashMap<String, String>,
        previews: &'a HashMap<String, FinanceAssetPreview>,
        portfolios: &'a HashMap<String, FinancePortfolioSweepSummary>,
    ) -> FinanceNodeSeriesBuildContext<'a> {
        FinanceNodeSeriesBuildContext {
            graph,
            sweep,
            snapshot,
            prim_paths: paths,
            asset_previews: previews,
            portfolio_by_node: portfolios,
        }
    }

    #[test]
    fn cache_includes_ta_gate_series() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend",
            type_id::TA_TREND,
            Position::new(120.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let paths = crate::finance_node_prim_paths(&graph);
        let sweep = crate::run_finance_sweep(&snapshot);
        let previews = crate::finance_asset_previews_for_snapshot(&snapshot, &paths);
        let empty_portfolios = HashMap::new();
        let ctx = build_ctx(
            &graph,
            &sweep,
            &snapshot,
            &paths,
            &previews,
            &empty_portfolios,
        );
        let cache = build_finance_node_series_cache(&ctx);
        let bundle = cache.get("trend").expect("ta bundle");
        assert!(!bundle.primary_series.is_empty());
        assert_eq!(bundle.primary_kind, FinanceSeriesKind::Gate);
        assert!(bundle.inputs.contains_key("source_stream"));
    }

    #[test]
    fn resolve_upstream_wealth_for_portfolio_fed_ta() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(120.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend",
            type_id::TA_TREND,
            Position::new(240.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "fund".into(),
            source_pin: "wealth".into(),
            target_node: "trend".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let paths = crate::finance_node_prim_paths(&graph);
        let sweep = crate::run_finance_sweep(&snapshot);
        let previews = crate::finance_asset_previews_for_snapshot(&snapshot, &paths);
        let mut portfolios = HashMap::new();
        for portfolio in &sweep.portfolios {
            for (node_id, prim_path) in &paths {
                if prim_path == &portfolio.prim_path {
                    portfolios.insert(node_id.clone(), portfolio.clone());
                }
            }
        }
        let ctx = build_ctx(
            &graph,
            &sweep,
            &snapshot,
            &paths,
            &previews,
            &portfolios,
        );
        let cache = build_finance_node_series_cache(&ctx);
        let resolve_ctx = FinanceSeriesResolveContext {
            graph: &graph,
            sweep: &sweep,
            prim_paths: &paths,
            asset_previews: &previews,
            portfolio_by_node: &portfolios,
            node_series: &cache,
        };
        let upstream = resolve_upstream_series("trend", &resolve_ctx).expect("wealth upstream");
        assert_eq!(upstream.kind, FinanceSeriesKind::Wealth);
        assert!(upstream.series.len() >= 2);
    }
}
