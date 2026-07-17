//! Sketch: Graphy [`GraphDescription`] → [`StageGraphSnapshot`] for engine sweeps.

use std::collections::{HashMap, HashSet};

use graphy::{GraphDescription, JsonValue, NodeInstance};
use pulsar_marketlab_core::{
    compose_uber_script_src, GraphCompileWire, StageGraphPrim, StageGraphSnapshot,
    TaArchetype, TaUberSignalConfig, USER_LABEL_ATTR,
};

use crate::provider::FinanceNodeMetadataProvider;
use crate::types::{FinanceNodeKind, PORTFOLIO_ALLOCATION_TOKENS};

const LINEAGE_UNDERLYING: &str = "inputs:underlying";
const LINEAGE_SOURCES: &str = "inputs:sources";
const LINEAGE_SERIES: &str = "inputs:series";
const LINEAGE_BENCHMARK: &str = "inputs:benchmark";

/// Converts a Graphy graph into an engine snapshot (no live USD stage).
pub fn graph_description_to_stage_snapshot(
    graph: &GraphDescription,
) -> StageGraphSnapshot {
    let paths = resolve_prim_paths(graph);
    let mut prims = Vec::with_capacity(graph.nodes.len());

    for node in graph.nodes.values() {
        let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        let Some(path) = paths.get(&node.id) else {
            continue;
        };
        prims.push(StageGraphPrim {
            path: path.clone(),
            type_name: kind.stage_schema_type().to_string(),
            attributes: prim_attributes(node, kind, path),
        });
    }

    let wires = lineage_wires(graph, &paths);
    let path_bindings = path_binding_index(&prims);
    let asset_registry = asset_registry_from_prims(&prims);

    StageGraphSnapshot {
        prims,
        wires,
        path_bindings,
        asset_registry,
    }
}

/// Strip terminal reporting prims before handing a snapshot to [`MarketLabGraphEngine`].
///
/// Performance Analytics nodes are post-sweep sinks: they stay in the authored snapshot for
/// USD/stage-tree wiring but must not participate in timeline execution.
pub fn snapshot_for_engine_execution(snapshot: &StageGraphSnapshot) -> StageGraphSnapshot {
    let reporting_paths: std::collections::HashSet<String> = snapshot
        .prims
        .iter()
        .filter(|prim| prim.type_name == "PerformanceAnalytics")
        .map(|prim| prim.path.clone())
        .collect();

    if reporting_paths.is_empty() {
        return snapshot.clone();
    }

    let prims = snapshot
        .prims
        .iter()
        .filter(|prim| prim.type_name != "PerformanceAnalytics")
        .cloned()
        .collect();
    let wires = snapshot
        .wires
        .iter()
        .filter(|wire| {
            !reporting_paths.contains(&wire.source_prim_path)
                && !reporting_paths.contains(&wire.target_prim_path)
        })
        .cloned()
        .collect();

    StageGraphSnapshot {
        prims,
        wires,
        path_bindings: snapshot.path_bindings.clone(),
        asset_registry: snapshot.asset_registry.clone(),
    }
}

/// Graph node id → USD prim path for finance nodes (used to map sweep output back to canvas nodes).
pub fn finance_node_prim_paths(graph: &GraphDescription) -> HashMap<String, String> {
    resolve_prim_paths(graph)
}

fn resolve_prim_paths(graph: &GraphDescription) -> HashMap<String, String> {
    let mut paths = HashMap::new();
    let mut path_claims: HashMap<String, String> = HashMap::new();

    for (id, node) in &graph.nodes {
        let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        let path = match kind {
            FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => {
                universe_asset_prim_path(id, node, kind, &mut path_claims)
            }
            FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => {
                claim_prim_path(id, format!("/MarketLab/Analytics/{id}"), &mut path_claims)
            }
            FinanceNodeKind::PortfolioIntegrator => {
                portfolio_prim_path(id, node, &mut path_claims)
            }
            FinanceNodeKind::PerformanceAnalytics => {
                reporting_prim_path(id, node, &mut path_claims)
            }
        };
        paths.insert(id.clone(), path);
    }
    paths
}

/// Reserve a unique absolute prim path; suffix with the graph node id when names collide.
fn claim_prim_path(
    node_id: &str,
    mut candidate: String,
    claims: &mut HashMap<String, String>,
) -> String {
    loop {
        match claims.get(&candidate) {
            Some(owner) if owner == node_id => return candidate,
            Some(_) => {
                candidate = format!("{candidate}_{}", sanitize_leaf(node_id));
            }
            None => {
                claims.insert(candidate.clone(), node_id.to_string());
                return candidate;
            }
        }
    }
}

