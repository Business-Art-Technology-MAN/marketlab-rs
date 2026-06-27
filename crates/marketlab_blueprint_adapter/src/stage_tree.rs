//! Finance stage tree model — Graphy topology → hierarchical rows for the Hydra Stage Tree panel.

use std::collections::{HashMap, HashSet};

use graphy::{GraphDescription, NodeInstance};

use crate::snapshot::finance_node_prim_paths;
use crate::types::{category, FinanceNodeKind};

/// One row in the Hydra stage tree (folder or finance prim).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceStageTreeRow {
    pub row_index: usize,
    pub node_id: Option<String>,
    pub label: String,
    pub type_label: String,
    pub prim_path: Option<String>,
    pub children: Vec<usize>,
    pub is_scope_folder: bool,
}

/// Built stage tree for UI binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceStageTreeModel {
    pub rows: Vec<FinanceStageTreeRow>,
    pub root_ids: Vec<usize>,
}

const SCOPES: [(&str, &str); 3] = [
    (category::UNIVERSE, "Universe"),
    (category::ANALYTICS, "Analytics"),
    (category::PORTFOLIOS, "Portfolios"),
];

/// Build a hierarchical stage tree from the live Graphy graph.
///
/// Scope folders mirror `/MarketLab/{Universe,Analytics,Portfolios}`; portfolio nodes
/// nest wired upstream prims as children (lineage from compile wires).
pub fn build_finance_stage_tree(graph: &GraphDescription) -> FinanceStageTreeModel {
    let paths = finance_node_prim_paths(graph);
    let mut rows: Vec<FinanceStageTreeRow> = Vec::new();
    let mut scope_folder_index: HashMap<&'static str, usize> = HashMap::new();

    for (scope_key, scope_label) in SCOPES {
        let index = rows.len();
        rows.push(FinanceStageTreeRow {
            row_index: index,
            node_id: None,
            label: scope_label.to_string(),
            type_label: "Scope".to_string(),
            prim_path: Some(format!("/MarketLab/{scope_key}")),
            children: Vec::new(),
            is_scope_folder: true,
        });
        scope_folder_index.insert(scope_key, index);
    }

    let root_ids: Vec<usize> = (0..SCOPES.len()).collect();
    let mut node_row_by_id: HashMap<String, usize> = HashMap::new();
    let mut portfolio_node_ids: Vec<String> = Vec::new();

    for (node_id, node) in &graph.nodes {
        let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
            continue;
        };
        let Some(prim_path) = paths.get(node_id).cloned() else {
            continue;
        };

        let scope_key = scope_for_kind(kind);
        let row_index = rows.len();
        rows.push(FinanceStageTreeRow {
            row_index,
            node_id: Some(node_id.clone()),
            label: node_display_label(node, kind, &prim_path),
            type_label: type_display_label(kind),
            prim_path: Some(prim_path),
            children: Vec::new(),
            is_scope_folder: false,
        });
        node_row_by_id.insert(node_id.clone(), row_index);

        if let Some(folder_index) = scope_folder_index.get(scope_key).copied() {
            rows[folder_index].children.push(row_index);
        }

        if kind == FinanceNodeKind::PortfolioIntegrator {
            portfolio_node_ids.push(node_id.clone());
        }
    }

    // Nest upstream lineage under each portfolio (asset → analytics → portfolio chains).
    for portfolio_id in &portfolio_node_ids {
        let Some(&portfolio_row) = node_row_by_id.get(portfolio_id) else {
            continue;
        };
        let mut visiting = HashSet::new();
        for upstream_id in upstream_node_ids(graph, portfolio_id) {
            attach_upstream_lineage(
                graph,
                &paths,
                &mut rows,
                &node_row_by_id,
                &scope_folder_index,
                portfolio_row,
                &upstream_id,
                &mut visiting,
            );
        }
    }

    // Stable ordering within each parent.
    let labels: Vec<String> = rows.iter().map(|row| row.label.clone()).collect();
    for row in &mut rows {
        row.children.sort_by_key(|child| labels[*child].to_ascii_lowercase());
    }

    let _ = portfolio_node_ids;

    FinanceStageTreeModel { rows, root_ids }
}

fn upstream_node_ids(graph: &GraphDescription, node_id: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    graph
        .connections
        .iter()
        .filter(|connection| connection.target_node == node_id)
        .map(|connection| connection.source_node.clone())
        .filter(|source_id| seen.insert(source_id.clone()))
        .collect()
}

