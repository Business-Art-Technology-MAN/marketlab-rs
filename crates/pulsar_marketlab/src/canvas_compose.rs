//! Compose an OpenUSD root layer from the visual node canvas graph.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use crate::graph_compiler::{
    resolved_otl_script, validated_connections, AssetSourceType, NodeConnection, NodeType,
    PipelineGraphSnapshot, VisualNode,
};
use pulsar_marketlab_core::compose_uber_script_src;
use pulsar_marketlab::trading_stage::{
    analytics_prim_path, portfolio_prim_path, MARKETLAB_DEFAULT_PRIM,
    MARKETLAB_ROOT,
};
use crate::graph_compiler::semantic_prim_leaf_for;
use pulsar_marketlab_core::{
    compile_object_program, embed_schema_inline_in_layer, initial_stage_usda,
    schema_sidecar_usda, OtlObjectDeclaration, OtlObjectKind, FrontendError,
    PORTFOLIOS_SCOPE, SESSION_SUBLAYER_REF, SIGNALS_SCOPE, SIGNALS_SUBLAYER_REF,
    SP500_SUBLAYER_REF, UNIVERSE_SCOPE, USER_LABEL_ATTR, SCHEMA_SIDECAR_FILENAME,
    SCHEMA_SUBLAYER_REF,
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
    /// When true, also reference `@./metadata_library.usda@` beside the document.
    pub include_metadata_sublayer: bool,
    /// When true, embed compiled schema class definitions inline in the session layer.
    /// Disabled for on-disk saves that use a physical schema sidecar instead.
    pub embed_schema_inline: bool,
    /// When true, reference the workstation `session` / `signals` / `sp500_universe` sublayers.
    pub include_workstation_layer_stack: bool,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        Self {
            include_schema_sublayer: false,
            include_metadata_sublayer: false,
            embed_schema_inline: true,
            include_workstation_layer_stack: false,
        }
    }
}

/// In-memory compose options for unit tests (no unresolved workstation sublayer refs).
pub fn test_compose_options() -> ComposeOptions {
    ComposeOptions {
        include_workstation_layer_stack: false,
        ..ComposeOptions::default()
    }
}

/// Stable absolute prim path for a canvas node id under a workstation scope bucket.
pub fn workstation_stable_path(scope: &str, node_id: usize) -> String {
    format!("{MARKETLAB_ROOT}/{scope}/node_{:08x}", node_id as u32)
}

