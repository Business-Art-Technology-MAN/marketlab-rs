//! Restore visual canvas nodes from a composed OpenUSD stage.

use std::collections::HashMap;

use openusd::sdf::Path;
use pulsar_marketlab::stage_bridge::UsdStageBridge;
use pulsar_marketlab::trading_stage::{
    classify_type_name, is_operational_instance_path, is_schema_template_prim, ExecutablePrimKind,
    MARKETLAB_ROOT,
};
use pulsar_marketlab::technical_analysis::{
    ta_indicator_label, DEFAULT_TA_INDICATOR_ID, DEFAULT_TA_LOOKBACK,
};
use pulsar_marketlab_ui::workspace::blender_slot_position;

use crate::canvas_compose::{compose_pipeline_usda, resolve_node_stage_paths, stage_prim_path_for_node};
use crate::graph_compiler::{
    apply_canonical_ta_ports, ta_uber_from_legacy_indicator, AssetSourceType, NodeConnection,
    NodeGradeType, NodeType, TaArchetype, TaUberSignalConfig, VisualNode,
};
use crate::workspace_state::resolve_csv_path;

const LINEAGE_RELATIONSHIPS: &[&str] = &["inputs:underlying", "inputs:sources"];

/// Canvas graph rebuilt from a composed USD stage.
#[derive(Debug, Clone, Default)]
pub struct HydratedCanvas {
    pub nodes: Vec<VisualNode>,
    pub connections: Vec<NodeConnection>,
}

/// Walk executable prims and rebuild canvas nodes plus wire connections.
pub fn hydrate_canvas_from_stage(bridge: &UsdStageBridge) -> HydratedCanvas {
    let mut canvas = HydratedCanvas::default();
    let mut ordered_paths: Vec<(String, ExecutablePrimKind)> = Vec::new();

    let _ = bridge.with_stage(|stage| {
        let root = Path::new("/")
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        collect_executable_prim_paths(stage, bridge, &root, &mut ordered_paths)?;
        Ok(())
    });

    ordered_paths.sort_by(|(left, left_kind), (right, right_kind)| {
        blender_tier_for_kind(*left_kind)
            .cmp(&blender_tier_for_kind(*right_kind))
            .then_with(|| left.cmp(right))
    });

    let mut tier_rows: HashMap<u8, usize> = HashMap::new();
    let mut path_to_id: HashMap<String, usize> = HashMap::new();

    for (index, (prim_path, kind)) in ordered_paths.iter().enumerate() {
        let node_id = index + 1;
        path_to_id.insert(prim_path.clone(), node_id);

        let tier = blender_tier_for_kind(*kind);
        let row = tier_rows.entry(tier).or_insert(0);
        let fallback = blender_slot_position(tier, *row);
        *row += 1;

        let (x, y) = bridge
            .field_vec2f(prim_path, "ui:canvas:pos")
            .map(|[px, py]| (px, py))
            .unwrap_or(fallback);

        if let Some(node) = build_visual_node(node_id, prim_path, *kind, (x, y), bridge) {
            canvas.nodes.push(node);
        }
    }

    canvas.connections = hydrate_connections(bridge, &path_to_id);
    canvas
}

fn collect_executable_prim_paths(
    stage: &openusd::Stage,
    bridge: &UsdStageBridge,
    path: &Path,
    out: &mut Vec<(String, ExecutablePrimKind)>,
) -> Result<(), std::io::Error> {
    let children = stage
        .prim_children(path.clone())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    for child_name in children {
        let child_path = path
            .append_path(child_name.as_str())
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let path_str = child_path.to_string();
        if !bridge.prim_active(&path_str) || is_schema_template_prim(&path_str) {
            continue;
        }
        if let Some(type_name) = bridge.prim_type_name(&path_str) {
            if let Some(kind) = classify_type_name(&type_name) {
                out.push((path_str.clone(), kind));
            }
        } else if is_operational_instance_path(&path_str) {
            if let Some(kind) = legacy_kind_from_path(&path_str) {
                out.push((path_str.clone(), kind));
            }
        }
        collect_executable_prim_paths(stage, bridge, &child_path, out)?;
    }
    Ok(())
}

