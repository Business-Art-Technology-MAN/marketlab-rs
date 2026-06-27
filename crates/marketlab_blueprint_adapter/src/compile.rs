//! Finance graph compile: validate Graphy IR and build [`StageGraphSnapshot`].

use graphy::GraphDescription;

use crate::snapshot::graph_description_to_stage_snapshot;
use crate::types::FinanceNodeKind;
use pulsar_marketlab_core::StageGraphSnapshot;

/// Human-readable result of a finance compile pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceCompileReport {
    pub prim_count: usize,
    pub wire_count: usize,
    pub asset_count: usize,
    pub analytics_count: usize,
    pub portfolio_count: usize,
    pub skipped_node_count: usize,
    pub warnings: Vec<String>,
}

impl FinanceCompileReport {
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Stage prims: {}", self.prim_count),
            format!("Compile wires: {}", self.wire_count),
            format!(
                "Assets: {} · Analytics: {} · Portfolios: {}",
                self.asset_count, self.analytics_count, self.portfolio_count
            ),
        ];
        if self.skipped_node_count > 0 {
            lines.push(format!(
                "Skipped {} non-finance node(s)",
                self.skipped_node_count
            ));
        }
        for warning in &self.warnings {
            lines.push(format!("Warning: {warning}"));
        }
        lines
    }
}

/// Build an engine snapshot and compile report from a Graphy graph.
pub fn compile_finance_graph(
    graph: &GraphDescription,
) -> Result<(StageGraphSnapshot, FinanceCompileReport), String> {
    let finance_node_count = graph
        .nodes
        .values()
        .filter(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type).is_some())
        .count();

    if finance_node_count == 0 {
        return Err(
            "No MarketLab finance nodes in graph — add nodes from Universe / Analytics / Portfolios"
                .to_string(),
        );
    }

    let snapshot = graph_description_to_stage_snapshot(graph);
    let report = build_report(graph, &snapshot);

    if snapshot.prims.is_empty() {
        return Err("Finance compile produced zero stage prims".to_string());
    }

    Ok((snapshot, report))
}

fn json_to_string(value: &graphy::JsonValue) -> Option<String> {
    match value {
        graphy::JsonValue::String(text) => Some(text.clone()),
        graphy::JsonValue::Number(number) => Some(number.to_string()),
        graphy::JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn build_report(graph: &GraphDescription, snapshot: &StageGraphSnapshot) -> FinanceCompileReport {
    let mut asset_count = 0usize;
    let mut analytics_count = 0usize;
    let mut portfolio_count = 0usize;
    let mut warnings = Vec::new();

    for prim in &snapshot.prims {
        match prim.type_name.as_str() {
            "FinancialAsset" | "FinancialReturnAsset" => asset_count += 1,
            "OtlOperator" | "OtlTaUberSignal" => analytics_count += 1,
            "PortfolioIntegrator" => portfolio_count += 1,
            _ => {}
        }
    }

    let skipped_node_count = graph
        .nodes
        .values()
        .filter(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type).is_none())
        .count();

    for node in graph.nodes.values() {
        let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        match kind {
            FinanceNodeKind::FinancialAsset => {
                let csv = node
                    .properties
                    .get("csv_path")
                    .and_then(json_to_string)
                    .unwrap_or_default();
                if csv.is_empty() {
                    warnings.push(format!(
                        "Asset node '{}' has no csv_path — sweep will try crates/pulsar_marketlab/data/{{symbol}}.csv",
                        node.id
                    ));
                }
            }
            FinanceNodeKind::FinancialReturnAsset => {
                let csv = node
                    .properties
                    .get("csv_path")
                    .and_then(json_to_string)
                    .unwrap_or_default();
                if csv.is_empty() {
                    warnings.push(format!(
                        "Return asset '{}' requires csv_path (Date,<Name> simple-return CSV)",
                        node.id
                    ));
                }
            }
            FinanceNodeKind::PortfolioIntegrator => {
                let wired = graph.connections.iter().any(|wire| wire.target_node == node.id);
                if !wired {
                    warnings.push(format!(
                        "Portfolio '{}' has no inbound signal wires",
                        node.id
                    ));
                }
            }
            FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => {
                let wired = graph.connections.iter().any(|wire| wire.target_node == node.id);
                if !wired {
                    warnings.push(format!(
                        "Analytics '{}' has no upstream price wire — TA/OTL will run on zeros until source_stream is connected",
                        node.id
                    ));
                }
                let price_feeds = graph
                    .connections
                    .iter()
                    .filter(|wire| wire.target_node == node.id)
                    .filter(|wire| {
                        graph
                            .nodes
                            .get(&wire.source_node)
                            .and_then(|source| FinanceNodeKind::from_graphy_type_id(&source.node_type))
                            .map(|kind| kind.is_price_source())
                            .unwrap_or(false)
                    })
                    .count();
                if price_feeds > 1 {
                    warnings.push(format!(
                        "Analytics '{}' has {price_feeds} upstream price wires — only one series is used; wire one asset per TA node",
                        node.id
                    ));
                }
            }
            _ => {}
        }
    }

    FinanceCompileReport {
        prim_count: snapshot.prims.len(),
        wire_count: snapshot.wires.len(),
        asset_count,
        analytics_count,
        portfolio_count,
        skipped_node_count,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

    use super::*;
    use crate::types::type_id;

    #[test]
    fn compile_reports_wired_finance_graph() {
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

        let (snapshot, report) = compile_finance_graph(&graph).expect("compile");
        assert_eq!(snapshot.prims.len(), 2);
        assert_eq!(report.wire_count, 1);
        assert_eq!(report.portfolio_count, 1);
    }

    #[test]
    fn compile_rejects_empty_finance_graph() {
        let graph = GraphDescription::new("empty");
        assert!(compile_finance_graph(&graph).is_err());
    }
}