/// Compose USDA for tests and inline `WorkspaceContext::from_usda_text` round-trips.
pub fn compose_pipeline_usda_for_tests(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> String {
    compose_pipeline_usda_with_options(nodes, connections, test_compose_options())
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

    let mut out = String::from("#usda 1.0\n(\n");
    if options.include_workstation_layer_stack
        || options.include_schema_sublayer
        || options.include_metadata_sublayer
    {
        out.push_str("    subLayers = [\n");
        if options.include_workstation_layer_stack {
            out.push_str(&format!("        {SESSION_SUBLAYER_REF}\n"));
            out.push_str(&format!("        {SIGNALS_SUBLAYER_REF}\n"));
            out.push_str(&format!("        {SP500_SUBLAYER_REF}\n"));
        }
        if options.include_schema_sublayer {
            out.push_str(&format!("        {SCHEMA_SUBLAYER_REF}\n"));
        }
        if options.include_metadata_sublayer {
            out.push_str(&format!(
                "        {}\n",
                pulsar_marketlab_core::METADATA_SUBLAYER_REF
            ));
        }
        out.push_str("    ]\n");
    }
    out.push_str(&format!("    defaultPrim = \"{MARKETLAB_DEFAULT_PRIM}\"\n)\n\n"));
    write_hierarchical_marketlab_scope(
        nodes,
        &paths,
        &relationships,
        &mut out,
    );
    if options.embed_schema_inline && !options.include_schema_sublayer {
        embed_schema_inline_in_layer(&out)
    } else {
        out
    }
}

/// Resolve absolute stage paths using **immutable** prim leaves under hierarchy scopes.
pub fn resolve_node_stage_paths(
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> HashMap<usize, String> {
    let mut paths = HashMap::new();
    let mut used_leaves = HashSet::new();
    for node in nodes {
        let mut leaf = semantic_prim_leaf_for(node, nodes, connections);
        if !used_leaves.insert(leaf.clone()) {
            let mut suffix = 1u32;
            loop {
                let candidate = format!("{leaf}_{suffix:02}");
                if used_leaves.insert(candidate.clone()) {
                    leaf = candidate;
                    break;
                }
                suffix += 1;
            }
        }
        let scope = hierarchy_scope_for_node(node);
        let path = format!("{MARKETLAB_ROOT}/{scope}/{leaf}");
        paths.insert(node.id, path);
    }
    paths
}

fn hierarchy_scope_for_node(node: &VisualNode) -> &'static str {
    if node.node_type.is_asset_adaptor() {
        UNIVERSE_SCOPE
    } else if node.node_type.is_portfolio() {
        PORTFOLIOS_SCOPE
    } else {
        SIGNALS_SCOPE
    }
}

fn write_hierarchical_marketlab_scope(
    nodes: &[VisualNode],
    paths: &HashMap<usize, String>,
    relationships: &HashMap<String, BTreeMap<String, Vec<String>>>,
    out: &mut String,
) {
    out.push_str(&format!("def Scope \"{MARKETLAB_DEFAULT_PRIM}\"\n{{\n"));
    for scope in [UNIVERSE_SCOPE, SIGNALS_SCOPE, PORTFOLIOS_SCOPE] {
        out.push_str("    def Scope \"");
        out.push_str(scope);
        out.push_str("\"\n    {\n");
        let mut scope_nodes: Vec<&VisualNode> = nodes
            .iter()
            .filter(|node| hierarchy_scope_for_node(node) == scope)
            .collect();
        scope_nodes.sort_by_key(|node| node.id);
        for node in scope_nodes {
            let Some(path) = paths.get(&node.id) else {
                continue;
            };
            write_prim(
                node,
                path,
                relationships.get(path),
                &[] as &[()],
                nodes,
                relationships,
                2,
                out,
            );
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n");
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
    fs::write(
        parent.join(pulsar_marketlab_core::METADATA_LIBRARY_SIDECAR_FILENAME),
        pulsar_marketlab_core::METADATA_LIBRARY_USDA,
    )?;
    fs::write(
        parent.join(pulsar_marketlab_core::SESSION_LAYER_FILENAME),
        pulsar_marketlab_core::session_layer_usda(),
    )?;
    fs::write(
        parent.join(pulsar_marketlab_core::SIGNALS_LAYER_FILENAME),
        pulsar_marketlab_core::signals_layer_usda(),
    )?;
    fs::write(
        parent.join(pulsar_marketlab_core::FINANCE_DATABASE_EQUITIES_LAYER_FILENAME),
        pulsar_marketlab_core::finance_database_equities_empty_layer_usda(),
    )?;
    fs::write(
        parent.join(pulsar_marketlab_core::SP500_UNIVERSE_LAYER_FILENAME),
        pulsar_marketlab_core::sp500_universe_layer_usda(),
    )?;
    let usda = compose_pipeline_usda_with_options(
        nodes,
        connections,
        ComposeOptions {
            include_schema_sublayer: true,
            include_metadata_sublayer: true,
            embed_schema_inline: false,
            include_workstation_layer_stack: true,
            ..ComposeOptions::default()
        },
    );
    fs::write(document_path, usda)?;
    Ok(())
}


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

pub fn prim_schema_type_name(node: &VisualNode) -> &'static str {
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

pub fn collect_relationships(
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
    _children: &[()],
    _nodes: &[VisualNode],
    _all_relationships: &HashMap<String, BTreeMap<String, Vec<String>>>,
    indent: usize,
    out: &mut String,
) {
    let pad = "    ".repeat(indent);
    let inner = "    ".repeat(indent + 1);
    let leaf = prim_leaf_name(path);
    let schema_type = prim_schema_type_name(node);
    out.push_str(&format!("{pad}def {schema_type} \"{leaf}\"\n{pad}{{\n"));

    match &node.node_type {
        NodeType::AssetAdaptor { prim_path } => {
            let symbol = prim_leaf_name(prim_path);
            let declared_class = node
                .asset_source
                .as_ref()
                .map(|_| "Equity");
            let taxonomy =
                pulsar_marketlab_core::flatten_asset_metadata(symbol, declared_class);
            write_bool(out, &inner, "inputs:active", true);
            write_token(out, &inner, "inputs:symbol", symbol);
            write_token(out, &inner, "inputs:asset_class", &taxonomy.asset_class);
            write_token(out, &inner, "inputs:provider", &taxonomy.provider);
            if !taxonomy.category.is_empty() {
                write_string(out, &inner, "inputs:category", &taxonomy.category);
            }
            if !taxonomy.sub_category.is_empty() {
                write_string(out, &inner, "inputs:sub_category", &taxonomy.sub_category);
            }
            if !taxonomy.exchange_mic.is_empty() {
                write_string(out, &inner, "inputs:exchange_mic", &taxonomy.exchange_mic);
            }
            if let Some(AssetSourceType::Csv { path: csv_path }) = &node.asset_source {
                write_string(out, &inner, "inputs:csv_path", csv_path);
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
            write_string(out, &inner, "outputs:weights", "");
        }
        NodeType::TerminalIntegrator { engine_target } => {
            write_token(out, &inner, "inputs:id", engine_target);
        }
    }

    write_ui_canvas_pos(out, &inner, node.x, node.y);
    write_user_label(out, &inner, node);

    if let Some(rels) = relationships {
        for (relationship, targets) in rels {
            write_relationship(out, &inner, relationship, targets);
        }
    }

    out.push_str(&format!("{pad}}}\n"));
}

fn write_ui_canvas_pos(out: &mut String, indent: &str, x: f32, y: f32) {
    out.push_str(&format!("{indent}custom float2 ui:canvas:pos = ({x}, {y})\n"));
}

fn write_user_label(out: &mut String, indent: &str, node: &VisualNode) {
    let label = node.name.trim();
    if label.is_empty() {
        return;
    }
    let escaped = label.replace('"', "\\\"");
    out.push_str(&format!(
        "{indent}custom string {USER_LABEL_ATTR} = \"{escaped}\"\n"
    ));
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
    use pulsar_marketlab_core::{prim_display_label, PORTFOLIOS_SCOPE, SIGNALS_SCOPE, UNIVERSE_SCOPE};

    fn stable_leaf(id: usize) -> String {
        format!("node_{:08x}", id as u32)
    }

    fn universe_path(id: usize) -> String {
        workstation_stable_path(UNIVERSE_SCOPE, id)
    }

    fn signals_path(id: usize) -> String {
        workstation_stable_path(SIGNALS_SCOPE, id)
    }

    fn portfolio_path(id: usize) -> String {
        workstation_stable_path(PORTFOLIOS_SCOPE, id)
    }

    fn compose_usda(nodes: &[VisualNode], connections: &[NodeConnection]) -> String {
        compose_pipeline_usda_for_tests(nodes, connections)
    }

    fn assert_rel_targets(usda: &str, target_path: &str, expected_sources: &[&str]) {
        let leaf = target_path.rsplit('/').next().unwrap_or(target_path);
        assert!(
            usda.contains(&format!("\"{leaf}\"")),
            "expected prim leaf \"{leaf}\" in composed usda (target {target_path})"
        );
        for source in expected_sources {
            let source_path = source.trim_start_matches('<').trim_end_matches('>');
            assert!(
                usda.contains(&format!("<{source_path}>")),
                "expected relationship source <{source_path}> wired to {target_path}"
            );
        }
    }

    #[test]
    fn portfolio_sources_resolve_via_stage_graph_wires() {
        use pulsar_marketlab_ui::workspace::build_stage_graph_snapshot;
        use pulsar_marketlab_ui::workspace::WorkspaceContext;

        let nodes = vec![
            sample_named_asset(2, "QQQ"),
            sample_named_asset(3, "SPY"),
            sample_portfolio(1, "Alpha"),
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
        ];
        let usda = compose_usda(&nodes, &connections);
        let context = WorkspaceContext::from_usda_text(&usda).expect("context from composed usda");
        let snapshot = build_stage_graph_snapshot(context.usd_stage());
        let paths = resolve_node_stage_paths(&nodes, &connections);
        let target = paths.get(&1).expect("portfolio path").clone();
        let mut sources: Vec<String> = snapshot
            .wires
            .iter()
            .filter(|wire| wire.target_prim_path == target && wire.relationship == "inputs:sources")
            .map(|wire| wire.source_prim_path.clone())
            .collect();
        sources.sort();
        let mut expected = vec![
            paths.get(&2).expect("qqq path").clone(),
            paths.get(&3).expect("spy path").clone(),
        ];
        expected.sort();
        assert_eq!(sources, expected);
    }

    #[test]
    fn test_compose_flat_hierarchy_with_labels() {
        let nodes = vec![
            sample_named_asset(2, "QQQ"),
            sample_portfolio(1, "Sim Portfolio 1"),
        ];
        let connections = vec![NodeConnection {
            from_node_id: 2,
            from_port_idx: 0,
            to_node_id: 1,
            to_port_idx: 0,
        }];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        let asset_path = paths.get(&2).expect("qqq path").clone();
        let port_path = paths.get(&1).expect("portfolio path").clone();

        assert_eq!(paths.get(&2).map(String::as_str), Some(asset_path.as_str()));
        assert_eq!(paths.get(&1).map(String::as_str), Some(port_path.as_str()));
        assert!(!port_path.contains("node_"));

        assert_eq!(paths.get(&2).map(String::as_str), Some("/MarketLab/Universe/qqq"));
        assert_eq!(
            paths.get(&1).map(String::as_str),
            Some("/MarketLab/Portfolios/sim_portfolio_1")
        );

        let usda = compose_usda(&nodes, &connections);
        assert!(usda.contains(&format!("custom string info:user_label = \"QQQ.csv\"")));
        assert!(usda.contains("custom string info:user_label = \"Sim Portfolio 1\""));
        assert_rel_targets(&usda, &port_path, &[asset_path.as_str()]);
    }

    fn sample_asset(id: usize) -> VisualNode {
        VisualNode {
            id,
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
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
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
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
        let usda = compose_usda(&nodes, &connections);
        assert!(usda.contains("def Scope \"MarketLab\""));
        assert!(usda.contains("def Scope \"Universe\""));
        assert!(usda.contains("def Scope \"Signals\""));
        assert!(usda.contains("def FinancialAsset \"gld\""));
        assert!(usda.contains("def OtlTaUberSignal \"gld_rsi\""));
        assert!(usda.contains("custom string info:user_label = \"RSI\""));
        assert!(!usda.contains("def Xform"));
        assert!(usda.contains("token inputs:symbol = \"GLD\"") || usda.contains("inputs:symbol"));
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
        let usda = compose_usda(&nodes, &connections);
        let bridge = UsdStageBridge::open_from_usda_text(&usda).expect("parse composed stage");
        let rows = bridge.stage_prim_rows().expect("list prims");
        assert!(rows.iter().any(|row| row.path.ends_with("/gld")));
        assert!(rows.iter().any(|row| row.path.ends_with("/gld_rsi")));
        assert!(!rows.iter().any(|row| row.path == "/FinancialAsset"));
    }

    #[test]
    fn in_memory_compose_embeds_schema_inline_without_sublayer() {
        let usda = compose_pipeline_usda_for_tests(&[], &[]);
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
                include_metadata_sublayer: false,
                embed_schema_inline: false,
                include_workstation_layer_stack: false,
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
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
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
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
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
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
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
    fn flat_portfolio_stage_matches_macro_fund_layout() {
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
            Some("/MarketLab/Portfolios/master_macro_fund")
        );
        assert_eq!(
            paths.get(&2).map(String::as_str),
            Some("/MarketLab/Portfolios/equity_sub_book")
        );
        assert_eq!(paths.get(&3).map(String::as_str), Some("/MarketLab/Universe/spy"));
        assert_eq!(paths.get(&5).map(String::as_str), Some("/MarketLab/Universe/gld"));

        let usda = compose_usda(&nodes, &connections);
        assert!(usda.contains("def PortfolioIntegrator \"master_macro_fund\""));
        assert!(usda.contains("def FinancialAsset \"spy\""));
        assert!(usda.contains("def OtlTaUberSignal \"gld_rsi\""));

        assert_rel_targets(
            &usda,
            paths.get(&2).expect("sub book"),
            &[paths.get(&6).expect("gld rsi").as_str()],
        );
        assert_rel_targets(
            &usda,
            paths.get(&1).expect("master fund"),
            &[paths.get(&4).expect("spy sma").as_str()],
        );
        assert!(!usda.contains("/Master_Macro_Fund/Equity_Sub_Book/"));
        assert!(!usda.contains("/Master_Macro_Fund/SPY"));
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
        assert_eq!(paths.get(&2).map(String::as_str), Some("/MarketLab/Signals/sma"));
        assert_eq!(paths.get(&3).map(String::as_str), Some("/MarketLab/Signals/sma_01"));
        assert_eq!(paths.get(&4).map(String::as_str), Some("/MarketLab/Signals/sma_02"));

        let usda = compose_usda(&nodes, &connections);
        assert!(usda.contains("def OtlTaUberSignal \"sma\""));
        assert!(usda.contains("def OtlTaUberSignal \"sma_01\""));
        assert!(usda.contains("def OtlTaUberSignal \"sma_02\""));
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
        let paths = resolve_node_stage_paths(&nodes, &connections);
        let usda = compose_usda(&nodes, &connections);
        assert_rel_targets(
            &usda,
            paths.get(&1).expect("portfolio path"),
            &[paths.get(&2).expect("asset path")],
        );
    }

    #[test]
    fn sim_portfolio_canvas_uses_flat_stage_paths() {
        let nodes = vec![
            sample_portfolio(1, "Sim Portfolio 3"),
            sample_portfolio(2, "Sim Portfolio 1"),
            sample_named_asset(3, "QQQ"),
            sample_named_asset(4, "SPY"),
        ];
        let connections = vec![
            NodeConnection {
                from_node_id: 3,
                from_port_idx: 0,
                to_node_id: 2,
                to_port_idx: 0,
            },
            NodeConnection {
                from_node_id: 4,
                from_port_idx: 0,
                to_node_id: 2,
                to_port_idx: 1,
            },
            NodeConnection {
                from_node_id: 2,
                from_port_idx: 0,
                to_node_id: 1,
                to_port_idx: 0,
            },
        ];
        let paths = resolve_node_stage_paths(&nodes, &connections);
        assert_eq!(
            paths.get(&1).map(String::as_str),
            Some("/MarketLab/Portfolios/sim_portfolio_3")
        );
        assert_eq!(
            paths.get(&2).map(String::as_str),
            Some("/MarketLab/Portfolios/sim_portfolio_1")
        );
        assert_eq!(paths.get(&3).map(String::as_str), Some("/MarketLab/Universe/qqq"));
        assert_eq!(paths.get(&4).map(String::as_str), Some("/MarketLab/Universe/spy"));

        let usda = compose_usda(&nodes, &connections);
        assert!(!usda.contains("/Sim_Portfolio_3/Sim_Portfolio_1"));
        assert_rel_targets(
            &usda,
            paths.get(&2).expect("inner portfolio"),
            &[
                paths.get(&3).expect("qqq").as_str(),
                paths.get(&4).expect("spy").as_str(),
            ],
        );
        assert_rel_targets(
            &usda,
            paths.get(&1).expect("outer portfolio"),
            &[paths.get(&2).expect("inner portfolio").as_str()],
        );
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
        let usda = compose_usda(&nodes, &connections);
        let context =
            WorkspaceContext::from_usda_text(&usda).expect("workspace context from composed usda");
        let snapshot = build_stage_graph_snapshot(context.usd_stage());
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("graph engine compile");
        let prices: Vec<f64> = (0..22).map(|i| 300.0 + i as f64).collect();
        let spy_path = paths.get(&2).expect("SPY path").clone();
        let asset_vectors = std::collections::HashMap::from([(spy_path, prices)]);
        let result = engine.execute_timeline(
            pulsar_marketlab_core::shared_columns_from_vec(asset_vectors),
            22,
        );
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

    /// Regression for multi-portfolio simulation layouts: flat prim siblings wired by rels.
    #[test]
    fn graph_engine_executes_flat_portfolio_simulation_stage() {
        use pulsar_marketlab_core::MarketLabGraphEngine;
        use pulsar_marketlab_ui::workspace::{build_stage_graph_snapshot, WorkspaceContext};

        const STAGE: &str = r#"#usda 1.0
(
    defaultPrim = "MarketLab"
)

def Scope "MarketLab"
{
    def Scope "Universe"
    {
        def FinancialAsset "node_00000001"
        {
            token inputs:symbol = "QQQ"
        }
        def FinancialAsset "node_00000002"
        {
            token inputs:symbol = "SPY"
        }
        def FinancialAsset "node_00000003"
        {
            token inputs:symbol = "AGG"
        }
        def FinancialAsset "node_00000004"
        {
            token inputs:symbol = "TMF"
        }
    }
    def Scope "Portfolios"
    {
        def PortfolioIntegrator "node_00000005"
        {
            token inputs:id = "Allocation::HierarchicalRiskParity"
            double inputs:initial_capital = 10000
            rel inputs:sources = [
                </MarketLab/Universe/node_00000001>,
                </MarketLab/Universe/node_00000002>,
            ]
        }
        def PortfolioIntegrator "node_00000006"
        {
            token inputs:id = "Allocation::MeanVariance"
            double inputs:initial_capital = 10000
            rel inputs:sources = [
                </MarketLab/Universe/node_00000003>,
                </MarketLab/Universe/node_00000004>,
            ]
        }
        def PortfolioIntegrator "node_00000007"
        {
            token inputs:id = "Allocation::HierarchicalRiskParity"
            double inputs:initial_capital = 10000
            rel inputs:sources = [
                </MarketLab/Portfolios/node_00000005>,
                </MarketLab/Portfolios/node_00000006>,
            ]
        }
    }
}
"#;

        let context =
            WorkspaceContext::from_usda_text(STAGE).expect("workspace context from manual stage");
        let snapshot = build_stage_graph_snapshot(context.usd_stage());
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("graph engine compile");

        let bars = 22;
        let trend = |base: f64| -> Vec<f64> {
            (0..bars).map(|i| base + i as f64 * 0.5).collect()
        };
        let asset_vectors = std::collections::HashMap::from([
            (universe_path(1), trend(400.0)),
            (universe_path(2), trend(500.0)),
            (universe_path(3), trend(90.0)),
            (universe_path(4), trend(15.0)),
        ]);

        let result = engine.execute_timeline(
            pulsar_marketlab_core::shared_columns_from_vec(asset_vectors),
            bars,
        );

        let book_one = portfolio_path(5);
        let book_two = portfolio_path(6);
        let master = portfolio_path(7);

        for path in [&book_one, &book_two, &master] {
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

        let master_last = result.portfolio_results[&master].wealth_series.last().copied().unwrap();
        let blend = result.portfolio_results[&book_one].wealth_series.last().copied().unwrap()
            * 0.5
            + result.portfolio_results[&book_two].wealth_series.last().copied().unwrap() * 0.5;
        assert!(
            (master_last - blend).abs() < 600.0,
            "master NAV should track combined child books: master={master_last} blend={blend}"
        );
    }
}
