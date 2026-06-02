//! Compose an OpenUSD root layer from the visual node canvas graph.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::graph_compiler::{
    resolved_otl_script, validated_connections, AssetSourceType, NodeConnection, NodeType,
    PipelineGraphSnapshot, VisualNode,
};
use pulsar_marketlab_core::compose_uber_script_src;
use pulsar_marketlab::trading_stage::{
    analytics_prim_path, nested_prim_path, portfolio_prim_path, MARKETLAB_DEFAULT_PRIM,
    MARKETLAB_ROOT,
};
use pulsar_marketlab_core::{
    compile_object_program, embed_schema_inline_in_layer, initial_stage_usda,
    schema_sidecar_usda, OtlObjectDeclaration, OtlObjectKind, FrontendError,
    SCHEMA_SIDECAR_FILENAME, SCHEMA_SUBLAYER_REF,
};

/// OTL canvas hydration error when script tier does not match node intent.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HydrationError {
    #[error("expected OTL tier {expected:?}, source declares {actual:?}")]
    TierMismatch {
        expected: OtlObjectKind,
        actual: OtlObjectKind,
    },
    #[error("OTL source produced no object declaration")]
    EmptyProgram,
    #[error(transparent)]
    Frontend(#[from] FrontendError),
}

/// Hydrated canvas node bound to a parsed OTL object declaration.
#[derive(Debug, Clone)]
pub struct CanvasNodeHydration {
    pub prim_path: String,
    pub object: OtlObjectDeclaration,
}

/// Validate and parse OTL source for a canvas prim at the expected three-tier object kind.
pub fn hydrate_canvas_node(
    prim_path: &str,
    script_src: &str,
    expected_tier: OtlObjectKind,
) -> Result<CanvasNodeHydration, HydrationError> {
    let program = compile_object_program(script_src)?;
    let object = program
        .objects
        .into_iter()
        .next()
        .ok_or(HydrationError::EmptyProgram)?;
    if object.kind != expected_tier {
        return Err(HydrationError::TierMismatch {
            expected: expected_tier,
            actual: object.kind,
        });
    }
    Ok(CanvasNodeHydration {
        prim_path: prim_path.to_string(),
        object,
    })
}

/// Controls whether composed USDA references a schema sublayer on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeOptions {
    /// When true, emit `subLayers = [@./schema.usda@]` in the root metadata block.
    /// Only use this when the document will be saved to a real directory that also
    /// contains `schema.usda`; in-memory / temp stages must leave this false.
    pub include_schema_sublayer: bool,
    /// When true, embed compiled schema class definitions inline in the session layer.
    /// Disabled for on-disk saves that use a physical schema sidecar instead.
    pub embed_schema_inline: bool,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        Self {
            include_schema_sublayer: false,
            embed_schema_inline: true,
        }
    }
}

/// Schema-validated empty stage used for new documents and cold startup.
pub fn blank_stage_usda() -> String {
    initial_stage_usda()
}

/// Build a conformant USDA root layer from canvas nodes and wire connections.
pub fn compose_pipeline_usda(nodes: &[VisualNode], connections: &[NodeConnection]) -> String {
    compose_pipeline_usda_with_options(nodes, connections, ComposeOptions::default())
}

/// Build USDA with optional schema sublayer metadata for on-disk saves.
pub fn compose_pipeline_usda_with_options(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
    options: ComposeOptions,
) -> String {
    let snapshot = PipelineGraphSnapshot {
        nodes: nodes.to_vec(),
        connections: connections.to_vec(),
        execution_order: Vec::new(),
        dag_valid: true,
        wiring_valid: true,
        wiring_errors: Vec::new(),
    };
    let connections = validated_connections(&snapshot);
    let paths = resolve_node_stage_paths(nodes, &connections);
    let relationships = collect_relationships(nodes, &connections, &paths);
    let forest = build_nest_forest(nodes, &paths);

    let mut out = String::from("#usda 1.0\n(\n");
    if options.include_schema_sublayer {
        out.push_str("    subLayers = [\n");
        out.push_str(&format!("        {SCHEMA_SUBLAYER_REF}\n"));
        out.push_str("    ]\n");
    }
    out.push_str(&format!("    defaultPrim = \"{MARKETLAB_DEFAULT_PRIM}\"\n)\n\n"));
    out.push_str(&format!("def Scope \"{MARKETLAB_DEFAULT_PRIM}\"\n{{\n"));
    for entry in &forest {
        write_nest_entry(
            nodes,
            entry,
            &relationships,
            1,
            &mut out,
        );
    }
    out.push_str("}\n");
    if options.embed_schema_inline && !options.include_schema_sublayer {
        embed_schema_inline_in_layer(&out)
    } else {
        out
    }
}