fn asset_ref_child_index(
    rows: &[FinanceStageTreeRow],
    parent_row: usize,
    node_id: &str,
) -> Option<usize> {
    rows.get(parent_row).and_then(|parent| {
        parent.children.iter().copied().find(|&child| {
            rows.get(child).is_some_and(|row| {
                row.type_label == "AssetRef" && row.node_id.as_deref() == Some(node_id)
            })
        })
    })
}

fn attach_asset_ref_under_parent(
    rows: &mut Vec<FinanceStageTreeRow>,
    parent_row: usize,
    node: &NodeInstance,
    kind: FinanceNodeKind,
    node_id: &str,
    prim_path: Option<String>,
) {
    if asset_ref_child_index(rows, parent_row, node_id).is_some() {
        return;
    }
    let ref_row = push_asset_ref_row(rows, node, kind, node_id, prim_path);
    push_child_unique(&mut rows[parent_row].children, ref_row);
}

fn push_child_unique(children: &mut Vec<usize>, child: usize) {
    if !children.contains(&child) {
        children.push(child);
    }
}

fn detach_from_scope_folder(
    rows: &mut [FinanceStageTreeRow],
    scope_folder_index: &HashMap<&'static str, usize>,
    row_index: usize,
    kind: FinanceNodeKind,
) {
    if let Some(folder_index) = scope_folder_index.get(scope_for_kind(kind)).copied() {
        rows[folder_index].children.retain(|child| *child != row_index);
    }
}

fn push_asset_ref_row(
    rows: &mut Vec<FinanceStageTreeRow>,
    node: &NodeInstance,
    kind: FinanceNodeKind,
    node_id: &str,
    prim_path: Option<String>,
) -> usize {
    let ref_row = rows.len();
    rows.push(FinanceStageTreeRow {
        row_index: ref_row,
        node_id: Some(node_id.to_string()),
        label: node_display_label(node, kind, prim_path.as_deref().unwrap_or("")),
        type_label: "AssetRef".to_string(),
        prim_path,
        children: Vec::new(),
        is_scope_folder: false,
    });
    ref_row
}

fn attach_upstream_lineage(
    graph: &GraphDescription,
    paths: &HashMap<String, String>,
    rows: &mut Vec<FinanceStageTreeRow>,
    node_row_by_id: &HashMap<String, usize>,
    scope_folder_index: &HashMap<&'static str, usize>,
    parent_row: usize,
    upstream_id: &str,
    visiting: &mut HashSet<String>,
) {
    if parent_row >= rows.len() || !visiting.insert(upstream_id.to_string()) {
        return;
    }
    let Some(node) = graph.nodes.get(upstream_id) else {
        visiting.remove(upstream_id);
        return;
    };
    let Some(kind) = FinanceNodeKind::from_graphy_type_id(&node.node_type) else {
        visiting.remove(upstream_id);
        return;
    };

    match kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => {
            attach_asset_ref_under_parent(
                rows,
                parent_row,
                node,
                kind,
                upstream_id,
                paths.get(upstream_id).cloned(),
            );
        }
        FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => {
            let Some(analytics_row) = node_row_by_id.get(upstream_id).copied() else {
                visiting.remove(upstream_id);
                return;
            };
            if analytics_row == parent_row {
                visiting.remove(upstream_id);
                return;
            }
            detach_from_scope_folder(rows, scope_folder_index, analytics_row, kind);
            push_child_unique(&mut rows[parent_row].children, analytics_row);
            for asset_id in upstream_node_ids(graph, upstream_id) {
                let Some(asset_node) = graph.nodes.get(&asset_id) else {
                    continue;
                };
                let Some(asset_kind) = FinanceNodeKind::from_graphy_type_id(&asset_node.node_type)
                else {
                    continue;
                };
                if !asset_kind.is_price_source() {
                    continue;
                }
                attach_asset_ref_under_parent(
                    rows,
                    analytics_row,
                    asset_node,
                    asset_kind,
                    &asset_id,
                    paths.get(&asset_id).cloned(),
                );
            }
            for nested_id in upstream_node_ids(graph, upstream_id) {
                let Some(nested_kind) = graph
                    .nodes
                    .get(&nested_id)
                    .and_then(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type))
                else {
                    continue;
                };
                match nested_kind {
                    FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => {
                        attach_upstream_lineage(
                            graph,
                            paths,
                            rows,
                            node_row_by_id,
                            scope_folder_index,
                            analytics_row,
                            &nested_id,
                            visiting,
                        );
                    }
                    FinanceNodeKind::PortfolioIntegrator => {
                        attach_upstream_lineage(
                            graph,
                            paths,
                            rows,
                            node_row_by_id,
                            scope_folder_index,
                            analytics_row,
                            &nested_id,
                            visiting,
                        );
                    }
                    _ => {}
                }
            }
        }
        FinanceNodeKind::PortfolioIntegrator => {
            let Some(sub_row) = node_row_by_id.get(upstream_id).copied() else {
                visiting.remove(upstream_id);
                return;
            };
            if sub_row == parent_row {
                visiting.remove(upstream_id);
                return;
            }
            detach_from_scope_folder(rows, scope_folder_index, sub_row, kind);
            push_child_unique(&mut rows[parent_row].children, sub_row);
            for nested_id in upstream_node_ids(graph, upstream_id) {
                attach_upstream_lineage(
                    graph,
                    paths,
                    rows,
                    node_row_by_id,
                    scope_folder_index,
                    sub_row,
                    &nested_id,
                    visiting,
                );
            }
        }
        FinanceNodeKind::PerformanceAnalytics => {}
    }

    visiting.remove(upstream_id);
}

