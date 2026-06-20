//! Cold-path OpenUSD import/export for the finance blueprint editor.
//!
//! `Stage::open` is only invoked from this module (explicit File Open/Save, session
//! sidecar reads). The interactive edit loop never touches live USD stages.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use graphy::{Connection, ConnectionType, GraphDescription, JsonValue, NodeInstance, Position};
use openusd::sdf::schema::FieldKey;
use openusd::sdf::PathListOp;
use openusd::Stage;
use pulsar_marketlab_core::{
    embed_schema_inline_in_layer, ensure_metadata_library_sidecar_for_document,
    ensure_schema_sidecar_for_document, finance_database_equities_empty_layer_usda,
    sp500_universe_layer_usda, GraphCompileWire, StageGraphPrim,
    FINANCE_DATABASE_EQUITIES_LAYER_FILENAME, PORTFOLIOS_SCOPE, SESSION_LAYER_FILENAME,
    SIGNALS_LAYER_FILENAME, SP500_UNIVERSE_LAYER_FILENAME, UNIVERSE_SCOPE, USER_LABEL_ATTR,
    WORKSTATION_LAYER_STACK, SIGNALS_SCOPE,
};

use crate::blueprint::finance_primary_output_pin;
use crate::cold_path_write::{
    prepare_finance_graph_for_cold_write, validate_finance_graph_for_cold_write,
    verify_cold_write_round_trip, FinanceColdWriteReport,
};
use crate::snapshot::{finance_node_prim_paths, graph_description_to_stage_snapshot};
use crate::taxonomy_index::{finance_asset_properties_for_symbol, FinanceDatabaseIndex};
use crate::types::{type_id, FinanceNodeKind, PORTFOLIO_ALLOCATION_TOKENS};

const MARKETLAB_ROOT: &str = "/MarketLab";
const MARKETLAB_DEFAULT_PRIM: &str = "MarketLab";
const LINEAGE_RELATIONSHIPS: [&str; 2] = ["inputs:underlying", "inputs:sources"];

static STAGE_OPEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Test / telemetry hook: number of `Stage::open` calls through this module.
pub fn stage_open_counter() -> u64 {
    STAGE_OPEN_COUNTER.load(Ordering::Relaxed)
}

#[derive(Debug, thiserror::Error)]
pub enum UsdPersistenceError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("USD error: {0}")]
    Usd(String),
    #[error("hydrate error: {0}")]
    Hydrate(String),
}

/// Reference to a workstation sublayer file beside the root document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceLayerRef {
    pub filename: String,
    pub strength_order: usize,
}

/// In-memory workstation document hydrated from a cold-path USD import.
#[derive(Clone, Debug)]
pub struct FinanceWorkspaceDocument {
    pub graph: GraphDescription,
    pub document_path: Option<PathBuf>,
    pub layer_stack: Vec<FinanceLayerRef>,
    /// Session-layer opinions keyed by absolute prim path → USD attribute → value.
    pub session_opinions: HashMap<String, HashMap<String, String>>,
    /// Graph node id → resolved USD prim path.
    pub resolved_prim_paths: HashMap<String, String>,
    /// Portfolio allocation overrides from session.usda keyed by node id.
    pub session_variant_overrides: HashMap<String, String>,
}

impl FinanceWorkspaceDocument {
    pub fn new(graph: GraphDescription) -> Self {
        let resolved_prim_paths = finance_node_prim_paths(&graph);
        Self {
            graph,
            document_path: None,
            layer_stack: Vec::new(),
            session_opinions: HashMap::new(),
            resolved_prim_paths,
            session_variant_overrides: HashMap::new(),
        }
    }

    pub fn refresh_resolved_prim_paths(&mut self) {
        self.resolved_prim_paths = finance_node_prim_paths(&self.graph);
    }
}

/// Session + resolved prim path context for the composition stack panel.
#[derive(Clone, Debug)]
pub struct FinanceSessionContext<'a> {
    pub opinions: &'a HashMap<String, HashMap<String, String>>,
    pub resolved_prim_paths: &'a HashMap<String, String>,
}

impl FinanceWorkspaceDocument {
    pub fn session_context(&self) -> FinanceSessionContext<'_> {
        FinanceSessionContext {
            opinions: &self.session_opinions,
            resolved_prim_paths: &self.resolved_prim_paths,
        }
    }
}

