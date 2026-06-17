//! Incremental canvas → USD structural sync (Milestone 3).
//!
//! When canvas topology is stable (same nodes/paths, prims already on stage), wiring and
//! scalar defaults are pushed through [`ManagedUsdStage`] overlays instead of rebuilding
//! the entire root layer from USDA text.

use std::collections::{HashMap, HashSet};

use openusd::sdf::Value;
use pulsar_marketlab_core::compose_uber_script_src;
use pulsar_marketlab_ui::workspace::ManagedUsdStage;

use crate::canvas_compose::{collect_relationships, resolve_node_stage_paths};
use crate::graph_compiler::{
    resolved_otl_script, AssetSourceType, NodeConnection, NodeType, VisualNode,
};
use crate::workspace_state::SIM_INITIAL_CASH;

/// Lineage relationships mirrored from the graph compile walker.
pub const LINEAGE_RELATIONSHIPS: &[&str] = &[
    "inputs:underlying",
    "inputs:sources",
    "inputs:constituents",
    "inputs:target",
];

/// Result of an incremental overlay sync pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IncrementalStageSyncReport {
    pub prims_touched: usize,
    pub relationships_updated: usize,
    pub attributes_updated: usize,
}

/// Whether a full USDA recompose + context reload is required before the stage matches canvas.
pub fn needs_full_stage_recompose(
    stage: &ManagedUsdStage,
    nodes: &[VisualNode],
    connections: &[NodeConnection],
    last_published_paths: &HashMap<usize, String>,
) -> bool {
    if nodes.is_empty() {
        return true;
    }

    let paths = resolve_node_stage_paths(nodes, connections);

    if nodes.len() < last_published_paths.len() {
        return true;
    }

    for node in nodes {
        let Some(path) = paths.get(&node.id) else {
            return true;
        };
        if last_published_paths.get(&node.id) != Some(path) {
            return true;
        }
        if !stage.prim_exists(path) {
            return true;
        }
    }

    false
}

