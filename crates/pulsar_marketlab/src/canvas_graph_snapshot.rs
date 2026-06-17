//! Build engine graph snapshots directly from in-memory canvas topology (no USDA round-trip).

use std::collections::HashMap;

use pulsar_marketlab_core::{
    ComposedAssetMeta, GraphCompileWire, StageGraphPrim, StageGraphSnapshot, USER_LABEL_ATTR,
};
use pulsar_marketlab_ui::workspace::build_path_binding_index;

use crate::canvas_compose::{
    collect_relationships, prim_schema_type_name, resolve_node_stage_paths,
};
use crate::canvas_stage_sync::{canvas_prim_attributes, LINEAGE_RELATIONSHIPS};
use crate::graph_compiler::{NodeConnection, VisualNode};

fn composed_asset_meta_from_attributes(prim: &StageGraphPrim) -> ComposedAssetMeta {
    fn attr(prim: &StageGraphPrim, key: &str) -> String {
        prim.attributes.get(key).cloned().unwrap_or_default()
    }

    let symbol = {
        let symbol = attr(prim, "inputs:symbol");
        if symbol.is_empty() {
            prim.path
                .rsplit('/')
                .next()
                .unwrap_or(prim.path.as_str())
                .to_string()
        } else {
            symbol
        }
    };
    let asset_class = attr(prim, "inputs:asset_class");
    let is_active = prim
        .attributes
        .get("inputs:active")
        .map(|value| value == "true")
        .unwrap_or(true);

    ComposedAssetMeta {
        symbol,
        asset_class: if asset_class.is_empty() {
            "Equity".to_string()
        } else {
            asset_class
        },
        category: attr(prim, "inputs:category"),
        sub_category: attr(prim, "inputs:sub_category"),
        is_active,
        sector: attr(prim, "info:sector"),
        industry: attr(prim, "info:industry"),
        market_cap_class: attr(prim, "info:market_cap_class"),
        currency: attr(prim, "info:currency"),
        country: attr(prim, "info:country"),
        user_label: attr(prim, USER_LABEL_ATTR),
    }
}

fn build_asset_registry_from_prims(prims: &[StageGraphPrim]) -> HashMap<String, ComposedAssetMeta> {
    let mut registry = HashMap::new();
    for prim in prims {
        if prim.type_name != "FinancialAsset" {
            continue;
        }
        registry.insert(
            prim.path.clone(),
            composed_asset_meta_from_attributes(prim),
        );
    }
    registry
}