fn universe_asset_prim_path(
    node_id: &str,
    node: &NodeInstance,
    kind: FinanceNodeKind,
    claims: &mut HashMap<String, String>,
) -> String {
    let derived = {
        let symbol = property_string(node, "symbol").unwrap_or_else(|| node_id.to_string());
        let leaf = if kind == FinanceNodeKind::FinancialReturnAsset {
            sanitize_leaf(&symbol)
        } else {
            symbol.to_ascii_uppercase()
        };
        format!("/MarketLab/Universe/{leaf}")
    };
    let preferred = property_string(node, "prim_path").unwrap_or(derived);
    claim_prim_path(node_id, preferred, claims)
}

fn reporting_prim_path(
    node_id: &str,
    node: &NodeInstance,
    claims: &mut HashMap<String, String>,
) -> String {
    let leaf = property_string(node, "name").unwrap_or_else(|| "report".into());
    let derived = format!("/MarketLab/Reporting/{}", sanitize_leaf(&leaf));
    let preferred = property_string(node, "prim_path").unwrap_or(derived);
    claim_prim_path(node_id, preferred, claims)
}

/// Each portfolio node needs a distinct prim path. Multiple nodes named "Fund" must not
/// collapse to `/MarketLab/Portfolios/fund` (that creates self-loop wires and cycle errors).
fn portfolio_prim_path(
    node_id: &str,
    node: &NodeInstance,
    claims: &mut HashMap<String, String>,
) -> String {
    let leaf = property_string(node, "name").unwrap_or_else(|| "fund".into());
    let derived = format!("/MarketLab/Portfolios/{}", sanitize_leaf(&leaf));
    let preferred = property_string(node, "prim_path").unwrap_or(derived);
    claim_prim_path(node_id, preferred, claims)
}

fn prim_attributes(
    node: &NodeInstance,
    kind: FinanceNodeKind,
    prim_path: &str,
) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let mut insert = |key: &str, value: String| {
        attrs.insert(key.to_string(), value);
    };

    match kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => {
            let symbol = property_string(node, "symbol").unwrap_or_else(|| {
                prim_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(prim_path)
                    .to_string()
            });
            insert("inputs:active", "true".to_string());
            insert("inputs:symbol", symbol);
            insert(
                "inputs:asset_class",
                property_string(node, "asset_class").unwrap_or_else(|| {
                    if kind == FinanceNodeKind::FinancialReturnAsset {
                        "Alternative".into()
                    } else {
                        "Equity".into()
                    }
                }),
            );
            if let Some(csv) = property_string(node, "csv_path") {
                insert("inputs:csv_path", csv);
            }
            if kind == FinanceNodeKind::FinancialAsset {
                if let Some(category) = property_string(node, "category") {
                    insert("inputs:category", category);
                }
                if let Some(sub_category) = property_string(node, "sub_category") {
                    insert("inputs:sub_category", sub_category);
                }
                if let Some(mic) = property_string(node, "exchange_mic") {
                    insert("inputs:exchange_mic", mic);
                }
                if let Some(provider) = property_string(node, "provider") {
                    insert("inputs:provider", provider);
                }
                insert_info_attributes(&mut insert, node);
            }
        }
        FinanceNodeKind::OtlOperator => {
            if let Some(script) = property_string(node, "script_src") {
                insert("inputs:script_src", script);
            }
            if let Some(path) = property_string(node, "script_compiled_path") {
                insert("inputs:script_compiled_path", path);
            }
        }
        FinanceNodeKind::OtlTaUberSignal => {
            let archetype = FinanceNodeKind::ta_archetype_token(&node.node_type)
                .and_then(TaArchetype::from_token)
                .unwrap_or(TaArchetype::Trend);
            let config = TaUberSignalConfig {
                archetype,
                algorithm: property_string(node, "algorithm")
                    .unwrap_or_else(|| archetype.default_algorithm().to_string()),
                period: property_u32(node, "period").unwrap_or_else(|| archetype.default_period()),
                signal_period: property_u32(node, "signal_period")
                    .unwrap_or_else(|| archetype.default_signal_period()),
                multiplier: property_f64(node, "multiplier").unwrap_or(2.0) as f32,
                annualization: property_f64(node, "annualization").unwrap_or(252.0) as f32,
            };
            insert(
                "info:archetype",
                config.archetype.as_token().to_string(),
            );
            insert("info:algorithm", config.algorithm.clone());
            insert("inputs:period", config.period.to_string());
            insert("inputs:signal_period", config.signal_period.to_string());
            insert("inputs:multiplier", config.multiplier.to_string());
            insert("inputs:annualization", config.annualization.to_string());
            insert("inputs:script_src", compose_uber_script_src(&config));
        }
        FinanceNodeKind::PortfolioIntegrator => {
            let allocation = property_string(node, "allocation_id").unwrap_or_else(|| {
                PORTFOLIO_ALLOCATION_TOKENS[0].to_string()
            });
            insert("inputs:id", allocation);
            insert(
                "inputs:initial_capital",
                property_f64(node, "initial_capital")
                    .unwrap_or(10_000_000.0)
                    .to_string(),
            );
            insert(
                "inputs:rebalance_frequency",
                property_string(node, "rebalance_frequency").unwrap_or_else(|| "monthly".into()),
            );
        }
        FinanceNodeKind::PerformanceAnalytics => {
            insert(
                "inputs:name",
                property_string(node, "name").unwrap_or_else(|| "Performance Report".into()),
            );
            insert(
                "inputs:risk_free_rate",
                property_f64(node, "risk_free_rate")
                    .unwrap_or(0.0)
                    .to_string(),
            );
            insert(
                "inputs:rolling_window",
                property_u32(node, "rolling_window")
                    .unwrap_or(63)
                    .to_string(),
            );
            insert(
                "inputs:benchmark_mode",
                property_string(node, "benchmark_mode").unwrap_or_else(|| "auto".into()),
            );
            insert(
                "inputs:benchmark_symbol",
                property_string(node, "benchmark_symbol").unwrap_or_else(|| "SPY".into()),
            );
        }
    }

    if let Some(label) = property_string(node, "display_name") {
        insert(USER_LABEL_ATTR, label);
    }

    attrs
}