/// Push canvas wiring and scalar defaults into passive USD overlays (no root-layer rebuild).
pub fn apply_incremental_canvas_sync(
    stage: &ManagedUsdStage,
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> IncrementalStageSyncReport {
    let paths = resolve_node_stage_paths(nodes, connections);
    let relationships = collect_relationships(nodes, connections, &paths);
    let mut prim_paths: HashSet<String> = paths.values().cloned().collect();
    prim_paths.extend(relationships.keys().cloned());

    let mut report = IncrementalStageSyncReport::default();

    for prim_path in &prim_paths {
        if !stage.prim_exists(prim_path) {
            continue;
        }
        report.prims_touched += 1;

        for relationship in LINEAGE_RELATIONSHIPS {
            let desired = relationships
                .get(prim_path)
                .and_then(|map| map.get(*relationship))
                .cloned()
                .unwrap_or_default();
            let current = stage.relationship_targets(prim_path, relationship);
            if current != desired {
                stage.set_relationship_targets(prim_path, relationship, desired);
                report.relationships_updated += 1;
            }
        }

        let nodes_by_id: HashMap<usize, &VisualNode> =
            nodes.iter().map(|node| (node.id, node)).collect();
        let Some(node) = paths
            .iter()
            .find_map(|(id, path)| (path == prim_path).then(|| nodes_by_id.get(id)).flatten())
        else {
            continue;
        };

        report.attributes_updated += sync_node_attributes(stage, prim_path, node);
    }

    report
}

/// Scalar prim attributes for engine snapshot building (mirrors [`sync_node_attributes`]).
pub fn canvas_prim_attributes(
    node: &VisualNode,
    prim_path: &str,
) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;

    let mut attrs = HashMap::new();
    let mut insert = |attr: &str, value: String| {
        attrs.insert(attr.to_string(), value);
    };

    match &node.node_type {
        NodeType::AssetAdaptor { .. } => {
            let symbol = prim_path.rsplit('/').next().unwrap_or(prim_path);
            let taxonomy = pulsar_marketlab_core::flatten_asset_metadata(symbol, Some("Equity"));
            insert("inputs:active", "true".to_string());
            insert("inputs:symbol", symbol.to_string());
            insert("inputs:asset_class", taxonomy.asset_class);
            insert("inputs:provider", taxonomy.provider);
            if !taxonomy.category.is_empty() {
                insert("inputs:category", taxonomy.category);
            }
            if !taxonomy.sub_category.is_empty() {
                insert("inputs:sub_category", taxonomy.sub_category);
            }
            if let Some(AssetSourceType::Csv { path: csv_path }) = &node.asset_source {
                insert("inputs:csv_path", csv_path.clone());
            }
        }
        NodeType::TaUberSignal { config } => {
            insert(
                "info:archetype",
                config.archetype.as_token().to_string(),
            );
            insert("info:algorithm", config.algorithm.clone());
            insert("inputs:period", config.period.to_string());
            insert("inputs:signal_period", config.signal_period.to_string());
            insert("inputs:multiplier", config.multiplier.to_string());
            insert("inputs:annualization", config.annualization.to_string());
            insert(
                "inputs:script_src",
                compose_uber_script_src(config),
            );
        }
        NodeType::OtlShader { compiled_path, .. } => {
            insert("inputs:script_src", resolved_otl_script(node));
            if let Some(path) = compiled_path.as_deref().filter(|p| !p.is_empty()) {
                insert("inputs:script_compiled_path", path.to_string());
            }
        }
        NodeType::TerminalIntegrator { .. } if node.node_type.is_portfolio() => {
            let allocation = node
                .portfolio_allocation_id
                .as_deref()
                .unwrap_or("Allocation::HierarchicalRiskParity");
            insert("inputs:id", allocation.to_string());
            insert(
                "inputs:initial_capital",
                SIM_INITIAL_CASH.to_string(),
            );
            insert("inputs:rebalance_frequency", "monthly".to_string());
        }
        NodeType::TerminalIntegrator { engine_target } if !node.node_type.is_portfolio() => {
            insert("inputs:id", engine_target.clone());
        }
        NodeType::TerminalIntegrator { .. } => {}
    }

    let label = node.name.trim();
    if !label.is_empty() {
        insert(
            pulsar_marketlab_core::USER_LABEL_ATTR,
            label.to_string(),
        );
    }

    attrs
}

fn sync_node_attributes(stage: &ManagedUsdStage, prim_path: &str, node: &VisualNode) -> usize {
    let mut updated = 0usize;
    let mut touch = |attr: &str, value: Value| {
        let property_path = format!("{prim_path}.{attr}");
        if stage.field(&property_path, "default").as_ref() != Some(&value) {
            stage.set_field(&property_path, "default", value);
            updated += 1;
        }
    };

    match &node.node_type {
        NodeType::AssetAdaptor { .. } => {
            let symbol = prim_path.rsplit('/').next().unwrap_or(prim_path);
            let taxonomy = pulsar_marketlab_core::flatten_asset_metadata(symbol, Some("Equity"));
            touch("inputs:active", Value::Bool(true));
            touch("inputs:symbol", Value::String(symbol.to_string()));
            touch("inputs:asset_class", Value::String(taxonomy.asset_class));
            touch("inputs:provider", Value::String(taxonomy.provider));
            if !taxonomy.category.is_empty() {
                touch("inputs:category", Value::String(taxonomy.category));
            }
            if !taxonomy.sub_category.is_empty() {
                touch("inputs:sub_category", Value::String(taxonomy.sub_category));
            }
            if let Some(AssetSourceType::Csv { path: csv_path }) = &node.asset_source {
                touch("inputs:csv_path", Value::String(csv_path.clone()));
            }
        }
        NodeType::TaUberSignal { config } => {
            touch(
                "info:archetype",
                Value::String(config.archetype.as_token().to_string()),
            );
            touch(
                "info:algorithm",
                Value::String(config.algorithm.clone()),
            );
            touch("inputs:period", Value::Int(config.period as i32));
            touch(
                "inputs:signal_period",
                Value::Int(config.signal_period as i32),
            );
            touch("inputs:multiplier", Value::Float(config.multiplier));
            touch("inputs:annualization", Value::Float(config.annualization));
            touch(
                "inputs:script_src",
                Value::String(compose_uber_script_src(config)),
            );
        }
        NodeType::OtlShader { compiled_path, .. } => {
            touch(
                "inputs:script_src",
                Value::String(resolved_otl_script(node)),
            );
            if let Some(path) = compiled_path.as_deref().filter(|p| !p.is_empty()) {
                touch("inputs:script_compiled_path", Value::String(path.to_string()));
            }
        }
        NodeType::TerminalIntegrator { .. } if node.node_type.is_portfolio() => {
            let allocation = node
                .portfolio_allocation_id
                .as_deref()
                .unwrap_or("Allocation::HierarchicalRiskParity");
            touch("inputs:id", Value::String(allocation.to_string()));
            touch(
                "inputs:initial_capital",
                Value::Double(SIM_INITIAL_CASH),
            );
            touch(
                "inputs:rebalance_frequency",
                Value::String("monthly".to_string()),
            );
        }
        _ => {}
    }

    // `ui:canvas:pos` is authored at compose time; position-only edits stay on canvas until publish.
    let label = node.name.trim();
    if !label.is_empty() {
        touch(
            pulsar_marketlab_core::USER_LABEL_ATTR,
            Value::String(label.to_string()),
        );
    }

    updated
}

