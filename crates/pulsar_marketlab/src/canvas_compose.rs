//! Compose an OpenUSD root layer from the visual node canvas graph.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::graph_compiler::{AssetSourceType, NodeConnection, NodeType, VisualNode};
use pulsar_marketlab::trading_stage::{
    analytics_prim_path, nested_prim_path, portfolio_prim_path, MARKETLAB_DEFAULT_PRIM,
    MARKETLAB_ROOT,
};
use pulsar_marketlab_core::{
    embed_schema_inline_in_layer, initial_stage_usda, schema_sidecar_usda,
    SCHEMA_SIDECAR_FILENAME, SCHEMA_SUBLAYER_REF,
};

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
    let paths = resolve_node_stage_paths(nodes, connections);
    let relationships = collect_relationships(nodes, connections, &paths);
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
        NodeType::OtlShader { .. } => ta_leaf_signature(node),
        NodeType::TerminalIntegrator { .. } => portfolio_leaf_name(&node.name),
    }
}

fn ta_leaf_signature(node: &VisualNode) -> String {
    let indicator = node
        .ta_indicator_id
        .as_deref()
        .unwrap_or("rsi");
    format!("{indicator}_{}", node.ta_lookback_period)
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
        if !node.node_type.is_otl_shader() {
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
            let leaf = disambiguate_leaf(&ta_leaf_signature(node), &mut used);
            leaves.insert(node_id, leaf);
        }
    }

    for node in nodes {
        if node.node_type.is_otl_shader() {
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
        NodeType::OtlShader { .. } => {
            let indicator_id = node.ta_indicator_id.as_deref().unwrap_or("rsi");
            analytics_prim_path(indicator_id).ok()
        }
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
        NodeType::OtlShader { .. } => "OtlOperator",
        NodeType::TerminalIntegrator { .. } => "PortfolioIntegrator",
    }
}

fn otl_script_src(node: &VisualNode) -> String {
    if let Some(formula) = node
        .dsl_formula
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return formula.to_string();
    }
    if let Some(script) = node.node_type.script().filter(|text| !text.is_empty()) {
        return script.to_string();
    }
    let indicator_id = node.ta_indicator_id.as_deref().unwrap_or("rsi");
    format!("{indicator_id}(period={})", node.ta_lookback_period)
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
            (NodeType::AssetAdaptor { .. }, NodeType::OtlShader { .. }) => "inputs:underlying",
            (NodeType::AssetAdaptor { .. }, NodeType::TerminalIntegrator { .. })
                if to_node.node_type.is_portfolio() =>
            {
                "inputs:sources"
            }
            (NodeType::OtlShader { .. }, NodeType::TerminalIntegrator { .. }) => "inputs:sources",
            (NodeType::OtlShader { .. }, NodeType::OtlShader { .. }) => "inputs:underlying",
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
        NodeType::OtlShader { .. } => {
            let indicator_id = node.ta_indicator_id.as_deref().unwrap_or("rsi");
            write_token(out, &inner, "inputs:id", indicator_id);
            write_string(out, &inner, "inputs:script_src", &otl_script_src(node));
        }
        NodeType::TerminalIntegrator { .. } if node.node_type.is_portfolio() => {
            let allocation = node
                .portfolio_allocation_id
                .as_deref()
                .unwrap_or("Allocation::HierarchicalRiskParity");
            write_token(out, &inner, "inputs:id", allocation);
            write_double(out, &inner, "inputs:initial_capital", 10_000_000.0);
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
    use crate::graph_compiler::{connection_is_valid, NodeGradeType};
    use pulsar_marketlab::stage_bridge::UsdStageBridge;
    use pulsar_marketlab::technical_analysis::DEFAULT_TA_INDICATOR_ID;

    fn sample_asset(id: usize) -> VisualNode {
        VisualNode {
            id,
            name: "GLD.csv".to_string(),
            node_type: NodeType::asset_adaptor("/MarketLab/GLD".to_string()),
            grade: NodeGradeType::Scalar,
            ta_indicator_id: None,
            ta_lookback_period: 14,
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
        VisualNode {
            id,
            name: "RSI".to_string(),
            node_type: NodeType::otl_shader(String::new()),
            grade: NodeGradeType::Scalar,
            ta_indicator_id: Some(DEFAULT_TA_INDICATOR_ID.to_string()),
            ta_lookback_period: 14,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 450.0,
            y: 320.0,
            collapsed: false,
            inputs: vec!["Price In".to_string()],
            outputs: vec!["TA Out".to_string()],
        }
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
        assert!(usda.contains("def OtlOperator \"rsi_14\""));
        assert!(!usda.contains("def Xform"));
        assert!(usda.contains("token inputs:symbol = \"GLD\""));
        assert!(usda.contains("string inputs:script_src = \"rsi(period=14)\""));
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
        assert!(rows.iter().any(|row| row.path.ends_with("/rsi_14") || row.path == "/MarketLab/rsi_14"));
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
            ta_indicator_id: None,
            ta_lookback_period: 14,
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
            ta_indicator_id: None,
            ta_lookback_period: 14,
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
        VisualNode {
            id,
            name: indicator.to_string(),
            node_type: NodeType::otl_shader(String::new()),
            grade: NodeGradeType::Scalar,
            ta_indicator_id: Some(indicator.to_string()),
            ta_lookback_period: lookback,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 450.0,
            y: 320.0,
            collapsed: false,
            inputs: vec!["Price In".to_string()],
            outputs: vec!["TA Out".to_string()],
        }
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
        assert!(usda.contains("def OtlOperator \"sma_14\""));
        assert!(usda.contains("def OtlOperator \"rsi_14\""));

        let master_pos = usda
            .find("def PortfolioIntegrator \"Master_Macro_Fund\"")
            .expect("master fund prim");
        let equity_pos = usda
            .find("def PortfolioIntegrator \"Equity_Sub_Book\"")
            .expect("nested sub-book prim");
        let spy_pos = usda.find("def FinancialAsset \"SPY\"").expect("SPY prim");
        let gld_pos = usda.find("def FinancialAsset \"GLD\"").expect("GLD prim");
        let gld_rsi_pos = usda
            .find("def OtlOperator \"rsi_14\"")
            .expect("rsi prim");
        assert!(master_pos < equity_pos);
        assert!(master_pos < spy_pos);
        assert!(equity_pos < gld_pos && gld_pos < gld_rsi_pos);

        assert!(usda.contains(
            "rel inputs:sources = </MarketLab/Master_Macro_Fund/Equity_Sub_Book/rsi_14>"
        ));
        assert!(usda.contains("</MarketLab/Master_Macro_Fund/sma_14>"));
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
            Some("/MarketLab/Alpha_Fund/sma_14")
        );
        assert_eq!(
            paths.get(&3).map(String::as_str),
            Some("/MarketLab/Alpha_Fund/sma_20")
        );
        assert_eq!(
            paths.get(&4).map(String::as_str),
            Some("/MarketLab/Alpha_Fund/sma_14_2")
        );

        let usda = compose_pipeline_usda(&nodes, &[]);
        assert!(usda.contains("def OtlOperator \"sma_14\""));
        assert!(usda.contains("def OtlOperator \"sma_20\""));
        assert!(usda.contains("def OtlOperator \"sma_14_2\""));
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
}