fn dedupe_compile_wires(wires: &mut Vec<GraphCompileWire>) {
    let mut seen = HashSet::new();
    wires.retain(|wire| {
        seen.insert((
            wire.source_prim_path.clone(),
            wire.target_prim_path.clone(),
            wire.relationship.clone(),
        ))
    });
}

fn lineage_wires(
    graph: &GraphDescription,
    paths: &HashMap<String, String>,
) -> Vec<GraphCompileWire> {
    let node_kind = |id: &str| -> Option<FinanceNodeKind> {
        graph
            .nodes
            .get(id)
            .and_then(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type))
    };

    let mut wires = Vec::new();
    for connection in &graph.connections {
        let Some(source_path) = paths.get(&connection.source_node) else {
            continue;
        };
        let Some(target_path) = paths.get(&connection.target_node) else {
            continue;
        };
        let Some(source_kind) = node_kind(&connection.source_node) else {
            continue;
        };
        let Some(target_kind) = node_kind(&connection.target_node) else {
            continue;
        };

        let relationship = match (source_kind, target_kind) {
            (source, FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal)
                if source.is_series_source() && source != FinanceNodeKind::OtlTaUberSignal && source != FinanceNodeKind::OtlOperator => {
                LINEAGE_UNDERLYING
            }
            (
                FinanceNodeKind::FinancialAsset
                    | FinanceNodeKind::FinancialReturnAsset
                    | FinanceNodeKind::OtlOperator
                    | FinanceNodeKind::OtlTaUberSignal
                    | FinanceNodeKind::PortfolioIntegrator,
                FinanceNodeKind::PortfolioIntegrator,
            ) => LINEAGE_SOURCES,
            (_, FinanceNodeKind::PerformanceAnalytics) => {
                if connection.target_pin == crate::series_pins::PERFORMANCE_BENCHMARK_PIN {
                    LINEAGE_BENCHMARK
                } else {
                    LINEAGE_SERIES
                }
            }
            (FinanceNodeKind::OtlOperator, FinanceNodeKind::OtlOperator)
            | (FinanceNodeKind::OtlOperator, FinanceNodeKind::OtlTaUberSignal)
            | (FinanceNodeKind::OtlTaUberSignal, FinanceNodeKind::OtlOperator)
            | (FinanceNodeKind::OtlTaUberSignal, FinanceNodeKind::OtlTaUberSignal) => {
                LINEAGE_UNDERLYING
            }
            _ => continue,
        };

        wires.push(GraphCompileWire {
            source_prim_path: source_path.clone(),
            target_prim_path: target_path.clone(),
            relationship: relationship.to_string(),
        });
    }
    dedupe_compile_wires(&mut wires);
    wires
}