/// Snapshot of resolved prim paths after a successful publish (incremental or full).
pub fn published_node_paths(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> HashMap<usize, String> {
    resolve_node_stage_paths(nodes, connections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas_compose::compose_pipeline_usda_for_tests;
    use pulsar_marketlab_core::{PORTFOLIOS_SCOPE, SIGNALS_SCOPE, UNIVERSE_SCOPE};
    use crate::canvas_compose::workstation_stable_path;
    use crate::graph_compiler::{NodeGradeType, VisualNode};
    use pulsar_marketlab_ui::workspace::ManagedUsdStage;

    fn sample_asset_node(id: usize, name: &str) -> VisualNode {
        VisualNode {
            id,
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
            name: format!("{name}.csv"),
            node_type: NodeType::asset_adaptor(format!("/MarketLab/{name}")),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 0.0,
            y: 0.0,
            collapsed: false,
            inputs: vec![],
            outputs: vec!["Close Out".to_string()],
        }
    }

    fn sample_shader_node(id: usize) -> VisualNode {
        VisualNode {
            id,
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
            name: "rsi".to_string(),
            node_type: NodeType::OtlShader {
                script: "identity".to_string(),
                compiled_path: None,
            },
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: Some("identity".to_string()),
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 100.0,
            y: 0.0,
            collapsed: false,
            inputs: vec!["In".to_string()],
            outputs: vec!["Out".to_string()],
        }
    }

    #[test]
    fn incremental_sync_updates_relationship_overlay() {
        let nodes = vec![sample_asset_node(1, "SPY"), sample_shader_node(2)];
        let connections = vec![NodeConnection {
            from_node_id: 1,
            to_node_id: 2,
            from_port_idx: 0,
            to_port_idx: 0,
        }];

        let usda = compose_pipeline_usda_for_tests(&nodes, &[]);
        let stage = ManagedUsdStage::open_from_usda_text(&usda).expect("open stage");
        assert!(needs_full_stage_recompose(&stage, &nodes, &[], &HashMap::new()));

        let usda = compose_pipeline_usda_for_tests(&nodes, &connections);
        let stage = ManagedUsdStage::open_from_usda_text(&usda).expect("open wired stage");
        let paths = published_node_paths(&nodes, &connections);

        assert!(!needs_full_stage_recompose(
            &stage,
            &nodes,
            &connections,
            &paths
        ));

        let spy_path = workstation_stable_path(UNIVERSE_SCOPE, 1);
        let signal_path = workstation_stable_path(SIGNALS_SCOPE, 2);
        stage.set_relationship_targets(&signal_path, "inputs:underlying", vec![]);
        let report = apply_incremental_canvas_sync(&stage, &nodes, &connections);
        assert!(report.relationships_updated >= 1);
        assert_eq!(
            stage.relationship_targets(&signal_path, "inputs:underlying"),
            vec![spy_path]
        );
    }
}