fn legacy_kind_from_path(path: &str) -> Option<ExecutablePrimKind> {
    if path.starts_with("/assets/") || path.starts_with(&format!("{MARKETLAB_ROOT}/")) {
        // disambiguate by typeName elsewhere; fallback heuristic uses leaf naming
        if path.contains("Portfolio") || path.contains("Sim_") {
            return Some(ExecutablePrimKind::PortfolioIntegrator);
        }
        None
    } else if path.starts_with("/analytics/") {
        Some(ExecutablePrimKind::OtlTaUberSignal)
    } else if path.starts_with("/portfolios/") {
        Some(ExecutablePrimKind::PortfolioIntegrator)
    } else {
        None
    }
}

fn blender_tier_for_kind(kind: ExecutablePrimKind) -> u8 {
    match kind {
        ExecutablePrimKind::FinancialAsset => 0,
        ExecutablePrimKind::OtlOperator | ExecutablePrimKind::OtlTaUberSignal => 1,
        ExecutablePrimKind::PortfolioIntegrator => 2,
    }
}

fn hydrate_asset_source(
    prim_path: &str,
    symbol: &str,
    bridge: &UsdStageBridge,
) -> Option<AssetSourceType> {
    if let Some(path) = bridge.field_string(prim_path, "inputs:csv_path") {
        let path = path.trim();
        if !path.is_empty() {
            return Some(AssetSourceType::Csv {
                path: path.to_string(),
            });
        }
    }

    let symbol_key = symbol.trim_end_matches(".csv");
    let candidate = format!("data/{symbol_key}.csv");
    if resolve_csv_path(&candidate).is_file() {
        return Some(AssetSourceType::Csv { path: candidate });
    }

    None
}

fn build_visual_node(
    id: usize,
    prim_path: &str,
    kind: ExecutablePrimKind,
    (x, y): (f32, f32),
    bridge: &UsdStageBridge,
) -> Option<VisualNode> {
    match kind {
        ExecutablePrimKind::FinancialAsset => {
            let symbol = bridge
                .field_string(prim_path, "inputs:symbol")
                .or_else(|| prim_path.rsplit('/').next().map(str::to_string))
                .unwrap_or_else(|| "ASSET".to_string());
            let asset_source = hydrate_asset_source(prim_path, &symbol, bridge);
            Some(VisualNode {
                id,
                name: format!("{symbol}.csv"),
                node_type: NodeType::asset_adaptor(prim_path.to_string()),
                grade: NodeGradeType::Scalar,
                portfolio_allocation_id: None,
                dsl_formula: None,
                aov_outputs: Vec::new(),
                asset_source,
                x,
                y,
                collapsed: false,
                inputs: vec![],
                outputs: vec!["Close Out".to_string()],
            })
        }
        ExecutablePrimKind::OtlTaUberSignal => {
            let config = hydrate_ta_uber_config(prim_path, bridge);
            let label = ta_indicator_label(&config.algorithm)
                .unwrap_or(config.algorithm.as_str())
                .to_string();
            let mut node = VisualNode {
                id,
                name: label,
                node_type: NodeType::ta_uber_signal(config),
                grade: NodeGradeType::Scalar,
                portfolio_allocation_id: None,
                dsl_formula: None,
                aov_outputs: Vec::new(),
                asset_source: None,
                x,
                y,
                collapsed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
            };
            apply_canonical_ta_ports(&mut node);
            Some(node)
        }
        ExecutablePrimKind::OtlOperator => {
            if bridge.field_string(prim_path, "info:archetype").is_some()
                || bridge.field_string(prim_path, "inputs:id").is_some()
            {
                return build_visual_node(id, prim_path, ExecutablePrimKind::OtlTaUberSignal, (x, y), bridge);
            }
            let script = bridge
                .field_string(prim_path, "inputs:script_src")
                .unwrap_or_default();
            let compiled_path = bridge
                .field_string(prim_path, "inputs:script_compiled_path")
                .filter(|path| !path.trim().is_empty());
            let node_type = NodeType::OtlShader {
                script: script.clone(),
                compiled_path: compiled_path.clone(),
            };
            let dsl_formula = if script.trim().is_empty() {
                None
            } else {
                Some(script.clone())
            };
            let mut node = VisualNode {
                id,
                name: "OTL Shader".to_string(),
                node_type,
                grade: NodeGradeType::Scalar,
                portfolio_allocation_id: None,
                dsl_formula,
                aov_outputs: Vec::new(),
                asset_source: None,
                x,
                y,
                collapsed: false,
                inputs: vec!["source_stream".to_string()],
                outputs: vec!["signal".to_string()],
            };
            if let Some(path) = compiled_path.as_deref() {
                if let Ok(asset) = pulsar_marketlab_core::load_compiled_asset_from_path(path) {
                    let _ = crate::graph_compiler::apply_compiled_otc_asset_to_node(
                        &mut node,
                        &asset,
                        &mut Vec::new(),
                    );
                }
            }
            Some(node)
        }
        ExecutablePrimKind::PortfolioIntegrator => {
            let leaf = prim_path.rsplit('/').next().unwrap_or("portfolio");
            let allocation = bridge.field_string(prim_path, "inputs:id");
            let is_portfolio = allocation
                .as_deref()
                .map(|id| id.starts_with("Allocation::") || id == "portfolio")
                .unwrap_or(true);
            let node_type = if is_portfolio {
                NodeType::portfolio()
            } else {
                NodeType::terminal_integrator(allocation.clone().unwrap_or_default())
            };
            Some(VisualNode {
                id,
                name: leaf.replace('_', " "),
                node_type,
                grade: NodeGradeType::Scalar,
                portfolio_allocation_id: allocation.filter(|id| id.starts_with("Allocation::")),
                dsl_formula: None,
                aov_outputs: Vec::new(),
                asset_source: None,
                x,
                y,
                collapsed: false,
                inputs: vec!["Signal In 0".to_string()],
                outputs: vec!["Portfolio Out".to_string()],
            })
        }
    }
}