/// Cold-path gate: wraps a single `Stage::open` and increments the global counter.
pub struct UsdTransaction {
    stage: Stage,
    document_path: PathBuf,
}

impl UsdTransaction {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, UsdPersistenceError> {
        let document_path = path.as_ref().to_path_buf();
        STAGE_OPEN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stage = Stage::open(document_path.to_string_lossy().as_ref())
            .map_err(|err| UsdPersistenceError::Usd(err.to_string()))?;
        Ok(Self {
            stage,
            document_path,
        })
    }

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn document_path(&self) -> &Path {
        &self.document_path
    }
}

/// Import a `.usda` document into an in-memory [`FinanceWorkspaceDocument`].
pub fn import_document(path: impl AsRef<Path>) -> Result<FinanceWorkspaceDocument, UsdPersistenceError> {
    let path = path.as_ref();
    let tx = UsdTransaction::open(path)?;
    let mut doc = hydrate_document(&tx)?;
    refresh_asset_taxonomy_in_graph(&mut doc.graph);
    doc.document_path = Some(path.to_path_buf());
    doc.layer_stack = default_layer_stack();
    doc.session_opinions = load_session_opinions(path)?;
    doc.session_variant_overrides =
        session_allocation_overrides(&doc.graph, &doc.session_opinions, &doc.resolved_prim_paths);
    Ok(doc)
}

/// Export a graph to an on-disk workstation `.usda` project (cold path only).
///
/// Reviews the full finance graph, refreshes asset tokens from the project database,
/// rebuilds the stage snapshot, writes USDA, and reopens the file to verify topology.
pub fn export_document(
    doc: &mut FinanceWorkspaceDocument,
    path: impl AsRef<Path>,
    database: Option<&FinanceDatabaseIndex>,
) -> Result<FinanceColdWriteReport, UsdPersistenceError> {
    let path = path.as_ref();
    let assets_refreshed = prepare_finance_graph_for_cold_write(&mut doc.graph, database);
    doc.refresh_resolved_prim_paths();

    let expected_finance_nodes = doc.graph.nodes.len();
    let (_snapshot, compile) = validate_finance_graph_for_cold_write(&doc.graph)?;

    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    ensure_schema_sidecar_for_document(path)?;
    ensure_metadata_library_sidecar_for_document(path)?;
    let session_path = parent.join(SESSION_LAYER_FILENAME);
    if !session_path.is_file() {
        fs::write(&session_path, "#usda 1.0\n(\n)\n")?;
    }
    let signals_path = parent.join(SIGNALS_LAYER_FILENAME);
    if !signals_path.is_file() {
        fs::write(&signals_path, "#usda 1.0\n(\n)\n")?;
    }
    let sp500_path = parent.join(SP500_UNIVERSE_LAYER_FILENAME);
    if !sp500_path.is_file() {
        fs::write(&sp500_path, sp500_universe_layer_usda())?;
    }
    let equities_path = parent.join(FINANCE_DATABASE_EQUITIES_LAYER_FILENAME);
    if !equities_path.is_file() {
        fs::write(
            &equities_path,
            finance_database_equities_empty_layer_usda(),
        )?;
    }

    let usda = compose_finance_usda(&doc.graph)?;
    let usda = embed_schema_inline_in_layer(&usda);
    fs::write(path, usda)?;

    let (round_trip_nodes, round_trip_connections) =
        verify_cold_write_round_trip(path, expected_finance_nodes)?;

    Ok(FinanceColdWriteReport {
        compile,
        assets_refreshed,
        round_trip_nodes,
        round_trip_connections,
    })
}

fn default_layer_stack() -> Vec<FinanceLayerRef> {
    WORKSTATION_LAYER_STACK
        .iter()
        .enumerate()
        .map(|(idx, filename)| FinanceLayerRef {
            filename: (*filename).to_string(),
            strength_order: idx,
        })
        .collect()
}

fn hydrate_document(tx: &UsdTransaction) -> Result<FinanceWorkspaceDocument, UsdPersistenceError> {
    let stage = tx.stage();
    let mut prims = Vec::new();
    let mut wires = Vec::new();
    collect_stage_graph(stage, MARKETLAB_ROOT, &mut prims, &mut wires)?;
    if prims.is_empty() {
        return Err(UsdPersistenceError::Hydrate(
            "no executable finance prims under /MarketLab".into(),
        ));
    }
    let graph = graph_from_stage_prims(&prims, &wires)?;
    Ok(FinanceWorkspaceDocument::new(graph))
}