/// Resolve absolute stage paths for every canvas node, including portfolio nesting.
pub fn resolve_node_stage_paths(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> HashMap<usize, String> {
    let nodes_by_id: HashMap<usize, &VisualNode> =
        nodes.iter().map(|node| (node.id, node)).collect();
    let mut portfolio_paths: HashMap<usize, String> = HashMap::new();

    fn portfolio_parent_id(
        portfolio_id: usize,
        nodes_by_id: &HashMap<usize, &VisualNode>,
        connections: &[NodeConnection],
    ) -> Option<usize> {
        connections.iter().find_map(|connection| {
            if connection.from_node_id != portfolio_id {
                return None;
            }
            let to_node = nodes_by_id.get(&connection.to_node_id)?;
            to_node.node_type.is_portfolio().then_some(to_node.id)
        })
    }

    fn resolve_portfolio_path(
        portfolio_id: usize,
        nodes_by_id: &HashMap<usize, &VisualNode>,
        connections: &[NodeConnection],
        memo: &mut HashMap<usize, String>,
        visiting: &mut HashSet<usize>,
    ) -> String {
        if let Some(path) = memo.get(&portfolio_id) {
            return path.clone();
        }
        if !visiting.insert(portfolio_id) {
            return MARKETLAB_ROOT.to_string();
        }
        let node = nodes_by_id.get(&portfolio_id).expect("portfolio node");
        let leaf = portfolio_leaf_name(&node.name);
        let path = if let Some(parent_id) = portfolio_parent_id(portfolio_id, nodes_by_id, connections)
        {
            let parent_path =
                resolve_portfolio_path(parent_id, nodes_by_id, connections, memo, visiting);
            nested_prim_path(&parent_path, &leaf).unwrap_or_else(|_| format!("{MARKETLAB_ROOT}/{leaf}"))
        } else {
            nested_prim_path(MARKETLAB_ROOT, &leaf)
                .unwrap_or_else(|_| format!("{MARKETLAB_ROOT}/{leaf}"))
        };
        visiting.remove(&portfolio_id);
        memo.insert(portfolio_id, path.clone());
        path
    }

    for node in nodes {
        if node.node_type.is_portfolio() {
            let _ = resolve_portfolio_path(
                node.id,
                &nodes_by_id,
                connections,
                &mut portfolio_paths,
                &mut HashSet::new(),
            );
        }
    }

    let mut paths = HashMap::new();
    let leaves = resolve_unique_operational_leaves(
        nodes,
        connections,
        &portfolio_paths,
        &nodes_by_id,
    );
    for node in nodes {
        let path = if node.node_type.is_portfolio() {
            portfolio_paths
                .get(&node.id)
                .cloned()
                .unwrap_or_else(|| {
                    nested_prim_path(MARKETLAB_ROOT, &portfolio_leaf_name(&node.name))
                        .unwrap_or_else(|_| format!("{MARKETLAB_ROOT}/portfolio"))
                })
        } else {
            let parent = enclosing_portfolio_path(node.id, &nodes_by_id, connections, &portfolio_paths)
                .unwrap_or_else(|| MARKETLAB_ROOT.to_string());
            let leaf = leaves
                .get(&node.id)
                .cloned()
                .unwrap_or_else(|| operational_leaf_name(node));
            nested_prim_path(&parent, &leaf)
                .unwrap_or_else(|_| format!("{parent}/{leaf}"))
        };
        paths.insert(node.id, path);
    }
    paths
}

fn enclosing_portfolio_path(
    node_id: usize,
    nodes_by_id: &HashMap<usize, &VisualNode>,
    connections: &[NodeConnection],
    portfolio_paths: &HashMap<usize, String>,
) -> Option<String> {
    let mut queue = vec![node_id];
    let mut visited = HashSet::new();
    while let Some(current) = queue.pop() {
        if !visited.insert(current) {
            continue;
        }
        for connection in connections
            .iter()
            .filter(|connection| connection.from_node_id == current)
        {
            let to_id = connection.to_node_id;
            if let Some(to_node) = nodes_by_id.get(&to_id) {
                if to_node.node_type.is_portfolio() {
                    return portfolio_paths.get(&to_id).cloned();
                }
                queue.push(to_id);
            }
        }
    }
    None
}

fn portfolio_leaf_name(label: &str) -> String {
    label.trim().replace(' ', "_")
}

fn operational_leaf_name(node: &VisualNode) -> String {
    match &node.node_type {
        NodeType::AssetAdaptor { prim_path } => prim_leaf_name(prim_path).to_string(),
        NodeType::TaUberSignal { config } => config.leaf_signature(),
        NodeType::OtlShader { .. } => {
            format!("otl_{}", node.id)
        }
        NodeType::TerminalIntegrator { .. } => portfolio_leaf_name(&node.name),
    }
}

fn disambiguate_leaf(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Assign unique OTL prim leaf names within each enclosing portfolio scope.
fn resolve_unique_operational_leaves(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
    portfolio_paths: &HashMap<usize, String>,
    nodes_by_id: &HashMap<usize, &VisualNode>,
) -> HashMap<usize, String> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for node in nodes {
        if !node.node_type.is_ta_uber_signal() {
            continue;
        }
        let parent = enclosing_portfolio_path(node.id, nodes_by_id, connections, portfolio_paths)
            .unwrap_or_else(|| MARKETLAB_ROOT.to_string());
        groups.entry(parent).or_default().push(node.id);
    }

    let mut leaves = HashMap::new();
    for mut node_ids in groups.into_values() {
        node_ids.sort_unstable();
        let mut used = HashSet::new();
        for node_id in node_ids {
            let node = nodes_by_id
                .get(&node_id)
                .copied()
                .expect("otl node in scope");
            let leaf = disambiguate_leaf(
                &node
                    .node_type
                    .ta_uber_config()
                    .map(|config| config.leaf_signature())
                    .unwrap_or_else(|| operational_leaf_name(node)),
                &mut used,
            );
            leaves.insert(node_id, leaf);
        }
    }

    for node in nodes {
        if node.node_type.is_ta_uber_signal() {
            continue;
        }
        if !node.node_type.is_portfolio() {
            leaves.insert(node.id, operational_leaf_name(node));
        }
    }

    leaves
}

struct NestEntry {
    node_id: usize,
    path: String,
    children: Vec<NestEntry>,
}

fn build_nest_forest(nodes: &[VisualNode], paths: &HashMap<usize, String>) -> Vec<NestEntry> {
    let mut grouped: HashMap<String, Vec<usize>> = HashMap::new();
    for node in nodes {
        let Some(path) = paths.get(&node.id) else {
            continue;
        };
        let parent = parent_path(path).unwrap_or_else(|| MARKETLAB_ROOT.to_string());
        grouped.entry(parent).or_default().push(node.id);
    }
    for ids in grouped.values_mut() {
        ids.sort_by_key(|id| paths.get(id).cloned().unwrap_or_default());
    }
    build_nest_entries(MARKETLAB_ROOT, &grouped, paths)
}

fn build_nest_entries(
    parent_path: &str,
    grouped: &HashMap<String, Vec<usize>>,
    paths: &HashMap<usize, String>,
) -> Vec<NestEntry> {
    grouped
        .get(parent_path)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|node_id| {
            let path = paths.get(&node_id)?.clone();
            Some(NestEntry {
                node_id,
                path: path.clone(),
                children: build_nest_entries(&path, grouped, paths),
            })
        })
        .collect()
}

