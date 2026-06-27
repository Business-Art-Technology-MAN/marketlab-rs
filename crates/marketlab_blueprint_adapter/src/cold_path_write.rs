//! Cold-path USD write pipeline: review full graph, refresh asset tokens, rebuild stage, verify.

use std::collections::HashSet;

use graphy::{GraphDescription, JsonValue};
use pulsar_marketlab_core::StageGraphSnapshot;

use crate::compile::{compile_finance_graph, FinanceCompileReport};
use crate::snapshot::finance_node_prim_paths;
use crate::taxonomy_index::{finance_asset_properties_for_symbol, FinanceDatabaseIndex};
use crate::types::FinanceNodeKind;
use crate::usd_persistence::{import_document, UsdPersistenceError};

/// Result of a validated cold-path export (compile + optional round-trip reopen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceColdWriteReport {
    pub compile: FinanceCompileReport,
    pub assets_refreshed: usize,
    pub round_trip_nodes: usize,
    pub round_trip_connections: usize,
}

impl FinanceColdWriteReport {
    pub fn summary_line(&self) -> String {
        format!(
            "{} prims · {} wires · {} assets ({} DB-refreshed) · reopen {} nodes / {} wires",
            self.compile.prim_count,
            self.compile.wire_count,
            self.compile.asset_count,
            self.assets_refreshed,
            self.round_trip_nodes,
            self.round_trip_connections,
        )
    }
}

/// Refresh financial-asset node properties from taxonomy + project database before export.
pub fn prepare_finance_graph_for_cold_write(
    graph: &mut GraphDescription,
    database: Option<&FinanceDatabaseIndex>,
) -> usize {
    let mut refreshed = 0usize;
    for node in graph.nodes.values_mut() {
        let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        if kind != FinanceNodeKind::FinancialAsset {
            continue;
        }
        let Some(symbol) = node
            .properties
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let had_database_row = database
            .and_then(|db| db.get(symbol))
            .is_some();
        let autofill = finance_asset_properties_for_symbol(symbol, database);
        for (key, value) in autofill {
            if key == "csv_path" {
                continue;
            }
            node.properties
                .insert(key, JsonValue::String(value));
        }
        if had_database_row {
            refreshed += 1;
        }
    }
    refreshed
}

/// Full-graph review before disk write: compile snapshot and enforce structural invariants.
pub fn validate_finance_graph_for_cold_write(
    graph: &GraphDescription,
) -> Result<(StageGraphSnapshot, FinanceCompileReport), UsdPersistenceError> {
    let finance_node_count = graph
        .nodes
        .values()
        .filter(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type).is_some())
        .count();

    let (snapshot, report) =
        compile_finance_graph(graph).map_err(UsdPersistenceError::Hydrate)?;

    if snapshot.prims.len() != finance_node_count {
        return Err(UsdPersistenceError::Hydrate(format!(
            "stage snapshot has {} prims but graph has {} finance nodes",
            snapshot.prims.len(),
            finance_node_count
        )));
    }

    let paths = finance_node_prim_paths(graph);
    if paths.len() != finance_node_count {
        return Err(UsdPersistenceError::Hydrate(
            "one or more finance nodes lack a resolved USD prim path".into(),
        ));
    }
    let unique_paths: HashSet<&String> = paths.values().collect();
    if unique_paths.len() != paths.len() {
        return Err(UsdPersistenceError::Hydrate(
            "two or more finance nodes resolve to the same USD prim path".into(),
        ));
    }

    let mut asset_symbols = HashSet::new();
    for node in graph.nodes.values() {
        let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        if kind != FinanceNodeKind::FinancialAsset {
            continue;
        }
        let symbol = node
            .properties
            .get("symbol")
            .and_then(|value| value.as_str())
            .unwrap_or(&node.id)
            .trim()
            .to_ascii_uppercase();
        if !asset_symbols.insert(symbol.clone()) {
            return Err(UsdPersistenceError::Hydrate(format!(
                "duplicate financial asset symbol '{symbol}' in graph"
            )));
        }
    }

    let prim_paths: HashSet<&str> = snapshot.prims.iter().map(|prim| prim.path.as_str()).collect();
    for wire in &snapshot.wires {
        if !prim_paths.contains(wire.source_prim_path.as_str()) {
            return Err(UsdPersistenceError::Hydrate(format!(
                "compile wire references missing source prim {}",
                wire.source_prim_path
            )));
        }
        if !prim_paths.contains(wire.target_prim_path.as_str()) {
            return Err(UsdPersistenceError::Hydrate(format!(
                "compile wire references missing target prim {}",
                wire.target_prim_path
            )));
        }
    }

    let finance_connections = graph
        .connections
        .iter()
        .filter(|wire| {
            graph
                .nodes
                .get(&wire.source_node)
                .and_then(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type))
                .is_some()
                && graph
                    .nodes
                    .get(&wire.target_node)
                    .and_then(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type))
                    .is_some()
        })
        .count();
    if finance_connections > 0 && snapshot.wires.is_empty() {
        return Err(UsdPersistenceError::Hydrate(
            "graph has finance connections but stage snapshot produced zero lineage wires".into(),
        ));
    }

    Ok((snapshot, report))
}