fn hydrate_ta_uber_config(prim_path: &str, bridge: &UsdStageBridge) -> TaUberSignalConfig {
    if let Some(archetype_token) = bridge.field_string(prim_path, "info:archetype") {
        let archetype = TaArchetype::from_token(&archetype_token)
            .unwrap_or(TaArchetype::Oscillator);
        let algorithm = bridge
            .field_string(prim_path, "info:algorithm")
            .unwrap_or_else(|| archetype.default_algorithm().to_string());
        let period = field_u32_attr(bridge, prim_path, "inputs:period")
            .unwrap_or_else(|| archetype.default_period())
            .max(1);
        let signal_period = field_u32_attr(bridge, prim_path, "inputs:signal_period")
            .unwrap_or(9)
            .max(1);
        let multiplier = bridge
            .field_f32(prim_path, "inputs:multiplier")
            .unwrap_or(2.0);
        let annualization = bridge
            .field_f32(prim_path, "inputs:annualization")
            .unwrap_or(252.0);
        let mut config = TaUberSignalConfig {
            archetype,
            algorithm,
            period,
            multiplier,
            annualization,
            signal_period,
        };
        config.normalize_algorithm();
        return config;
    }

    let indicator_id = bridge
        .field_string(prim_path, "inputs:id")
        .or_else(|| prim_path.rsplit('/').next().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_TA_INDICATOR_ID.to_string());
    let script = bridge
        .field_string(prim_path, "inputs:script_src")
        .unwrap_or_default();
    let lookback = parse_ta_lookback_period(&script);
    ta_uber_from_legacy_indicator(&indicator_id, lookback)
}

fn field_u32_attr(bridge: &UsdStageBridge, prim_path: &str, attribute: &str) -> Option<u32> {
    bridge
        .field_string(prim_path, attribute)
        .and_then(|text| text.trim().parse().ok())
        .or_else(|| {
            bridge
                .field_f32(prim_path, attribute)
                .map(|value| value.max(0.0) as u32)
        })
}

fn parse_ta_lookback_period(script: &str) -> u32 {
    let Some(start) = script.find("period=") else {
        return DEFAULT_TA_LOOKBACK as u32;
    };
    let digits = script[start + "period=".len()..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits
        .parse()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TA_LOOKBACK as u32)
}