fn parent_path(path: &str) -> Option<String> {
    let normalized = path.trim_end_matches('/');
    let (_, leaf) = normalized.rsplit_once('/')?;
    if leaf.is_empty() {
        return None;
    }
    let parent = normalized.trim_end_matches(leaf).trim_end_matches('/');
    if parent.is_empty() {
        return None;
    }
    Some(parent.to_string())
}

fn write_nest_entry(
    nodes: &[VisualNode],
    entry: &NestEntry,
    relationships: &HashMap<String, BTreeMap<String, Vec<String>>>,
    indent: usize,
    out: &mut String,
) {
    let Some(node) = nodes.iter().find(|node| node.id == entry.node_id) else {
        return;
    };
    write_prim(
        node,
        &entry.path,
        relationships.get(&entry.path),
        &entry.children,
        nodes,
        relationships,
        indent,
        out,
    );
}

/// Compose the stage layer and write `schema.usda` + document to `document_path`.
pub fn write_pipeline_usd_document(
    document_path: &Path,
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> io::Result<()> {
    let parent = document_path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    fs::write(parent.join(SCHEMA_SIDECAR_FILENAME), schema_sidecar_usda())?;
    let usda = compose_pipeline_usda_with_options(
        nodes,
        connections,
        ComposeOptions {
            include_schema_sublayer: true,
            embed_schema_inline: false,
        },
    );
    fs::write(document_path, usda)?;
    Ok(())
}

/// Directory containing `schema.usda` when saving a document at `document_path`.
pub use pulsar_marketlab_core::schema_sidecar_directory;

pub(crate) fn stage_prim_path_for_node(node: &VisualNode) -> Option<String> {
    match &node.node_type {
        NodeType::AssetAdaptor { prim_path } => Some(prim_path.clone()),
        NodeType::TaUberSignal { config } => {
            analytics_prim_path(&config.algorithm).ok()
        }
        NodeType::OtlShader { .. } => analytics_prim_path("otl").ok(),
        NodeType::TerminalIntegrator { .. } => portfolio_prim_path(&node.name).ok(),
    }
}

pub(crate) fn stage_prim_path_for_node_resolved(
    node: &VisualNode,
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> Option<String> {
    resolve_node_stage_paths(nodes, connections)
        .get(&node.id)
        .cloned()
        .or_else(|| stage_prim_path_for_node(node))
}

fn prim_leaf_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn schema_type_for_node(node: &VisualNode) -> &'static str {
    match &node.node_type {
        NodeType::AssetAdaptor { .. } => "FinancialAsset",
        NodeType::TaUberSignal { .. } => "OtlTaUberSignal",
        NodeType::OtlShader { .. } => "OtlOperator",
        NodeType::TerminalIntegrator { .. } => "PortfolioIntegrator",
    }
}

fn otl_script_src(node: &VisualNode) -> String {
    resolved_otl_script(node)
}

fn collect_relationships(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
    paths: &HashMap<usize, String>,
) -> HashMap<String, BTreeMap<String, Vec<String>>> {
    let nodes_by_id: HashMap<usize, &VisualNode> =
        nodes.iter().map(|node| (node.id, node)).collect();
    let mut out: HashMap<String, BTreeMap<String, Vec<String>>> = HashMap::new();

    for connection in connections {
        let Some(from_node) = nodes_by_id.get(&connection.from_node_id) else {
            continue;
        };
        let Some(to_node) = nodes_by_id.get(&connection.to_node_id) else {
            continue;
        };
        let Some(source_path) = paths.get(&from_node.id).cloned() else {
            continue;
        };
        let Some(target_path) = paths.get(&to_node.id).cloned() else {
            continue;
        };
        let relationship = match (&from_node.node_type, &to_node.node_type) {
            (NodeType::AssetAdaptor { .. }, NodeType::OtlShader { .. })
            | (NodeType::AssetAdaptor { .. }, NodeType::TaUberSignal { .. }) => "inputs:underlying",
            (NodeType::AssetAdaptor { .. }, NodeType::TerminalIntegrator { .. })
                if to_node.node_type.is_portfolio() =>
            {
                "inputs:sources"
            }
            (NodeType::OtlShader { .. }, NodeType::TerminalIntegrator { .. })
            | (NodeType::TaUberSignal { .. }, NodeType::TerminalIntegrator { .. }) => {
                "inputs:sources"
            }
            (NodeType::OtlShader { .. }, NodeType::OtlShader { .. })
            | (NodeType::OtlShader { .. }, NodeType::TaUberSignal { .. })
            | (NodeType::TaUberSignal { .. }, NodeType::OtlShader { .. })
            | (NodeType::TaUberSignal { .. }, NodeType::TaUberSignal { .. }) => "inputs:underlying",
            (NodeType::TerminalIntegrator { .. }, NodeType::TerminalIntegrator { .. })
                if from_node.node_type.is_portfolio() && to_node.node_type.is_portfolio() =>
            {
                "inputs:sources"
            }
            _ => continue,
        };
        out.entry(target_path)
            .or_default()
            .entry(relationship.to_string())
            .or_default()
            .push(source_path);
    }

    out
}

fn write_prim(
    node: &VisualNode,
    path: &str,
    relationships: Option<&BTreeMap<String, Vec<String>>>,
    children: &[NestEntry],
    nodes: &[VisualNode],
    all_relationships: &HashMap<String, BTreeMap<String, Vec<String>>>,
    indent: usize,
    out: &mut String,
) {
    let pad = "    ".repeat(indent);
    let inner = "    ".repeat(indent + 1);
    let leaf = prim_leaf_name(path);
    let schema_type = schema_type_for_node(node);
    out.push_str(&format!("{pad}def {schema_type} \"{leaf}\"\n{pad}{{\n"));

    match &node.node_type {
        NodeType::AssetAdaptor { .. } => {
            let symbol = prim_leaf_name(path);
            write_bool(out, &inner, "inputs:active", true);
            write_token(out, &inner, "inputs:symbol", symbol);
            write_token(out, &inner, "inputs:provider", "yahoo");
            if matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) {
                write_token(out, &inner, "inputs:asset_class", "Equity");
            }
        }
        NodeType::TaUberSignal { config } => {
            write_token(out, &inner, "info:archetype", config.archetype.as_token());
            write_token(out, &inner, "info:algorithm", &config.algorithm);
            write_int(out, &inner, "inputs:period", config.period as i32);
            write_int(out, &inner, "inputs:signal_period", config.signal_period as i32);
            write_float(out, &inner, "inputs:multiplier", config.multiplier);
            write_float(out, &inner, "inputs:annualization", config.annualization);
            write_string(
                out,
                &inner,
                "inputs:script_src",
                &compose_uber_script_src(config),
            );
        }
        NodeType::OtlShader {
            compiled_path, ..
        } => {
            write_string(out, &inner, "inputs:script_src", &otl_script_src(node));
            if let Some(path) = compiled_path.as_deref().filter(|p| !p.is_empty()) {
                write_string(out, &inner, "inputs:script_compiled_path", path);
            }
        }
        NodeType::TerminalIntegrator { .. } if node.node_type.is_portfolio() => {
            let allocation = node
                .portfolio_allocation_id
                .as_deref()
                .unwrap_or("Allocation::HierarchicalRiskParity");
            write_token(out, &inner, "inputs:id", allocation);
            write_double(out, &inner, "inputs:initial_capital", crate::workspace_state::SIM_INITIAL_CASH);
            write_token(out, &inner, "inputs:rebalance_frequency", "monthly");
        }
        NodeType::TerminalIntegrator { engine_target } => {
            write_token(out, &inner, "inputs:id", engine_target);
        }
    }

    write_ui_canvas_pos(out, &inner, node.x, node.y);

    if let Some(rels) = relationships {
        for (relationship, targets) in rels {
            write_relationship(out, &inner, relationship, targets);
        }
    }

    for child in children {
        write_nest_entry(nodes, child, all_relationships, indent + 1, out);
    }

    out.push_str(&format!("{pad}}}\n"));
}