fn path_binding_index(prims: &[StageGraphPrim]) -> pulsar_marketlab_core::PathBindingIndex {
    let mut ordered_prim_paths = Vec::new();
    let mut asset_slots = HashMap::new();
    for prim in prims {
        if !crate::types::is_finance_price_asset_stage_type(&prim.type_name) {
            continue;
        }
        let slot = ordered_prim_paths.len();
        asset_slots.insert(prim.path.clone(), slot);
        ordered_prim_paths.push(prim.path.clone());
    }
    pulsar_marketlab_core::PathBindingIndex {
        asset_slots,
        ordered_prim_paths,
    }
}

fn asset_registry_from_prims(
    prims: &[StageGraphPrim],
) -> HashMap<String, pulsar_marketlab_core::ComposedAssetMeta> {
    let mut registry = HashMap::new();
    for prim in prims {
        if !crate::types::is_finance_price_asset_stage_type(&prim.type_name) {
            continue;
        }
        let symbol = prim
            .attributes
            .get("inputs:symbol")
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                prim.path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&prim.path)
                    .to_string()
            });
        let asset_class = prim
            .attributes
            .get("inputs:asset_class")
            .cloned()
            .unwrap_or_else(|| "Equity".to_string());
        registry.insert(
            prim.path.clone(),
            pulsar_marketlab_core::ComposedAssetMeta {
                symbol,
                asset_class,
                category: prim.attributes.get("inputs:category").cloned().unwrap_or_default(),
                sub_category: prim
                    .attributes
                    .get("inputs:sub_category")
                    .cloned()
                    .unwrap_or_default(),
                is_active: true,
                sector: String::new(),
                industry: String::new(),
                market_cap_class: String::new(),
                currency: String::new(),
                country: String::new(),
                user_label: prim.attributes.get(USER_LABEL_ATTR).cloned().unwrap_or_default(),
            },
        );
    }
    registry
}

fn property_string(node: &NodeInstance, key: &str) -> Option<String> {
    node.properties
        .get(key)
        .and_then(json_to_string)
        .filter(|value| !value.is_empty())
}

fn insert_info_attributes(
    insert: &mut impl FnMut(&str, String),
    node: &NodeInstance,
) {
    const INFO_FIELDS: [(&str, &str); 8] = [
        ("info_sector", "info:sector"),
        ("info_industry_group", "info:industry_group"),
        ("info_industry", "info:industry"),
        ("info_currency", "info:currency"),
        ("info_country", "info:country"),
        ("info_state", "info:state"),
        ("info_zipcode", "info:zipcode"),
        ("info_market_cap_class", "info:market_cap_class"),
    ];
    for (prop_key, usd_key) in INFO_FIELDS {
        if let Some(value) = property_string(node, prop_key) {
            insert(usd_key, value);
        }
    }
}

fn property_u32(node: &NodeInstance, key: &str) -> Option<u32> {
    node.properties.get(key).and_then(|value| match value {
        JsonValue::Number(number) => number.as_u64().map(|n| n as u32),
        JsonValue::String(text) => text.parse().ok(),
        _ => None,
    })
}

fn property_f64(node: &NodeInstance, key: &str) -> Option<f64> {
    node.properties.get(key).and_then(|value| match value {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.parse().ok(),
        _ => None,
    })
}