fn hydrate_connections(
    bridge: &UsdStageBridge,
    path_to_id: &HashMap<String, usize>,
) -> Vec<NodeConnection> {
    let mut connections = Vec::new();
    let mut portfolio_port_cursor: HashMap<usize, usize> = HashMap::new();

    for target_path in path_to_id.keys() {
        for relationship in LINEAGE_RELATIONSHIPS {
            for source_path in bridge.relationship_targets(target_path, relationship) {
                let Some(from_node_id) = path_to_id.get(&source_path).copied() else {
                    continue;
                };
                let Some(to_node_id) = path_to_id.get(target_path).copied() else {
                    continue;
                };
                let to_port_idx = if bridge.prim_type_name(target_path).as_deref()
                    == Some("PortfolioIntegrator")
                {
                    let port = portfolio_port_cursor.entry(to_node_id).or_insert(0);
                    let idx = *port;
                    *port += 1;
                    idx
                } else {
                    0
                };
                connections.push(NodeConnection {
                    from_node_id,
                    from_port_idx: 0,
                    to_node_id,
                    to_port_idx,
                });
            }
        }
    }

    connections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_compiler::{apply_canonical_ta_ports, ta_uber_from_legacy_indicator, AssetSourceType};
    use pulsar_marketlab::stage_bridge::UsdStageBridge;

    fn sample_asset(id: usize) -> VisualNode {
        VisualNode {
            id,
            name: "GLD.csv".to_string(),
            node_type: NodeType::asset_adaptor("/MarketLab/GLD".to_string()),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: Some(AssetSourceType::Csv {
                path: "data/GLD.csv".to_string(),
            }),
            x: 120.0,
            y: 80.0,
            collapsed: false,
            inputs: vec![],
            outputs: vec!["Close Out".to_string()],
        }
    }

    fn sample_ta(id: usize) -> VisualNode {
        let mut node = VisualNode {
            id,
            name: "RSI".to_string(),
            node_type: NodeType::ta_uber_signal(ta_uber_from_legacy_indicator("rsi", 14)),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 450.0,
            y: 320.0,
            collapsed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        apply_canonical_ta_ports(&mut node);
        node
    }

    #[test]
    fn hydrate_restores_canvas_positions_and_wires() {
        let nodes = vec![sample_asset(1), sample_ta(2)];
        let connections = vec![NodeConnection {
            from_node_id: 1,
            from_port_idx: 0,
            to_node_id: 2,
            to_port_idx: 0,
        }];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        let usda = compose_pipeline_usda(&nodes, &connections);
        let bridge = UsdStageBridge::open_from_usda_text(&usda).expect("parse composed stage");
        let hydrated = hydrate_canvas_from_stage(&bridge);

        assert_eq!(hydrated.nodes.len(), 2);
        let asset_path = paths.get(&1).expect("asset path");
        let ta_path = paths.get(&2).expect("ta path");
        let hydrated_paths = resolve_node_stage_paths(&hydrated.nodes, &hydrated.connections);
        let asset = hydrated
            .nodes
            .iter()
            .find(|node| hydrated_paths.get(&node.id).map(String::as_str) == Some(asset_path.as_str()))
            .expect("asset");
        let ta = hydrated
            .nodes
            .iter()
            .find(|node| hydrated_paths.get(&node.id).map(String::as_str) == Some(ta_path.as_str()))
            .expect("ta");
        assert!((asset.x - 120.0).abs() < f32::EPSILON);
        assert!((asset.y - 80.0).abs() < f32::EPSILON);
        assert!((ta.x - 450.0).abs() < f32::EPSILON);
        assert!((ta.y - 320.0).abs() < f32::EPSILON);
        assert_eq!(hydrated.connections.len(), 1);
    }

    #[test]
    fn hydrate_uses_blender_fallback_when_canvas_pos_missing() {
        let usda = r#"#usda 1.0
(
    defaultPrim = "MarketLab"
)

def Scope "MarketLab"
{
    def FinancialAsset "GLD"
    {
        token inputs:symbol = "GLD"
    }
}
"#;
        let bridge = UsdStageBridge::open_from_usda_text(usda).expect("parse stage");
        let hydrated = hydrate_canvas_from_stage(&bridge);
        assert_eq!(hydrated.nodes.len(), 1);
        let (expected_x, expected_y) = blender_slot_position(0, 0);
        assert!((hydrated.nodes[0].x - expected_x).abs() < f32::EPSILON);
        assert!((hydrated.nodes[0].y - expected_y).abs() < f32::EPSILON);
    }
}