fn collect_stage_graph(
    stage: &Stage,
    path: &str,
    prims: &mut Vec<StageGraphPrim>,
    wires: &mut Vec<GraphCompileWire>,
) -> Result<(), UsdPersistenceError> {
    let children = stage
        .prim_children(path)
        .map_err(|err| UsdPersistenceError::Usd(err.to_string()))?;
    for child in children {
        let child_path = format!("{path}/{child}");
        if !prim_active(stage, &child_path) || is_schema_template_prim(&child_path) {
            continue;
        }
        if let Some(prim) = classify_prim(stage, &child_path) {
            prims.push(prim);
        }
        for relationship in LINEAGE_RELATIONSHIPS {
            for target in relationship_targets(stage, &child_path, relationship) {
                wires.push(GraphCompileWire {
                    source_prim_path: target,
                    target_prim_path: child_path.clone(),
                    relationship: relationship.to_string(),
                });
            }
        }
        collect_stage_graph(stage, &child_path, prims, wires)?;
    }
    Ok(())
}

fn graph_from_stage_prims(
    prims: &[StageGraphPrim],
    wires: &[GraphCompileWire],
) -> Result<GraphDescription, UsdPersistenceError> {
    let mut graph = GraphDescription::new("imported");
    let mut path_to_id: HashMap<String, String> = HashMap::new();
    let mut used_ids: HashSet<String> = HashSet::new();

    for (index, prim) in prims.iter().enumerate() {
        let node_id = unique_node_id(prim, index, &mut used_ids);
        path_to_id.insert(prim.path.clone(), node_id.clone());
        let kind = FinanceNodeKind::from_stage_type_name(&prim.type_name).ok_or_else(|| {
            UsdPersistenceError::Hydrate(format!("unknown prim type {}", prim.type_name))
        })?;
        let node_type = graphy_type_for_prim(kind, &prim.attributes);
        let (x, y) = canvas_position(&prim.attributes);
        let mut node = NodeInstance::new(
            node_id.clone(),
            node_type,
            Position::new(x as f64, y as f64),
        );
        apply_prim_properties(&mut node, kind, &prim.attributes, &prim.path);
        graph.add_node(node);
    }

    let mut portfolio_port_cursor: HashMap<String, usize> = HashMap::new();
    for wire in wires {
        let Some(source_id) = path_to_id.get(&wire.source_prim_path) else {
            continue;
        };
        let Some(target_id) = path_to_id.get(&wire.target_prim_path) else {
            continue;
        };
        let source_node = graph.nodes.get(source_id).ok_or_else(|| {
            UsdPersistenceError::Hydrate(format!("missing source node {source_id}"))
        })?;
        let target_node = graph.nodes.get(target_id).ok_or_else(|| {
            UsdPersistenceError::Hydrate(format!("missing target node {target_id}"))
        })?;
        let source_pin = finance_primary_output_pin(&source_node.node_type)
            .unwrap_or("result")
            .to_string();
        let target_pin = if wire.relationship == "inputs:sources" {
            let port = portfolio_port_cursor.entry(target_id.clone()).or_insert(0);
            let idx = *port;
            *port += 1;
            format!("signal_{idx}")
        } else if FinanceNodeKind::from_graphy_type_id(&target_node.node_type)
            == Some(FinanceNodeKind::OtlOperator)
            || FinanceNodeKind::from_graphy_type_id(&target_node.node_type)
                == Some(FinanceNodeKind::OtlTaUberSignal)
        {
            "underlying".to_string()
        } else {
            "source_stream".to_string()
        };
        graph.add_connection(Connection {
            source_node: source_id.clone(),
            source_pin,
            target_node: target_id.clone(),
            target_pin,
            connection_type: ConnectionType::Data,
        });
    }

    Ok(graph)
}