fn scope_for_kind(kind: FinanceNodeKind) -> &'static str {
    match kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => category::UNIVERSE,
        FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => category::ANALYTICS,
        FinanceNodeKind::PortfolioIntegrator => category::PORTFOLIOS,
        FinanceNodeKind::PerformanceAnalytics => category::REPORTING,
    }
}

fn type_display_label(kind: FinanceNodeKind) -> String {
    match kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => "AssetRef".to_string(),
        FinanceNodeKind::OtlOperator => "OtlOperator".to_string(),
        FinanceNodeKind::OtlTaUberSignal => "SignalTransform".to_string(),
        FinanceNodeKind::PortfolioIntegrator => "Portfolio".to_string(),
        FinanceNodeKind::PerformanceAnalytics => "PerformanceAnalytics".to_string(),
    }
}

fn node_display_label(node: &NodeInstance, kind: FinanceNodeKind, prim_path: &str) -> String {
    if let Some(label) = property_string(node, "display_name") {
        if !label.is_empty() {
            return label;
        }
    }
    match kind {
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => property_string(node, "symbol").unwrap_or_else(|| {
            prim_path
                .rsplit('/')
                .next()
                .unwrap_or(prim_path)
                .to_string()
        }),
        FinanceNodeKind::PortfolioIntegrator => {
            property_string(node, "name").unwrap_or_else(|| "Portfolio".to_string())
        }
        FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => property_string(
            node,
            "algorithm",
        )
        .unwrap_or_else(|| node.id.clone()),
        FinanceNodeKind::PerformanceAnalytics => property_string(node, "name")
            .unwrap_or_else(|| "Performance Report".to_string()),
    }
}

fn property_string(node: &NodeInstance, key: &str) -> Option<String> {
    node.properties.get(key).and_then(json_to_string)
}