/// Reopen a freshly written USDA file and compare hydrated topology to expectations.
pub fn verify_cold_write_round_trip(
    path: &std::path::Path,
    expected_finance_nodes: usize,
) -> Result<(usize, usize), UsdPersistenceError> {
    let reopened = import_document(path)?;
    let node_count = reopened.graph.nodes.len();
    let connection_count = reopened.graph.connections.len();
    if node_count != expected_finance_nodes {
        return Err(UsdPersistenceError::Hydrate(format!(
            "round-trip reopen loaded {node_count} nodes, expected {expected_finance_nodes}"
        )));
    }
    Ok((node_count, connection_count))
}

#[cfg(test)]
mod tests {
    use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

    use super::*;
    use crate::types::type_id;

    #[test]
    fn prepare_refreshes_asset_taxonomy_tokens() {
        let mut graph = GraphDescription::new("test");
        let mut node = NodeInstance::new(
            "aapl",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        );
        node.properties.insert(
            "symbol".to_string(),
            graphy::JsonValue::String("AAPL".to_string()),
        );
        graph.add_node(node);

        prepare_finance_graph_for_cold_write(&mut graph, None);
        let node = graph.nodes.get("aapl").expect("node");
        assert_eq!(
            node.properties.get("category").and_then(|v| v.as_str()),
            Some("Information Technology")
        );
    }

    #[test]
    fn validate_rejects_duplicate_asset_symbols() {
        let mut graph = GraphDescription::new("test");
        for id in ["spy_a", "spy_b"] {
            let mut node = NodeInstance::new(
                id,
                type_id::FINANCIAL_ASSET,
                Position::new(0.0, 0.0),
            );
            node.properties.insert(
                "symbol".to_string(),
                graphy::JsonValue::String("SPY".to_string()),
            );
            graph.add_node(node);
        }
        assert!(validate_finance_graph_for_cold_write(&graph).is_err());
    }

    #[test]
    fn validate_accepts_wired_risk_parity_graph() {
        let mut graph = GraphDescription::new("risk_parity");
        for (index, symbol) in ["spy", "vea", "ief", "tlt"].iter().enumerate() {
            let mut node = NodeInstance::new(
                *symbol,
                type_id::FINANCIAL_ASSET,
                Position::new(index as f64 * 100.0, 0.0),
            );
            node.properties.insert(
                "symbol".to_string(),
                graphy::JsonValue::String(symbol.to_ascii_uppercase()),
            );
            graph.add_node(node);
        }
        for (id, y) in [("equities", 100.0), ("rates", 200.0), ("final", 300.0)] {
            let mut node = NodeInstance::new(
                id,
                type_id::PORTFOLIO_INTEGRATOR,
                Position::new(400.0, y),
            );
            node.properties.insert(
                "name".to_string(),
                graphy::JsonValue::String(id.to_string()),
            );
            graph.add_node(node);
        }
        for (portfolio, sources) in [
            ("equities", vec!["spy", "vea"]),
            ("rates", vec!["ief", "tlt"]),
            ("final", vec!["equities", "rates"]),
        ] {
            for (index, source) in sources.iter().enumerate() {
                graph.add_connection(Connection {
                    source_node: (*source).to_string(),
                    source_pin: "close".to_string(),
                    target_node: portfolio.to_string(),
                    target_pin: format!("signal_{index}"),
                    connection_type: ConnectionType::Data,
                });
            }
        }

        let (snapshot, report) =
            validate_finance_graph_for_cold_write(&graph).expect("validate");
        assert_eq!(snapshot.prims.len(), 7);
        assert_eq!(report.asset_count, 4);
        assert_eq!(report.portfolio_count, 3);
        assert!(!snapshot.wires.is_empty());
    }
}