/// Build a [`StageGraphSnapshot`] from canvas nodes and connections without composing or parsing USDA.
pub fn build_stage_graph_snapshot_from_graph(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> StageGraphSnapshot {
    let paths = resolve_node_stage_paths(nodes, connections);
    let relationships = collect_relationships(nodes, connections, &paths);

    let mut prims = Vec::with_capacity(nodes.len());
    for node in nodes {
        let Some(path) = paths.get(&node.id) else {
            continue;
        };
        prims.push(StageGraphPrim {
            path: path.clone(),
            type_name: prim_schema_type_name(node).to_string(),
            attributes: canvas_prim_attributes(node, path),
        });
    }

    let mut wires = Vec::new();
    for (target_path, rel_map) in &relationships {
        for relationship in LINEAGE_RELATIONSHIPS {
            let Some(sources) = rel_map.get(*relationship) else {
                continue;
            };
            for source_path in sources {
                wires.push(GraphCompileWire {
                    source_prim_path: source_path.clone(),
                    target_prim_path: target_path.clone(),
                    relationship: (*relationship).to_string(),
                });
            }
        }
    }

    let path_bindings = build_path_binding_index(&prims);
    let asset_registry = build_asset_registry_from_prims(&prims);

    StageGraphSnapshot {
        prims,
        wires,
        path_bindings,
        asset_registry,
    }
}

/// Back-compat alias used by graph-engine hosts.
pub fn build_stage_graph_snapshot_from_canvas(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> StageGraphSnapshot {
    build_stage_graph_snapshot_from_graph(nodes, connections)
}

#[cfg(test)]
mod perf_tests {
    use std::time::Instant;

    use pulsar_marketlab_core::{MarketLabGraphEngine, SharedPriceColumn};

    use pulsar_marketlab_ui::workspace::{build_stage_graph_snapshot, WorkspaceContext};

    use super::*;
    use crate::canvas_compose::compose_pipeline_usda;
    use crate::graph_compiler::{AssetSourceType, NodeConnection, NodeGradeType, NodeType, VisualNode};

    const BAR_COUNT: usize = 2872;
    const ASSET_COUNT: usize = 6;

    fn synthetic_series() -> std::sync::Arc<[f64]> {
        (0..BAR_COUNT)
            .map(|i| 100.0 + i as f64 * 0.05)
            .collect::<Vec<_>>()
            .into()
    }

    fn sample_asset(id: usize, label: &str) -> VisualNode {
        VisualNode {
            id,
            stable_prim_leaf: None,
            name: label.to_string(),
            node_type: NodeType::asset_adaptor_from_csv_path(&format!("{label}.csv")),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: Some(AssetSourceType::Csv {
                path: format!("data/{label}.csv"),
            }),
            x: 0.0,
            y: 0.0,
            collapsed: false,
            inputs: vec!["close".to_string()],
            outputs: vec!["close".to_string()],
        }
    }

    fn sample_graph() -> (Vec<VisualNode>, Vec<NodeConnection>) {
        let mut nodes = vec![
            VisualNode {
                id: 1,
                stable_prim_leaf: None,
                name: "Fund".to_string(),
                node_type: NodeType::terminal_integrator("portfolio"),
                grade: NodeGradeType::Scalar,
                portfolio_allocation_id: Some("Allocation::EqualWeight".to_string()),
                dsl_formula: None,
                aov_outputs: Vec::new(),
                asset_source: None,
                x: 0.0,
                y: 0.0,
                collapsed: false,
                inputs: vec!["signal_0".to_string()],
                outputs: vec!["wealth".to_string()],
            },
            sample_asset(2, "SPY"),
        ];
        for idx in 3..=ASSET_COUNT + 1 {
            nodes.push(sample_asset(idx, &format!("ASSET{idx}")));
        }
        let connections: Vec<NodeConnection> = (2..=ASSET_COUNT + 1)
            .map(|asset_id| NodeConnection {
                from_node_id: asset_id,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: asset_id - 2,
            })
            .collect();
        (nodes, connections)
    }

    #[test]
    fn perf_engine_canvas_direct() {
        let (nodes, connections) = sample_graph();
        let series = synthetic_series();
        let mut vectors = std::collections::HashMap::new();
        for node in &nodes {
            if let NodeType::AssetAdaptor { prim_path } = &node.node_type {
                vectors.insert(prim_path.clone(), SharedPriceColumn::from_series(std::sync::Arc::clone(&series)));
            }
        }
        let started = Instant::now();
        let snapshot = build_stage_graph_snapshot_from_graph(&nodes, &connections);
        let mut engine = MarketLabGraphEngine::compile_from_canvas(&snapshot).expect("compile");
        let _ = engine.execute_timeline(vectors, BAR_COUNT);
        eprintln!(
            "perf_engine_canvas_direct (module, {}×{}): {} ms",
            ASSET_COUNT,
            BAR_COUNT,
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn perf_engine_usd_roundtrip() {
        let (nodes, connections) = sample_graph();
        let vectors = {
            let series = synthetic_series();
            let mut map = std::collections::HashMap::new();
            for node in &nodes {
                if let NodeType::AssetAdaptor { prim_path } = &node.node_type {
                    map.insert(
                        prim_path.clone(),
                        SharedPriceColumn::from_series(std::sync::Arc::clone(&series)),
                    );
                }
            }
            map
        };
        let started = Instant::now();
        let usda = compose_pipeline_usda(&nodes, &connections);
        let context = WorkspaceContext::from_usda_text(&usda).unwrap_or_default();
        let snapshot = build_stage_graph_snapshot(context.usd_stage());
        let mut engine = MarketLabGraphEngine::compile_from_stage(&snapshot).expect("compile");
        let _ = engine.execute_timeline(vectors, BAR_COUNT);
        eprintln!(
            "perf_engine_usd_roundtrip (module, {}×{}): {} ms",
            ASSET_COUNT,
            BAR_COUNT,
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn perf_usd_compose_only() {
        let (nodes, connections) = sample_graph();
        let started = Instant::now();
        let _ = compose_pipeline_usda(&nodes, &connections);
        eprintln!(
            "perf_usd_compose_only (module, {}×{}): {} ms",
            ASSET_COUNT,
            BAR_COUNT,
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn direct_snapshot_builds_compilable_portfolio_topology() {
        let (nodes, connections) = sample_graph();
        let direct = build_stage_graph_snapshot_from_graph(&nodes, &connections);

        assert_eq!(direct.prims.len(), ASSET_COUNT + 1);
        assert_eq!(direct.wires.len(), ASSET_COUNT);
        assert!(
            MarketLabGraphEngine::compile_from_canvas(&direct).is_ok(),
            "direct snapshot must compile"
        );
    }

    #[test]
    fn direct_snapshot_produces_same_sweep_as_usda_stage_walk() {
        let (nodes, connections) = sample_graph();
        let series = synthetic_series();
        let vectors: std::collections::HashMap<_, _> = nodes
            .iter()
            .filter_map(|node| {
                let NodeType::AssetAdaptor { prim_path } = &node.node_type else {
                    return None;
                };
                Some((
                    prim_path.clone(),
                    SharedPriceColumn::from_series(std::sync::Arc::clone(&series)),
                ))
            })
            .collect();

        let direct = build_stage_graph_snapshot_from_graph(&nodes, &connections);
        let usda = compose_pipeline_usda(&nodes, &connections);
        let context = WorkspaceContext::from_usda_text(&usda).unwrap_or_default();
        let from_stage = build_stage_graph_snapshot(context.usd_stage());

        assert_eq!(direct.prims.len(), from_stage.prims.len());
        assert!(
            MarketLabGraphEngine::compile_from_canvas(&direct).is_ok(),
            "direct snapshot must compile"
        );
        assert!(
            MarketLabGraphEngine::compile_from_stage(&from_stage).is_ok(),
            "USD stage snapshot must compile"
        );

        let mut direct_engine =
            MarketLabGraphEngine::compile_from_canvas(&direct).expect("direct compile");
        let mut stage_engine =
            MarketLabGraphEngine::compile_from_stage(&from_stage).expect("stage compile");
        let direct_result = direct_engine.execute_timeline(vectors.clone(), BAR_COUNT);
        let stage_result = stage_engine.execute_timeline(vectors, BAR_COUNT);

        let direct_wealth = direct_result
            .portfolio_results
            .values()
            .next()
            .and_then(|integration| integration.wealth_series.last().copied());
        let stage_wealth = stage_result
            .portfolio_results
            .values()
            .next()
            .and_then(|integration| integration.wealth_series.last().copied());
        match (direct_wealth, stage_wealth) {
            (Some(direct), Some(stage)) => {
                let delta = (direct - stage).abs();
                assert!(
                    delta < 1e-6 * direct.abs().max(stage.abs()).max(1.0),
                    "direct canvas ({direct}) and USDA stage walk ({stage}) must agree on portfolio wealth"
                );
            }
            (None, None) => {}
            _ => panic!(
                "direct canvas and USD stage walk must both produce portfolio wealth (direct={direct_wealth:?}, stage={stage_wealth:?})"
            ),
        }
    }

    #[test]
    fn perf_snapshot_build_direct_beats_usd_roundtrip() {
        let (nodes, connections) = sample_graph();

        let direct_started = Instant::now();
        let _ = build_stage_graph_snapshot_from_graph(&nodes, &connections);
        let direct_ms = direct_started.elapsed();

        let roundtrip_started = Instant::now();
        let usda = compose_pipeline_usda(&nodes, &connections);
        let context = WorkspaceContext::from_usda_text(&usda).unwrap_or_default();
        let _ = build_stage_graph_snapshot(context.usd_stage());
        let roundtrip_ms = roundtrip_started.elapsed();

        eprintln!(
            "perf_snapshot_build (module, {} assets): direct {} ms · USDA round-trip {} ms",
            ASSET_COUNT,
            direct_ms.as_millis(),
            roundtrip_ms.as_millis()
        );

        assert!(
            direct_ms < roundtrip_ms,
            "direct canvas snapshot build ({direct_ms:?}) must beat USDA compose+parse ({roundtrip_ms:?})"
        );
    }
}