fn write_ui_canvas_pos(out: &mut String, indent: &str, x: f32, y: f32) {
    out.push_str(&format!("{indent}custom float2 ui:canvas:pos = ({x}, {y})\n"));
}

fn write_token(out: &mut String, indent: &str, name: &str, value: &str) {
    let escaped = value.replace('"', "\\\"");
    out.push_str(&format!("{indent}token {name} = \"{escaped}\"\n"));
}

fn write_string(out: &mut String, indent: &str, name: &str, value: &str) {
    let escaped = value.replace('"', "\\\"");
    out.push_str(&format!("{indent}string {name} = \"{escaped}\"\n"));
}

fn write_bool(out: &mut String, indent: &str, name: &str, value: bool) {
    out.push_str(&format!(
        "{indent}bool {name} = {}\n",
        if value { "1" } else { "0" }
    ));
}

fn write_double(out: &mut String, indent: &str, name: &str, value: f64) {
    out.push_str(&format!("{indent}double {name} = {value}\n"));
}

fn write_int(out: &mut String, indent: &str, name: &str, value: i32) {
    out.push_str(&format!("{indent}int {name} = {value}\n"));
}

fn write_float(out: &mut String, indent: &str, name: &str, value: f32) {
    out.push_str(&format!("{indent}float {name} = {value}\n"));
}