fn json_to_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn sanitize_leaf(leaf: &str) -> String {
    leaf.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Adapter entry point used by future WGPUI host wiring.
#[derive(Clone, Debug, Default)]
pub struct FinanceGraphAdapter {
    pub metadata: FinanceNodeMetadataProvider,
}

impl FinanceGraphAdapter {
    pub fn new() -> Self {
        Self {
            metadata: FinanceNodeMetadataProvider::new(),
        }
    }

    pub fn to_stage_graph_snapshot(&self, graph: &GraphDescription) -> StageGraphSnapshot {
        let _ = &self.metadata;
        graph_description_to_stage_snapshot(graph)
    }
}

#[cfg(test)]
mod tests {
    use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

    use super::*;
    use crate::types::type_id;

    #[test]
    fn snapshot_from_asset_to_portfolio_wire() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert_eq!(snapshot.prims.len(), 2);
        assert_eq!(snapshot.wires.len(), 1);
        assert_eq!(snapshot.wires[0].relationship, LINEAGE_SOURCES);
    }

    #[test]
    fn lineage_wires_dedupe_identical_portfolio_sources() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        let mut stream = NodeInstance::new(
            "stream_1",
            type_id::FINANCIAL_RETURN_ASSET,
            Position::new(0.0, 0.0),
        );
        stream
            .properties
            .insert("symbol".into(), JsonValue::String("stream_1".into()));
        graph.add_node(stream);
        graph.add_node(NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(100.0, 0.0),
        ));
        for pin in ["signal_0", "signal_1"] {
            graph.add_connection(Connection {
                source_node: "stream_1".into(),
                source_pin: "close".into(),
                target_node: "ear_fund".into(),
                target_pin: pin.into(),
                connection_type: ConnectionType::Data,
            });
        }

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert_eq!(
            snapshot
                .wires
                .iter()
                .filter(|wire| wire.relationship == LINEAGE_SOURCES)
                .count(),
            1,
            "identical portfolio lineage wires should dedupe before USD export"
        );
    }

    #[test]
    fn portfolio_prim_paths_are_unique_when_fund_names_match() {
        let mut graph = GraphDescription::new("test");
        for id in ["sub_a", "sub_b", "master"] {
            graph.add_node(NodeInstance::new(
                id,
                type_id::PORTFOLIO_INTEGRATOR,
                Position::new(0.0, 0.0),
            ));
        }
        let paths = finance_node_prim_paths(&graph);
        let unique: std::collections::HashSet<_> = paths.values().cloned().collect();
        assert_eq!(unique.len(), 3);
        assert!(unique.iter().any(|path| path == "/MarketLab/Portfolios/fund"));
    }

    #[test]
    fn prim_paths_disambiguate_colliding_reporting_and_return_assets() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        for id in ["report_a", "report_b"] {
            let mut node = NodeInstance::new(
                id,
                type_id::PERFORMANCE_ANALYTICS,
                Position::new(0.0, 0.0),
            );
            node.properties.insert(
                "name".into(),
                JsonValue::String("Performance Report".into()),
            );
            graph.add_node(node);
        }
        for id in ["stream_x", "stream_y"] {
            let mut node = NodeInstance::new(
                id,
                type_id::FINANCIAL_RETURN_ASSET,
                Position::new(0.0, 0.0),
            );
            node.properties.insert("symbol".into(), JsonValue::String("stream_1".into()));
            node.properties.insert(
                "prim_path".into(),
                JsonValue::String("/MarketLab/Universe/stream_1".into()),
            );
            graph.add_node(node);
        }

        let paths = finance_node_prim_paths(&graph);
        let unique: std::collections::HashSet<_> = paths.values().cloned().collect();
        assert_eq!(unique.len(), 4, "colliding names must get distinct prim paths: {paths:?}");
    }

    #[test]
    fn lineage_wires_portfolio_wealth_into_ta_as_underlying() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        let mut ear_fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(0.0, 0.0),
        );
        ear_fund
            .properties
            .insert("name".into(), JsonValue::String("ear_fund".into()));
        graph.add_node(ear_fund);
        graph.add_node(NodeInstance::new(
            "ta_master",
            type_id::TA_TREND,
            Position::new(100.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "ear_fund".into(),
            source_pin: "wealth".into(),
            target_node: "ta_master".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert_eq!(snapshot.wires.len(), 1);
        assert_eq!(snapshot.wires[0].relationship, LINEAGE_UNDERLYING);
        assert!(snapshot.wires[0].source_prim_path.contains("ear_fund"));
        assert!(snapshot.wires[0].target_prim_path.contains("ta_master"));
    }

    #[test]
    fn hierarchical_portfolios_compile_without_dependency_cycle() {
        use pulsar_marketlab_core::MarketLabGraphEngine;

        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "asset_0",
            type_id::FINANCIAL_RETURN_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "sub_a",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "master",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(400.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "asset_0".into(),
            source_pin: "close".into(),
            target_node: "sub_a".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "sub_a".into(),
            source_pin: "close".into(),
            target_node: "master".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let engine_snapshot = snapshot_for_engine_execution(&snapshot);
        MarketLabGraphEngine::compile_from_stage(&engine_snapshot)
            .expect("hierarchical portfolios should not form a cycle");
    }

    #[test]
    fn engine_execution_snapshot_drops_reporting_prims() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "report",
            type_id::PERFORMANCE_ANALYTICS,
            Position::new(200.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "report".into(),
            target_pin: "series_0".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert_eq!(snapshot.prims.len(), 2);

        let execution = snapshot_for_engine_execution(&snapshot);
        assert_eq!(execution.prims.len(), 1);
        assert_eq!(execution.prims[0].type_name, "FinancialAsset");
        assert!(execution.wires.is_empty());

        pulsar_marketlab_core::MarketLabGraphEngine::compile_from_stage(&execution)
            .expect("reporting prim should not block engine compile");
    }
}