fn json_to_string(value: &graphy::JsonValue) -> Option<String> {
    match value {
        graphy::JsonValue::String(text) => Some(text.clone()),
        graphy::JsonValue::Number(number) => Some(number.to_string()),
        graphy::JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// Filter rows by case-insensitive substring match on label, type, or prim path.
pub fn filter_stage_tree_model(
    model: &FinanceStageTreeModel,
    query: &str,
) -> FinanceStageTreeModel {
    let query = query.trim();
    if query.is_empty() {
        return model.clone();
    }
    let needle = query.to_ascii_lowercase();
    let mut visible: HashSet<usize> = HashSet::new();

    for row in &model.rows {
        if row.is_scope_folder {
            continue;
        }
        let haystack = format!(
            "{} {} {}",
            row.label,
            row.type_label,
            row.prim_path.as_deref().unwrap_or_default()
        )
        .to_ascii_lowercase();
        if haystack.contains(&needle) {
            visible.insert(row.row_index);
            mark_ancestors_visible(model, row.row_index, &mut visible);
        }
    }

    if visible.is_empty() {
        return FinanceStageTreeModel {
            rows: model.rows.clone(),
            root_ids: model.root_ids.clone(),
        };
    }

    let mut rows = model.rows.clone();
    for index in 0..rows.len() {
        if rows[index].is_scope_folder {
            rows[index].children = rows[index]
                .children
                .iter()
                .copied()
                .filter(|child| subtree_has_visible(&rows, *child, &visible))
                .collect();
        } else if !visible.contains(&rows[index].row_index) {
            rows[index].children.clear();
        }
    }

    let root_ids: Vec<usize> = model
        .root_ids
        .iter()
        .copied()
        .filter(|root| subtree_has_visible(&rows, *root, &visible))
        .collect();

    FinanceStageTreeModel { rows, root_ids }
}

fn mark_ancestors_visible(
    model: &FinanceStageTreeModel,
    mut index: usize,
    visible: &mut HashSet<usize>,
) {
    visible.insert(index);
    loop {
        let Some(parent) = model
            .rows
            .iter()
            .find(|row| row.children.contains(&index))
            .map(|row| row.row_index)
        else {
            break;
        };
        if !visible.insert(parent) {
            break;
        }
        index = parent;
    }
}

fn subtree_has_visible(
    rows: &[FinanceStageTreeRow],
    index: usize,
    visible: &HashSet<usize>,
) -> bool {
    let Some(row) = rows.get(index) else {
        return false;
    };
    if !row.is_scope_folder && visible.contains(&index) {
        return true;
    }
    row.children
        .iter()
        .any(|child| subtree_has_visible(rows, *child, visible))
}

#[cfg(test)]
mod tests {
    use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

    use super::*;
    use crate::types::type_id;

    #[test]
    fn stage_tree_groups_finance_nodes_by_scope() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend",
            type_id::TA_TREND,
            Position::new(100.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        ));

        let model = build_finance_stage_tree(&graph);
        assert_eq!(model.root_ids.len(), 3);
        assert_eq!(model.rows.len(), 6);

        let universe = &model.rows[0];
        assert_eq!(universe.children.len(), 1);
        assert_eq!(model.rows[universe.children[0]].label, "SPY");
    }

    #[test]
    fn stage_tree_nests_wired_nodes_under_portfolio() {
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

        let model = build_finance_stage_tree(&graph);
        let universe = &model.rows[0];
        assert_eq!(universe.children.len(), 1);
        assert_eq!(model.rows[universe.children[0]].label, "SPY");

        let portfolio_row = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("fund"))
            .expect("portfolio row");
        assert_eq!(portfolio_row.children.len(), 1);
        let asset_ref = &model.rows[portfolio_row.children[0]];
        assert_eq!(asset_ref.type_label, "AssetRef");
        assert_eq!(asset_ref.label, "SPY");
        assert_eq!(asset_ref.node_id.as_deref(), Some("spy"));
    }

    #[test]
    fn stage_tree_nests_asset_under_ta_under_portfolio() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend",
            type_id::TA_TREND,
            Position::new(100.0, 0.0),
        ));
        let mut ear_fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        );
        ear_fund.properties.insert(
            "name".into(),
            graphy::JsonValue::String("ear_fund".into()),
        );
        graph.add_node(ear_fund);
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "trend".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let model = build_finance_stage_tree(&graph);
        let portfolio_row = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("ear_fund"))
            .expect("ear_fund row");
        assert_eq!(portfolio_row.children.len(), 1);
        let ta_row = &model.rows[portfolio_row.children[0]];
        assert_eq!(ta_row.node_id.as_deref(), Some("trend"));
        assert_eq!(ta_row.children.len(), 1);
        let asset_ref = &model.rows[ta_row.children[0]];
        assert_eq!(asset_ref.type_label, "AssetRef");
        assert_eq!(asset_ref.node_id.as_deref(), Some("spy"));

        let universe = &model.rows[0];
        assert_eq!(universe.children.len(), 1);
    }

    #[test]
    fn stage_tree_nests_sub_portfolio_lineage_under_master() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend",
            type_id::TA_TREND,
            Position::new(100.0, 0.0),
        ));
        let mut ear_fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        );
        ear_fund.properties.insert(
            "name".into(),
            JsonValue::String("ear_fund".into()),
        );
        let mut master = NodeInstance::new(
            "master",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(300.0, 0.0),
        );
        master
            .properties
            .insert("name".into(), JsonValue::String("fund".into()));
        graph.add_node(ear_fund);
        graph.add_node(master);
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "trend".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ear_fund".into(),
            source_pin: "wealth".into(),
            target_node: "master".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let model = build_finance_stage_tree(&graph);
        let master_row = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("master"))
            .expect("master row");
        assert!(
            master_row
                .children
                .iter()
                .any(|child| model.rows[*child].node_id.as_deref() == Some("ear_fund")),
            "master should nest ear_fund"
        );
        let ear_row = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("ear_fund"))
            .expect("ear_fund row");
        assert!(
            ear_row
                .children
                .iter()
                .any(|child| model.rows[*child].node_id.as_deref() == Some("trend")),
            "ear_fund should nest TA"
        );
    }

    #[test]
    fn stage_tree_nests_portfolio_under_ta_under_master() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "ta_ear",
            type_id::TA_TREND,
            Position::new(100.0, 0.0),
        ));
        let mut ear_fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        );
        ear_fund.properties.insert(
            "name".into(),
            JsonValue::String("ear_fund".into()),
        );
        graph.add_node(ear_fund);
        graph.add_node(NodeInstance::new(
            "ta_master",
            type_id::TA_TREND,
            Position::new(300.0, 0.0),
        ));
        let mut master = NodeInstance::new(
            "master",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(400.0, 0.0),
        );
        master
            .properties
            .insert("name".into(), JsonValue::String("fund".into()));
        graph.add_node(master);

        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "ta_ear".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_ear".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ear_fund".into(),
            source_pin: "wealth".into(),
            target_node: "ta_master".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_master".into(),
            source_pin: "result".into(),
            target_node: "master".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let model = build_finance_stage_tree(&graph);
        let master_row = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("master"))
            .expect("master row");
        let ta_master_row = master_row
            .children
            .iter()
            .map(|child| &model.rows[*child])
            .find(|row| row.node_id.as_deref() == Some("ta_master"))
            .expect("ta_master nested under master");
        assert!(
            ta_master_row
                .children
                .iter()
                .any(|child| model.rows[*child].node_id.as_deref() == Some("ear_fund")),
            "ear_fund wealth source should appear under ta_master"
        );
    }

    #[test]
    fn stage_tree_dedupes_duplicate_asset_wires_under_portfolio() {
        let mut graph = GraphDescription::new("test");
        for id in ["stream_1", "ear_fund"] {
            graph.add_node(NodeInstance::new(
                id,
                if id == "stream_1" {
                    type_id::FINANCIAL_RETURN_ASSET
                } else {
                    type_id::PORTFOLIO_INTEGRATOR
                },
                Position::new(0.0, 0.0),
            ));
        }
        for pin in ["signal_0", "signal_1"] {
            graph.add_connection(Connection {
                source_node: "stream_1".into(),
                source_pin: "close".into(),
                target_node: "ear_fund".into(),
                target_pin: pin.into(),
                connection_type: ConnectionType::Data,
            });
        }

        let model = build_finance_stage_tree(&graph);
        let ear = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("ear_fund"))
            .expect("ear_fund");
        assert_eq!(
            ear.children.len(),
            1,
            "duplicate wires to the same portfolio leg should appear once in the stage tree"
        );
    }

    #[test]
    fn stage_tree_asset_ref_stub_per_portfolio_not_shared_row() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "vea",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "equities",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "rates",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(400.0, 0.0),
        ));
        for target in ["equities", "rates"] {
            graph.add_connection(Connection {
                source_node: "vea".into(),
                source_pin: "close".into(),
                target_node: target.into(),
                target_pin: "signal_0".into(),
                connection_type: ConnectionType::Data,
            });
        }

        let model = build_finance_stage_tree(&graph);
        let universe = &model.rows[0];
        assert_eq!(universe.children.len(), 1);

        let equities = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("equities"))
            .expect("equities portfolio");
        let rates = model
            .rows
            .iter()
            .find(|row| row.node_id.as_deref() == Some("rates"))
            .expect("rates portfolio");
        assert_eq!(equities.children.len(), 1);
        assert_eq!(rates.children.len(), 1);
        assert_ne!(equities.children[0], rates.children[0]);
        assert_eq!(model.rows[equities.children[0]].label, "VEA");
        assert_eq!(model.rows[rates.children[0]].label, "VEA");
    }
}