fn write_relationship(out: &mut String, indent: &str, name: &str, targets: &[String]) {
    if targets.is_empty() {
        return;
    }
    if targets.len() == 1 {
        out.push_str(&format!("{indent}rel {name} = <{}>\n", targets[0]));
    } else {
        out.push_str(&format!("{indent}rel {name} = [\n"));
        for target in targets {
            out.push_str(&format!("{indent}    <{target}>,\n"));
        }
        out.push_str(&format!("{indent}]\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_compiler::{
        apply_canonical_ta_ports, connection_is_valid, ta_uber_from_legacy_indicator,
        NodeGradeType,
    };
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
    fn composed_stage_uses_schema_types_not_xform() {
        let nodes = vec![sample_asset(1), sample_ta(2)];
        let connections = vec![NodeConnection {
            from_node_id: 1,
            from_port_idx: 0,
            to_node_id: 2,
            to_port_idx: 0,
        }];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        let usda = compose_pipeline_usda(&nodes, &connections);
        assert!(usda.contains("def Scope \"MarketLab\""));
        assert!(usda.contains("def FinancialAsset \"GLD\""));
        assert!(usda.contains("def OtlTaUberSignal \"oscillator_rsi_14\""));
        assert!(!usda.contains("def Xform"));
        assert!(usda.contains("token inputs:symbol = \"GLD\""));
        assert!(usda.contains("string inputs:script_src = \"ta::rsi(input, 14)\""));
        assert!(usda.contains("token info:archetype = \"oscillator\""));
        assert!(usda.contains(&format!("rel inputs:underlying = <{}>", paths.get(&1).expect("asset"))));
        assert!(usda.contains("custom float2 ui:canvas:pos = (450, 320)"));
    }

    #[test]
    fn composed_stage_exposes_canvas_prims() {
        let nodes = vec![sample_asset(1), sample_ta(2)];
        let connections = vec![NodeConnection {
            from_node_id: 1,
            from_port_idx: 0,
            to_node_id: 2,
            to_port_idx: 0,
        }];
        let usda = compose_pipeline_usda(&nodes, &connections);
        let bridge = UsdStageBridge::open_from_usda_text(&usda).expect("parse composed stage");
        let rows = bridge.stage_prim_rows().expect("list prims");
        assert!(rows.iter().any(|row| row.path.ends_with("/GLD") || row.path == "/MarketLab/GLD"));
        assert!(rows.iter().any(|row| {
            row.path.ends_with("/oscillator_rsi_14") || row.path == "/MarketLab/oscillator_rsi_14"
        }));
        assert!(!rows.iter().any(|row| row.path == "/FinancialAsset"));
    }

    #[test]
    fn in_memory_compose_embeds_schema_inline_without_sublayer() {
        let usda = compose_pipeline_usda(&[], &[]);
        assert!(!usda.contains("subLayers"));
        assert!(usda.contains("class \"FinancialAsset\""));
        assert!(usda.contains("def Scope \"MarketLab\""));
    }

    #[test]
    fn disk_compose_includes_schema_sublayer_not_inline_classes() {
        let usda = compose_pipeline_usda_with_options(
            &[],
            &[],
            ComposeOptions {
                include_schema_sublayer: true,
                embed_schema_inline: false,
            },
        );
        assert!(usda.contains("subLayers"));
        assert!(usda.contains(SCHEMA_SUBLAYER_REF));
        assert!(!usda.contains("class \"FinancialAsset\""));
    }

    #[test]
    fn write_pipeline_document_resolves_schema_sublayer() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "marketlab_compose_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let doc = dir.join("stage.usda");
        let nodes = vec![sample_asset(1)];
        write_pipeline_usd_document(&doc, &nodes, &[]).expect("write document");
        assert!(dir.join(SCHEMA_SIDECAR_FILENAME).is_file());
        UsdStageBridge::open(&doc).expect("open saved stage with schema sidecar");
        let _ = fs::remove_dir_all(&dir);
    }

    fn sample_portfolio(id: usize, name: &str) -> VisualNode {
        VisualNode {
            id,
            name: name.to_string(),
            node_type: NodeType::portfolio(),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: Some("Allocation::HierarchicalRiskParity".to_string()),
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 800.0,
            y: 120.0,
            collapsed: false,
            inputs: vec!["Signal In 0".to_string(), "Signal In 1".to_string()],
            outputs: vec!["Portfolio Out".to_string()],
        }
    }

    fn sample_named_asset(id: usize, symbol: &str) -> VisualNode {
        VisualNode {
            id,
            name: format!("{symbol}.csv"),
            node_type: NodeType::asset_adaptor(format!("/MarketLab/{symbol}")),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 120.0,
            y: 80.0,
            collapsed: false,
            inputs: vec![],
            outputs: vec!["Close Out".to_string()],
        }
    }

    fn sample_named_ta(id: usize, indicator: &str, lookback: u32) -> VisualNode {
        let mut node = VisualNode {
            id,
            name: indicator.to_string(),
            node_type: NodeType::ta_uber_signal(ta_uber_from_legacy_indicator(indicator, lookback)),
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
    fn nested_portfolio_stage_matches_macro_fund_layout() {
        let nodes = vec![
            sample_portfolio(1, "Master_Macro_Fund"),
            sample_portfolio(2, "Equity_Sub_Book"),
            sample_named_asset(3, "SPY"),
            sample_named_ta(4, "sma", 14),
            sample_named_asset(5, "GLD"),
            sample_named_ta(6, "rsi", 14),
        ];
        let connections = vec![
            NodeConnection {
                from_node_id: 2,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 3,
                from_port_idx: 0,
                to_node_id: 4,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 4,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 5,
                from_port_idx: 0,
                to_node_id: 6,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 6,
                from_port_idx: 0,
                to_node_id: 2,
                to_port_idx: 0,
            },
        ];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        assert_eq!(
            paths.get(&1).map(String::as_str),
            Some("/MarketLab/Master_Macro_Fund")
        );
        assert_eq!(
            paths.get(&2).map(String::as_str),
            Some("/MarketLab/Master_Macro_Fund/Equity_Sub_Book")
        );
        assert_eq!(
            paths.get(&3).map(String::as_str),
            Some("/MarketLab/Master_Macro_Fund/SPY")
        );
        assert_eq!(
            paths.get(&5).map(String::as_str),
            Some("/MarketLab/Master_Macro_Fund/Equity_Sub_Book/GLD")
        );

        let usda = compose_pipeline_usda(&nodes, &connections);
        assert!(usda.contains("def PortfolioIntegrator \"Master_Macro_Fund\""));
        assert!(usda.contains("def PortfolioIntegrator \"Equity_Sub_Book\""));
        assert!(usda.contains("def FinancialAsset \"SPY\""));
        assert!(usda.contains("def FinancialAsset \"GLD\""));
        assert!(usda.contains("def OtlTaUberSignal \"trend_sma_14\""));
        assert!(usda.contains("def OtlTaUberSignal \"oscillator_rsi_14\""));

        let master_pos = usda
            .find("def PortfolioIntegrator \"Master_Macro_Fund\"")
            .expect("master fund prim");
        let equity_pos = usda
            .find("def PortfolioIntegrator \"Equity_Sub_Book\"")
            .expect("nested sub-book prim");
        let spy_pos = usda.find("def FinancialAsset \"SPY\"").expect("SPY prim");
        let gld_pos = usda.find("def FinancialAsset \"GLD\"").expect("GLD prim");
        let gld_rsi_pos = usda
            .find("def OtlTaUberSignal \"oscillator_rsi_14\"")
            .expect("rsi prim");
        assert!(master_pos < equity_pos);
        assert!(master_pos < spy_pos);
        assert!(equity_pos < gld_pos && gld_pos < gld_rsi_pos);

        assert!(usda.contains(
            "rel inputs:sources = </MarketLab/Master_Macro_Fund/Equity_Sub_Book/oscillator_rsi_14>"
        ));
        assert!(usda.contains("</MarketLab/Master_Macro_Fund/trend_sma_14>"));
        assert!(usda.contains("</MarketLab/Master_Macro_Fund/Equity_Sub_Book>"));
    }

    #[test]
    fn duplicate_ta_type_in_portfolio_gets_unique_stage_paths() {
        let nodes = vec![
            sample_portfolio(1, "Alpha_Fund"),
            sample_named_ta(2, "sma", 14),
            sample_named_ta(3, "sma", 20),
            sample_named_ta(4, "sma", 14),
        ];
        let connections = vec![
            NodeConnection {
                from_node_id: 2,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 3,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 1,
            },
            NodeConnection {
                from_node_id: 4,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 2,
            },
        ];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        assert_eq!(
            paths.get(&2).map(String::as_str),
            Some("/MarketLab/Alpha_Fund/trend_sma_14")
        );
        assert_eq!(
            paths.get(&3).map(String::as_str),
            Some("/MarketLab/Alpha_Fund/trend_sma_20")
        );
        assert_eq!(
            paths.get(&4).map(String::as_str),
            Some("/MarketLab/Alpha_Fund/trend_sma_14_2")
        );

        let usda = compose_pipeline_usda(&nodes, &[]);
        assert!(usda.contains("def OtlTaUberSignal \"trend_sma_14\""));
        assert!(usda.contains("def OtlTaUberSignal \"trend_sma_20\""));
        assert!(usda.contains("def OtlTaUberSignal \"trend_sma_14_2\""));
    }

    #[test]
    fn buy_and_hold_asset_wires_directly_to_portfolio() {
        let nodes = vec![
            sample_portfolio(1, "Buy_Hold_Fund"),
            sample_named_asset(2, "SPY"),
        ];
        let connections = vec![NodeConnection {
            from_node_id: 2,
            from_port_idx: 0,
            to_node_id: 1,
            to_port_idx: 0,
        }];
        assert!(connection_is_valid(
            &nodes[1],
            0,
            &nodes[0],
            0
        ));
        let usda = compose_pipeline_usda(&nodes, &connections);
        assert!(usda.contains(
            "rel inputs:sources = </MarketLab/Buy_Hold_Fund/SPY>"
        ));
    }

    #[test]
    fn graph_engine_executes_nested_portfolio_chain() {
        use pulsar_marketlab_core::MarketLabGraphEngine;
        use pulsar_marketlab_ui::workspace::{build_stage_graph_snapshot, WorkspaceContext};

        let nodes = vec![
            sample_portfolio(1, "Sim Portfolio 1"),
            sample_named_asset(2, "SPY"),
            sample_named_ta(3, "sma", 10),
        ];
        let connections = vec![
            NodeConnection {
                from_node_id: 2,
                from_port_idx: 0,
                to_node_id: 3,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 3,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 0,
            },
        ];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        let portfolio_path = paths.get(&1).expect("portfolio path");
        let usda = compose_pipeline_usda(&nodes, &connections);
        let context =
            WorkspaceContext::from_usda_text(&usda).expect("workspace context from composed usda");
        let snapshot = build_stage_graph_snapshot(context.usd_stage());
        let engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("graph engine compile");
        let prices: Vec<f64> = (0..22).map(|i| 300.0 + i as f64).collect();
        let asset_vectors = std::collections::HashMap::from([("SPY".to_string(), prices)]);
        let result = engine.execute_timeline(&asset_vectors, 22);
        assert!(
            result
                .streams
                .iter()
                .any(|stream| {
                    stream.prim_path == *portfolio_path
                        && stream.attribute == "outputs:portfolio_wealth"
                }),
            "expected portfolio wealth stream at {portfolio_path}, got streams: {:?}",
            result.streams.iter().map(|s| (&s.prim_path, &s.attribute)).collect::<Vec<_>>()
        );
        assert!(
            result.portfolio_results.contains_key(portfolio_path),
            "expected portfolio_results for {portfolio_path}"
        );
    }

    /// Regression for `marketlab_stage_portfolio_nodes.usda`: three nested portfolio tiers
    /// must compile, execute, and produce rising NAV when underlying assets trend up.
    #[test]
    fn graph_engine_executes_manual_nested_portfolio_stage() {
        use pulsar_marketlab_core::MarketLabGraphEngine;
        use pulsar_marketlab_ui::workspace::{build_stage_graph_snapshot, WorkspaceContext};

        const STAGE: &str = r#"#usda 1.0
(
    defaultPrim = "MarketLab"
)

def Scope "MarketLab"
{
    def PortfolioIntegrator "Sim_Portfolio_3"
    {
        token inputs:id = "Allocation::HierarchicalRiskParity"
        double inputs:initial_capital = 10000
        rel inputs:sources = [
            </MarketLab/Sim_Portfolio_3/Sim_Portfolio_2>,
            </MarketLab/Sim_Portfolio_3/Sim_Portfolio_1>,
        ]
        def PortfolioIntegrator "Sim_Portfolio_1"
        {
            token inputs:id = "Allocation::HierarchicalRiskParity"
            double inputs:initial_capital = 10000
            rel inputs:sources = [
                </MarketLab/Sim_Portfolio_3/Sim_Portfolio_1/QQQ>,
                </MarketLab/Sim_Portfolio_3/Sim_Portfolio_1/SPY>,
            ]
            def FinancialAsset "QQQ"
            {
                token inputs:symbol = "QQQ"
            }
            def FinancialAsset "SPY"
            {
                token inputs:symbol = "SPY"
            }
        }
        def PortfolioIntegrator "Sim_Portfolio_2"
        {
            token inputs:id = "Allocation::MeanVariance"
            double inputs:initial_capital = 10000
            rel inputs:sources = [
                </MarketLab/Sim_Portfolio_3/Sim_Portfolio_2/AGG>,
                </MarketLab/Sim_Portfolio_3/Sim_Portfolio_2/TMF>,
            ]
            def FinancialAsset "AGG"
            {
                token inputs:symbol = "AGG"
            }
            def FinancialAsset "TMF"
            {
                token inputs:symbol = "TMF"
            }
        }
    }
}
"#;

        let context =
            WorkspaceContext::from_usda_text(STAGE).expect("workspace context from manual stage");
        let snapshot = build_stage_graph_snapshot(context.usd_stage());
        let engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("graph engine compile");

        let bars = 22;
        let trend = |base: f64| -> Vec<f64> {
            (0..bars).map(|i| base + i as f64 * 0.5).collect()
        };
        let asset_vectors = std::collections::HashMap::from([
            ("QQQ".to_string(), trend(400.0)),
            ("SPY".to_string(), trend(500.0)),
            ("AGG".to_string(), trend(90.0)),
            ("TMF".to_string(), trend(15.0)),
        ]);

        let result = engine.execute_timeline(&asset_vectors, bars);

        let book_one = "/MarketLab/Sim_Portfolio_3/Sim_Portfolio_1";
        let book_two = "/MarketLab/Sim_Portfolio_3/Sim_Portfolio_2";
        let master = "/MarketLab/Sim_Portfolio_3";

        for path in [book_one, book_two, master] {
            let integration = result
                .portfolio_results
                .get(path)
                .unwrap_or_else(|| panic!("missing portfolio_results for {path}"));
            assert_eq!(
                integration.wealth_series.len(),
                bars,
                "wealth series length for {path}"
            );
            let first = integration.wealth_series.first().copied().unwrap_or(0.0);
            let last = integration.wealth_series.last().copied().unwrap_or(0.0);
            assert!(
                last > first + 1.0,
                "expected rising NAV at {path}, got first={first} last={last}"
            );
            assert!(
                !integration.tracking_matrix.is_empty(),
                "expected tracking rows for {path}"
            );
        }

        let master_last = result.portfolio_results[master].wealth_series.last().copied().unwrap();
        let blend = result.portfolio_results[book_one].wealth_series.last().copied().unwrap()
            * 0.5
            + result.portfolio_results[book_two].wealth_series.last().copied().unwrap() * 0.5;
        assert!(
            (master_last - blend).abs() < 50.0,
            "master NAV should approximate 50/50 child blend: master={master_last} blend={blend}"
        );
    }
}