fn unique_node_id(prim: &StageGraphPrim, index: usize, used: &mut HashSet<String>) -> String {
    let leaf = prim
        .path
        .rsplit('/')
        .next()
        .unwrap_or("node")
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let base = if leaf.is_empty() {
        format!("node_{index:04}")
    } else {
        leaf
    };
    if used.insert(base.clone()) {
        return base;
    }
    let mut suffix = 1u32;
    loop {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn graphy_type_for_prim(kind: FinanceNodeKind, attrs: &HashMap<String, String>) -> String {
    match kind {
        FinanceNodeKind::FinancialAsset => type_id::FINANCIAL_ASSET.to_string(),
        FinanceNodeKind::OtlOperator => type_id::OTL_OPERATOR.to_string(),
        FinanceNodeKind::PortfolioIntegrator => type_id::PORTFOLIO_INTEGRATOR.to_string(),
        FinanceNodeKind::OtlTaUberSignal => {
            let token = attrs
                .get("info:archetype")
                .map(String::as_str)
                .unwrap_or("trend");
            match token {
                "volatility" => type_id::TA_VOLATILITY.to_string(),
                "oscillator" => type_id::TA_OSCILLATOR.to_string(),
                "channel" => type_id::TA_CHANNEL.to_string(),
                _ => type_id::TA_TREND.to_string(),
            }
        }
    }
}

fn apply_prim_properties(
    node: &mut NodeInstance,
    kind: FinanceNodeKind,
    attrs: &HashMap<String, String>,
    prim_path: &str,
) {
    let insert = |node: &mut NodeInstance, key: &str, value: String| {
        if !value.is_empty() {
            node.properties
                .insert(key.to_string(), JsonValue::String(value));
        }
    };

    match kind {
        FinanceNodeKind::FinancialAsset => {
            insert(
                node,
                "symbol",
                attrs
                    .get("inputs:symbol")
                    .cloned()
                    .unwrap_or_else(|| prim_leaf(prim_path).to_string()),
            );
            insert(
                node,
                "asset_class",
                attrs
                    .get("inputs:asset_class")
                    .cloned()
                    .unwrap_or_else(|| "Equity".into()),
            );
            insert(
                node,
                "csv_path",
                attrs.get("inputs:csv_path").cloned().unwrap_or_default(),
            );
            insert(
                node,
                "category",
                attrs.get("inputs:category").cloned().unwrap_or_default(),
            );
            insert(
                node,
                "sub_category",
                attrs
                    .get("inputs:sub_category")
                    .cloned()
                    .unwrap_or_default(),
            );
            insert(
                node,
                "exchange_mic",
                attrs
                    .get("inputs:exchange_mic")
                    .cloned()
                    .unwrap_or_default(),
            );
            insert(
                node,
                "provider",
                attrs.get("inputs:provider").cloned().unwrap_or_default(),
            );
            for (usd_key, prop_key) in [
                ("info:sector", "info_sector"),
                ("info:industry_group", "info_industry_group"),
                ("info:industry", "info_industry"),
                ("info:currency", "info_currency"),
                ("info:country", "info_country"),
                ("info:state", "info_state"),
                ("info:zipcode", "info_zipcode"),
                ("info:market_cap_class", "info_market_cap_class"),
            ] {
                insert(
                    node,
                    prop_key,
                    attrs.get(usd_key).cloned().unwrap_or_default(),
                );
            }
            insert(
                node,
                "prim_path",
                format!(
                    "/MarketLab/Universe/{}",
                    attrs
                        .get("inputs:symbol")
                        .map(String::as_str)
                        .unwrap_or_else(|| prim_leaf(prim_path))
                        .to_ascii_uppercase()
                ),
            );
        }
        FinanceNodeKind::OtlOperator => {
            insert(
                node,
                "script_src",
                attrs.get("inputs:script_src").cloned().unwrap_or_default(),
            );
            insert(
                node,
                "script_compiled_path",
                attrs
                    .get("inputs:script_compiled_path")
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        FinanceNodeKind::OtlTaUberSignal => {
            insert(
                node,
                "archetype",
                attrs
                    .get("info:archetype")
                    .cloned()
                    .unwrap_or_else(|| "trend".into()),
            );
            insert(
                node,
                "algorithm",
                attrs
                    .get("info:algorithm")
                    .cloned()
                    .unwrap_or_default(),
            );
            for key in ["period", "signal_period", "multiplier", "annualization"] {
                if let Some(value) = attrs.get(&format!("inputs:{key}")) {
                    insert(node, key, value.clone());
                }
            }
        }
        FinanceNodeKind::PortfolioIntegrator => {
            insert(
                node,
                "name",
                attrs
                    .get(USER_LABEL_ATTR)
                    .cloned()
                    .unwrap_or_else(|| prim_leaf(prim_path).to_string()),
            );
            insert(
                node,
                "allocation_id",
                attrs
                    .get("inputs:id")
                    .cloned()
                    .unwrap_or_else(|| PORTFOLIO_ALLOCATION_TOKENS[0].to_string()),
            );
            if let Some(value) = attrs.get("inputs:initial_capital") {
                insert(node, "initial_capital", value.clone());
            }
            if let Some(value) = attrs.get("inputs:rebalance_frequency") {
                insert(node, "rebalance_frequency", value.clone());
            }
        }
    }

    if let Some(label) = attrs.get(USER_LABEL_ATTR) {
        insert(node, "display_name", label.clone());
    }
}

fn canvas_position(attrs: &HashMap<String, String>) -> (f32, f32) {
    attrs
        .get("ui:canvas:pos")
        .and_then(|text| {
            let parts: Vec<_> = text
                .trim_matches(|c| c == '(' || c == ')')
                .split(',')
                .collect();
            if parts.len() == 2 {
                let x = parts[0].trim().parse().ok()?;
                let y = parts[1].trim().parse().ok()?;
                Some((x, y))
            } else {
                None
            }
        })
        .unwrap_or((0.0, 0.0))
}

fn compose_finance_usda(graph: &GraphDescription) -> Result<String, UsdPersistenceError> {
    let snapshot = graph_description_to_stage_snapshot(graph);
    let paths = finance_node_prim_paths(graph);
    let path_to_node: HashMap<String, String> = paths
        .iter()
        .map(|(id, path)| (path.clone(), id.clone()))
        .collect();

    let mut relationships: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for wire in &snapshot.wires {
        relationships
            .entry(wire.target_prim_path.clone())
            .or_default()
            .entry(wire.relationship.clone())
            .or_default()
            .push(wire.source_prim_path.clone());
    }

    let mut out = String::from("#usda 1.0\n(\n");
    out.push_str(&format!("    defaultPrim = \"{MARKETLAB_DEFAULT_PRIM}\"\n)\n\n"));
    out.push_str(&format!("def Scope \"{MARKETLAB_DEFAULT_PRIM}\"\n{{\n"));

    for scope in [UNIVERSE_SCOPE, SIGNALS_SCOPE, PORTFOLIOS_SCOPE] {
        out.push_str(&format!("    def Scope \"{scope}\"\n    {{\n"));
        let mut scope_prims: Vec<&StageGraphPrim> = snapshot
            .prims
            .iter()
            .filter(|prim| prim_in_scope(&prim.path, scope))
            .collect();
        scope_prims.sort_by(|a, b| a.path.cmp(&b.path));
        let mut written_paths = HashSet::new();
        for prim in scope_prims {
            if !written_paths.insert(prim.path.clone()) {
                continue;
            }
            write_prim_block(&mut out, prim, relationships.get(&prim.path), graph, &path_to_node)?;
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n");
    Ok(out)
}

fn write_prim_block(
    out: &mut String,
    prim: &StageGraphPrim,
    relationships: Option<&HashMap<String, Vec<String>>>,
    graph: &GraphDescription,
    path_to_node: &HashMap<String, String>,
) -> Result<(), UsdPersistenceError> {
    let leaf = if prim.type_name == "FinancialAsset" {
        prim.attributes
            .get("inputs:symbol")
            .filter(|symbol| !symbol.is_empty())
            .map(|symbol| symbol.to_ascii_uppercase())
            .unwrap_or_else(|| prim_leaf(&prim.path).to_ascii_uppercase())
    } else {
        prim_leaf(&prim.path).to_string()
    };
    out.push_str(&format!("        def {} \"{leaf}\"\n        {{\n", prim.type_name));
    for (key, value) in &prim.attributes {
        write_attribute(out, key, value);
    }
    if let Some(node_id) = path_to_node.get(&prim.path) {
        if let Some(node) = graph.nodes.get(node_id) {
            write_attribute(
                out,
                "ui:canvas:pos",
                &format!("({}, {})", node.position.x, node.position.y),
            );
        }
    }
    if let Some(rels) = relationships {
        for (relationship, targets) in rels {
            if targets.is_empty() {
                continue;
            }
            out.push_str(&format!("            rel {relationship} = ["));
            for (idx, target) in targets.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("<{target}>"));
            }
            out.push_str("]\n");
        }
    }
    out.push_str("        }\n");
    Ok(())
}

fn write_attribute(out: &mut String, key: &str, value: &str) {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let (usd_type, formatted) = if value == "true" || value == "false" {
        ("bool", value.to_string())
    } else if key.starts_with("info:")
        || key == "inputs:category"
        || key == "inputs:sub_category"
    {
        ("string", format!("\"{escaped}\""))
    } else if value.parse::<f64>().is_ok() && !value.is_empty() {
        ("float", value.to_string())
    } else if value.parse::<u64>().is_ok() && !value.is_empty() {
        ("int", value.to_string())
    } else {
        ("token", format!("\"{escaped}\""))
    };
    out.push_str(&format!("            {usd_type} {key} = {formatted}\n"));
}

fn prim_in_scope(path: &str, scope: &str) -> bool {
    if path.starts_with(&format!("{MARKETLAB_ROOT}/{scope}/")) {
        return true;
    }
    match scope {
        UNIVERSE_SCOPE if path.starts_with(&format!("{MARKETLAB_ROOT}/"))
            && path.matches('/').count() == 2 =>
        {
            true
        }
        SIGNALS_SCOPE if path.starts_with("/analytics/") => true,
        PORTFOLIOS_SCOPE if path.starts_with("/portfolios/") => true,
        _ => false,
    }
}

fn load_session_opinions(
    document_path: &Path,
) -> Result<HashMap<String, HashMap<String, String>>, UsdPersistenceError> {
    let Some(parent) = document_path.parent() else {
        return Ok(HashMap::new());
    };
    let session_path = parent.join(SESSION_LAYER_FILENAME);
    if !session_path.is_file() {
        return Ok(HashMap::new());
    }
    let tx = UsdTransaction::open(&session_path)?;
    let mut opinions = HashMap::new();
    collect_prim_opinions(tx.stage(), MARKETLAB_ROOT, &mut opinions);
    Ok(opinions)
}

fn collect_prim_opinions(
    stage: &Stage,
    path: &str,
    out: &mut HashMap<String, HashMap<String, String>>,
) {
    if let Some(type_name) = prim_type_name(stage, path) {
        if classify_type_name(&type_name).is_some() {
            let attrs = read_prim_attributes(stage, path);
            if !attrs.is_empty() {
                out.insert(path.to_string(), attrs);
            }
        }
    }
    if let Ok(children) = stage.prim_children(path) {
        for child in children {
            collect_prim_opinions(stage, &format!("{path}/{child}"), out);
        }
    }
}

fn session_allocation_overrides(
    graph: &GraphDescription,
    session_opinions: &HashMap<String, HashMap<String, String>>,
    resolved_paths: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut overrides = HashMap::new();
    for (node_id, prim_path) in resolved_paths {
        let Some(node) = graph.nodes.get(node_id) else {
            continue;
        };
        if FinanceNodeKind::from_graphy_type_id(&node.node_type)
            != Some(FinanceNodeKind::PortfolioIntegrator)
        {
            continue;
        }
        if let Some(attrs) = session_opinions.get(prim_path) {
            if let Some(allocation) = attrs.get("inputs:id") {
                overrides.insert(node_id.clone(), allocation.clone());
            }
        }
    }
    overrides
}

fn prim_active(stage: &Stage, path: &str) -> bool {
    stage
        .field::<bool>(path, FieldKey::Active)
        .ok()
        .flatten()
        .unwrap_or(true)
}

fn is_schema_template_prim(path: &str) -> bool {
    matches!(
        path,
        "/FinancialAsset"
            | "/OtlOperator"
            | "/OtlTaUberSignal"
            | "/PortfolioIntegrator"
            | "/Typed"
            | "/Plugins"
            | "/Scope"
    )
}

fn prim_type_name(stage: &Stage, path_str: &str) -> Option<String> {
    stage
        .field::<String>(path_str, FieldKey::TypeName)
        .ok()
        .flatten()
        .map(|token| token.trim_matches('"').to_string())
        .filter(|name| !name.is_empty())
}

fn classify_type_name(type_name: &str) -> Option<&str> {
    match type_name {
        "FinancialAsset" | "OtlOperator" | "OtlTaUberSignal" | "PortfolioIntegrator" => {
            Some(type_name)
        }
        _ => None,
    }
}

fn classify_prim(stage: &Stage, path_str: &str) -> Option<StageGraphPrim> {
    if is_schema_template_prim(path_str) {
        return None;
    }
    let type_name = prim_type_name(stage, path_str).or_else(|| legacy_type_name_from_path(path_str))?;
    classify_type_name(&type_name)?;
    let attributes = read_prim_attributes(stage, path_str);
    Some(StageGraphPrim {
        path: path_str.to_string(),
        type_name: type_name.to_string(),
        attributes,
    })
}

fn read_prim_attributes(stage: &Stage, path_str: &str) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    if let Ok(properties) = stage.prim_properties(path_str) {
        for property in properties {
            if property == "active" {
                continue;
            }
            let property_path = format!("{path_str}.{property}");
            if let Ok(Some(value)) = stage.field::<String>(property_path.as_str(), FieldKey::Default)
            {
                attributes.insert(property, value.trim_matches('"').to_string());
            } else if let Ok(Some(value)) =
                stage.field::<f64>(property_path.as_str(), FieldKey::Default)
            {
                attributes.insert(property, value.to_string());
            } else if let Ok(Some(value)) =
                stage.field::<bool>(property_path.as_str(), FieldKey::Default)
            {
                attributes.insert(property, value.to_string());
            }
        }
    }
    if attributes.get("inputs:symbol").is_none() {
        if let Some(symbol) = path_str.rsplit('/').next() {
            attributes.insert("inputs:symbol".to_string(), symbol.to_string());
        }
    }
    attributes
}

fn legacy_type_name_from_path(path_str: &str) -> Option<String> {
    if path_str.starts_with("/assets/") {
        Some("FinancialAsset".to_string())
    } else if path_str.starts_with("/analytics/") {
        Some("OtlTaUberSignal".to_string())
    } else if path_str.starts_with("/portfolios/") {
        Some("PortfolioIntegrator".to_string())
    } else {
        None
    }
}

fn relationship_targets(stage: &Stage, prim_path: &str, relationship: &str) -> Vec<String> {
    let property_path = format!("{prim_path}.{relationship}");
    stage
        .field::<PathListOp>(property_path.as_str(), FieldKey::TargetPaths)
        .ok()
        .flatten()
        .map(path_list_op_targets)
        .unwrap_or_default()
}

fn path_list_op_targets(list_op: PathListOp) -> Vec<String> {
    list_op
        .iter()
        .map(|path| path.to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

fn prim_leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Merge embedded taxonomy metadata into financial asset node properties after cold import.
fn refresh_asset_taxonomy_in_graph(graph: &mut GraphDescription) {
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
        let autofill = finance_asset_properties_for_symbol(symbol, None);
        for (key, value) in autofill {
            if key == "csv_path" {
                continue;
            }
            node.properties
                .insert(key, JsonValue::String(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::compile::compile_finance_graph;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("pulsar_marketlab")
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn import_spy_assets_hydrates_graph() {
        let doc = import_document(fixture_path("spy_assets.usda")).expect("import");
        assert_eq!(doc.graph.nodes.len(), 1);
        let node = doc.graph.nodes.values().next().expect("node");
        assert_eq!(node.node_type, type_id::FINANCIAL_ASSET);
        assert_eq!(
            node.properties.get("symbol").and_then(|v| v.as_str()),
            Some("SPY")
        );
    }

    #[test]
    fn export_round_trip_preserves_topology_and_asset_taxonomy() {
        let mut doc = import_document(fixture_path("spy_assets.usda")).expect("import");
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("roundtrip.usda");
        export_document(&mut doc, &path, None).expect("export");
        let reopened = import_document(&path).expect("reopen");
        assert_eq!(reopened.graph.nodes.len(), doc.graph.nodes.len());
        let node = reopened.graph.nodes.values().next().expect("node");
        assert_eq!(
            node.properties.get("symbol").and_then(|v| v.as_str()),
            Some("SPY")
        );
        assert!(
            node.properties.get("category").and_then(|v| v.as_str()).is_some(),
            "taxonomy category should be present after import"
        );
        let (snapshot, _report) = compile_finance_graph(&reopened.graph).expect("compile");
        assert_eq!(snapshot.prims.len(), 1);
        assert_eq!(snapshot.prims[0].path, "/MarketLab/Universe/SPY");
        assert!(
            snapshot.prims[0]
                .attributes
                .get("inputs:category")
                .is_some(),
            "category should round-trip in USD attributes"
        );
    }

    #[test]
    fn export_round_trip_preserves_risk_parity_topology() {
        use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

        let mut graph = GraphDescription::new("risk_parity");
        let assets = ["spy", "vea", "ief", "tlt"];
        for (index, symbol) in assets.iter().enumerate() {
            let mut node = NodeInstance::new(
                *symbol,
                type_id::FINANCIAL_ASSET,
                Position::new(index as f64 * 120.0, 0.0),
            );
            node.properties.insert(
                "symbol".to_string(),
                graphy::JsonValue::String(symbol.to_ascii_uppercase()),
            );
            node.properties.insert(
                "name".to_string(),
                graphy::JsonValue::String(symbol.to_ascii_uppercase()),
            );
            graph.add_node(node);
        }

        let portfolios = [
            ("equities_portfolio", "Equities portfolio", vec!["spy", "vea"]),
            ("rates_portfolio", "Rates Portfolio", vec!["ief", "tlt"]),
            ("final_portfolio", "Final portfolio", vec!["equities_portfolio", "rates_portfolio"]),
        ];
        for (id, name, _) in &portfolios {
            let mut node = NodeInstance::new(
                *id,
                type_id::PORTFOLIO_INTEGRATOR,
                Position::new(400.0, 0.0),
            );
            node.properties.insert(
                "name".to_string(),
                graphy::JsonValue::String((*name).to_string()),
            );
            node.properties.insert(
                "allocation_id".to_string(),
                graphy::JsonValue::String("HierarchicalRiskParity".to_string()),
            );
            graph.add_node(node);
        }

        for (portfolio_id, _, sources) in &portfolios {
            for (index, source) in sources.iter().enumerate() {
                graph.add_connection(Connection {
                    source_node: (*source).to_string(),
                    source_pin: "close".to_string(),
                    target_node: (*portfolio_id).to_string(),
                    target_pin: format!("signal_{index}"),
                    connection_type: ConnectionType::Data,
                });
            }
        }

        let mut doc = FinanceWorkspaceDocument::new(graph);
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("risk_parity.usda");
        export_document(&mut doc, &path, None).expect("export");

        let usda = std::fs::read_to_string(&path).expect("read usda");
        assert!(usda.contains("FinancialAsset \"SPY\""), "missing SPY prim");
        assert!(usda.contains("FinancialAsset \"VEA\""), "missing VEA prim");
        assert!(usda.contains("FinancialAsset \"IEF\""), "missing IEF prim");
        assert!(usda.contains("FinancialAsset \"TLT\""), "missing TLT prim");
        assert!(
            usda.contains("rel inputs:sources"),
            "missing portfolio lineage relationships"
        );
        assert!(
            !usda.contains("subLayers"),
            "root export must not reference workstation sublayers"
        );

        let reopened = import_document(&path).expect("reopen");
        assert_eq!(reopened.graph.nodes.len(), 7, "expected 4 assets + 3 portfolios");
        assert_eq!(
            reopened.graph.connections.len(),
            6,
            "expected 2+2 asset wires + 2 portfolio wires"
        );

        let asset_count = reopened
            .graph
            .nodes
            .values()
            .filter(|node| node.node_type == type_id::FINANCIAL_ASSET)
            .count();
        assert_eq!(asset_count, 4);
    }

    #[test]
    fn financial_asset_prim_name_uses_symbol_not_path_leaf() {
        use graphy::{GraphDescription, NodeInstance, Position};

        let mut graph = GraphDescription::new("test");
        let mut node = NodeInstance::new(
            "asset_a",
            type_id::FINANCIAL_ASSET,
            Position::new(10.0, 20.0),
        );
        node.properties.insert(
            "symbol".to_string(),
            graphy::JsonValue::String("TLT".to_string()),
        );
        node.properties.insert(
            "prim_path".to_string(),
            graphy::JsonValue::String("/MarketLab/Universe/SPY".to_string()),
        );
        graph.add_node(node);

        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("symbol_leaf.usda");
        export_document(&mut FinanceWorkspaceDocument::new(graph), &path, None).expect("export");
        let usda = std::fs::read_to_string(&path).expect("read");
        assert!(
            usda.contains("def FinancialAsset \"TLT\""),
            "asset prim name must follow symbol, not stale path leaf: {usda}"
        );
        assert!(!usda.contains("def FinancialAsset \"SPY\""));
    }

    #[test]
    fn stage_open_counter_increments_per_transaction() {
        let before = stage_open_counter();
        let _ = UsdTransaction::open(fixture_path("spy_assets.usda")).expect("open");
        assert!(stage_open_counter() > before);
    }
}
