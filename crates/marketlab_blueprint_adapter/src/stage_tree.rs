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

    // Nest wired upstream nodes under their portfolio parent.
    // Financial assets stay in Universe; portfolios get lightweight AssetRef stub rows.
    for connection in &graph.connections {
        let Some(&target_row) = node_row_by_id.get(&connection.target_node) else {
            continue;
        };
        let target_node = graph.nodes.get(&connection.target_node);
        let Some(target_kind) = target_node
            .and_then(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type))
        else {
            continue;
        };
        if target_kind != FinanceNodeKind::PortfolioIntegrator {
            continue;
        }
        let Some(source_node) = graph.nodes.get(&connection.source_node) else {
            continue;
        };
        let Some(source_kind) = FinanceNodeKind::from_graphy_type_id(&source_node.node_type) else {
            continue;
        };

        if source_kind == FinanceNodeKind::FinancialAsset {
            let source_path = paths.get(&connection.source_node).cloned();
            let ref_label = node_display_label(source_node, source_kind, source_path.as_deref().unwrap_or(""));
            let ref_row = rows.len();
            rows.push(FinanceStageTreeRow {
                row_index: ref_row,
                node_id: Some(connection.source_node.clone()),
                label: ref_label,
                type_label: "AssetRef".to_string(),
                prim_path: source_path,
                children: Vec::new(),
                is_scope_folder: false,
            });
            if !rows[target_row].children.contains(&ref_row) {
                rows[target_row].children.push(ref_row);
            }
            continue;
        }

        let Some(&source_row) = node_row_by_id.get(&connection.source_node) else {
            continue;
        };
        if source_row == target_row {
            continue;
        }

        let source_scope = scope_for_kind(source_kind);
        if let Some(folder_index) = scope_folder_index.get(source_scope).copied() {
            rows[folder_index].children.retain(|child| *child != source_row);
        }

        if !rows[target_row].children.contains(&source_row) {
            rows[target_row].children.push(source_row);
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

fn scope_for_kind(kind: FinanceNodeKind) -> &'static str {
    match kind {
        FinanceNodeKind::FinancialAsset => category::UNIVERSE,
        FinanceNodeKind::OtlOperator | FinanceNodeKind::OtlTaUberSignal => category::ANALYTICS,
        FinanceNodeKind::PortfolioIntegrator => category::PORTFOLIOS,
        FinanceNodeKind::PerformanceAnalytics => category::REPORTING,
    }
}

fn type_display_label(kind: FinanceNodeKind) -> String {
    match kind {
        FinanceNodeKind::FinancialAsset => "AssetRef".to_string(),
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
        FinanceNodeKind::FinancialAsset => property_string(node, "symbol").unwrap_or_else(|| {
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
