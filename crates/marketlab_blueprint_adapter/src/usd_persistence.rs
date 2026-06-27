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

use crate::asset_data::normalize_finance_file_path;
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
/// USD scope folder for OTL / TA analytics prims (`/MarketLab/Analytics/...`).
const ANALYTICS_SCOPE: &str = "Analytics";
/// USD scope folder for performance tear-sheet prims (`/MarketLab/Reporting/...`).
const REPORTING_SCOPE: &str = "Reporting";
const LINEAGE_RELATIONSHIPS: [&str; 4] = [
    "inputs:underlying",
    "inputs:sources",
    "inputs:series",
    "inputs:benchmark",
];

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
    dedupe_finance_graph_connections(&mut doc.graph);
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
    dedupe_finance_graph_connections(&mut doc.graph);
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
                let wire = GraphCompileWire {
                    source_prim_path: target,
                    target_prim_path: child_path.clone(),
                    relationship: relationship.to_string(),
                };
                if wires.iter().any(|existing| {
                    existing.source_prim_path == wire.source_prim_path
                        && existing.target_prim_path == wire.target_prim_path
                        && existing.relationship == wire.relationship
                }) {
                    continue;
                }
                wires.push(wire);
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
    let mut performance_series_cursor: HashMap<String, usize> = HashMap::new();
    let mut seen_lineage: HashSet<(String, String, String)> = HashSet::new();
    for wire in wires {
        let lineage_key = (
            wire.source_prim_path.clone(),
            wire.target_prim_path.clone(),
            wire.relationship.clone(),
        );
        if !seen_lineage.insert(lineage_key) {
            continue;
        }
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
        } else if wire.relationship == "inputs:series" {
            let port = performance_series_cursor.entry(target_id.clone()).or_insert(0);
            let idx = *port;
            *port += 1;
            format!("series_{idx}")
        } else if wire.relationship == "inputs:benchmark" {
            crate::series_pins::PERFORMANCE_BENCHMARK_PIN.to_string()
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
        FinanceNodeKind::FinancialReturnAsset => type_id::FINANCIAL_RETURN_ASSET.to_string(),
        FinanceNodeKind::OtlOperator => type_id::OTL_OPERATOR.to_string(),
        FinanceNodeKind::PortfolioIntegrator => type_id::PORTFOLIO_INTEGRATOR.to_string(),
        FinanceNodeKind::PerformanceAnalytics => type_id::PERFORMANCE_ANALYTICS.to_string(),
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
        FinanceNodeKind::FinancialAsset | FinanceNodeKind::FinancialReturnAsset => {
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
                    .unwrap_or_else(|| {
                        if kind == FinanceNodeKind::FinancialReturnAsset {
                            "Alternative".into()
                        } else {
                            "Equity".into()
                        }
                    }),
            );
            insert(
                node,
                "csv_path",
                normalize_finance_file_path(
                    &attrs.get("inputs:csv_path").cloned().unwrap_or_default(),
                ),
            );
            if kind == FinanceNodeKind::FinancialAsset {
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
            }
            let symbol_leaf = attrs
                .get("inputs:symbol")
                .map(String::as_str)
                .unwrap_or_else(|| prim_leaf(prim_path));
            let prim_leaf = if kind == FinanceNodeKind::FinancialReturnAsset {
                symbol_leaf.to_string()
            } else {
                symbol_leaf.to_ascii_uppercase()
            };
            insert(
                node,
                "prim_path",
                format!("/MarketLab/Universe/{prim_leaf}"),
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
        FinanceNodeKind::PerformanceAnalytics => {
            insert(
                node,
                "name",
                attrs
                    .get("inputs:name")
                    .or_else(|| attrs.get(USER_LABEL_ATTR))
                    .cloned()
                    .unwrap_or_else(|| prim_leaf(prim_path).to_string()),
            );
            if let Some(value) = attrs.get("inputs:risk_free_rate") {
                insert(node, "risk_free_rate", value.clone());
            }
            if let Some(value) = attrs.get("inputs:rolling_window") {
                insert(node, "rolling_window", value.clone());
            }
            if let Some(value) = attrs.get("inputs:benchmark_mode") {
                insert(node, "benchmark_mode", value.clone());
            }
            if let Some(value) = attrs.get("inputs:benchmark_symbol") {
                insert(node, "benchmark_symbol", value.clone());
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
        let targets = relationships
            .entry(wire.target_prim_path.clone())
            .or_default()
            .entry(wire.relationship.clone())
            .or_default();
        if !targets.iter().any(|target| target == &wire.source_prim_path) {
            targets.push(wire.source_prim_path.clone());
        }
    }

    let mut out = String::from("#usda 1.0\n(\n");
    out.push_str(&format!("    defaultPrim = \"{MARKETLAB_DEFAULT_PRIM}\"\n)\n\n"));
    out.push_str(&format!("def Scope \"{MARKETLAB_DEFAULT_PRIM}\"\n{{\n"));

    for scope in [
        UNIVERSE_SCOPE,
        ANALYTICS_SCOPE,
        PORTFOLIOS_SCOPE,
        REPORTING_SCOPE,
    ] {
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
                return Err(UsdPersistenceError::Hydrate(format!(
                    "duplicate prim path {} — two nodes cannot share the same USD path",
                    prim.path
                )));
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
    } else if prim.type_name == "FinancialReturnAsset" {
        prim.attributes
            .get("inputs:symbol")
            .filter(|symbol| !symbol.is_empty())
            .cloned()
            .unwrap_or_else(|| prim_leaf(&prim.path).to_string())
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
    let normalized = if key == "inputs:csv_path" {
        normalize_finance_file_path(value)
    } else {
        value.to_string()
    };
    let escaped = normalized.replace('\\', "\\\\").replace('"', "\\\"");
    let (usd_type, formatted) = if normalized == "true" || normalized == "false" {
        ("bool", normalized.clone())
    } else if key == "inputs:csv_path" {
        ("string", format!("\"{escaped}\""))
    } else if key.starts_with("info:")
        || key == "inputs:category"
        || key == "inputs:sub_category"
    {
        ("string", format!("\"{escaped}\""))
    } else if normalized.parse::<f64>().is_ok() && !normalized.is_empty() {
        ("float", normalized.clone())
    } else if normalized.parse::<u64>().is_ok() && !normalized.is_empty() {
        ("int", normalized.clone())
    } else {
        ("token", format!("\"{escaped}\""))
    };
    out.push_str(&format!("            {usd_type} {key} = {formatted}\n"));
}

fn parse_usd_string_attribute(property: &str, value: &str) -> String {
    if property == "inputs:csv_path" {
        normalize_finance_file_path(value)
    } else {
        value.trim_matches('"').to_string()
    }
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
            | "/FinancialReturnAsset"
            | "/OtlOperator"
            | "/OtlTaUberSignal"
            | "/PortfolioIntegrator"
            | "/PerformanceAnalytics"
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
        "FinancialAsset" | "FinancialReturnAsset" | "OtlOperator" | "OtlTaUberSignal" | "PortfolioIntegrator"
        | "PerformanceAnalytics" => Some(type_name),
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
                let parsed = parse_usd_string_attribute(&property, &value);
                attributes.insert(property, parsed);
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
    } else if path_str.starts_with("/analytics/")
        || path_str.starts_with(&format!("{MARKETLAB_ROOT}/Analytics/"))
    {
        Some("OtlTaUberSignal".to_string())
    } else if path_str.starts_with("/portfolios/") {
        Some("PortfolioIntegrator".to_string())
    } else if path_str.starts_with("/reporting/")
        || path_str.starts_with(&format!("{MARKETLAB_ROOT}/Reporting/"))
    {
        Some("PerformanceAnalytics".to_string())
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

/// Collapse duplicate canvas wires that serialize to the same USD lineage edge.
fn dedupe_finance_graph_connections(graph: &mut GraphDescription) {
    let mut exact = HashSet::new();
    graph.connections.retain(|connection| {
        exact.insert((
            connection.source_node.clone(),
            connection.source_pin.clone(),
            connection.target_node.clone(),
            connection.target_pin.clone(),
        ))
    });

    let mut lineage = HashSet::new();
    graph.connections.retain(|connection| {
        let Some(target_kind) = graph
            .nodes
            .get(&connection.target_node)
            .and_then(|node| FinanceNodeKind::from_graphy_type_id(&node.node_type))
        else {
            return true;
        };
        let semantic_key = match target_kind {
            FinanceNodeKind::PortfolioIntegrator
            | FinanceNodeKind::OtlOperator
            | FinanceNodeKind::OtlTaUberSignal
            | FinanceNodeKind::PerformanceAnalytics => Some((
                connection.source_node.clone(),
                connection.target_node.clone(),
            )),
            _ => None,
        };
        match semantic_key {
            Some(key) => lineage.insert(key),
            None => true,
        }
    });
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
    fn export_round_trip_preserves_duplicate_named_reporting_nodes() {
        use graphy::{GraphDescription, NodeInstance, Position};

        let mut graph = GraphDescription::new("reporting_dup");
        for id in ["report_a", "report_b"] {
            let mut node = NodeInstance::new(
                id,
                type_id::PERFORMANCE_ANALYTICS,
                Position::new(0.0, 0.0),
            );
            node.properties.insert(
                "name".to_string(),
                graphy::JsonValue::String("total".to_string()),
            );
            graph.add_node(node);
        }

        let mut doc = FinanceWorkspaceDocument::new(graph);
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("reporting_dup.usda");
        export_document(&mut doc, &path, None).expect("export");
        let reopened = import_document(&path).expect("reopen");
        assert_eq!(
            reopened.graph.nodes.len(),
            2,
            "both reporting nodes must survive save when names collide"
        );
    }

    #[test]
    fn export_round_trip_preserves_performance_reporting_nodes() {
        use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

        let mut graph = GraphDescription::new("reporting_save");
        let mut fund = NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(0.0, 0.0),
        );
        fund.properties.insert(
            "name".to_string(),
            graphy::JsonValue::String("fund".to_string()),
        );
        let mut report = NodeInstance::new(
            "perf_report",
            type_id::PERFORMANCE_ANALYTICS,
            Position::new(200.0, 0.0),
        );
        report.properties.insert(
            "name".to_string(),
            graphy::JsonValue::String("Performance Report".to_string()),
        );
        graph.add_node(fund);
        graph.add_node(report);
        graph.add_connection(Connection {
            source_node: "fund".to_string(),
            source_pin: "wealth".to_string(),
            target_node: "perf_report".to_string(),
            target_pin: "series_0".to_string(),
            connection_type: ConnectionType::Data,
        });

        let mut doc = FinanceWorkspaceDocument::new(graph);
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("reporting_roundtrip.usda");
        export_document(&mut doc, &path, None).expect("export");

        let usda = std::fs::read_to_string(&path).expect("read usda");
        assert!(
            usda.contains("def Scope \"Reporting\""),
            "reporting prims must export under Reporting scope: {usda}"
        );
        assert!(
            usda.contains("def PerformanceAnalytics"),
            "missing performance analytics prim block: {usda}"
        );
        assert!(
            usda.contains("rel inputs:series"),
            "portfolio wealth wire must round-trip as inputs:series: {usda}"
        );

        let reopened = import_document(&path).expect("reopen");
        assert_eq!(
            reopened.graph.nodes.len(),
            2,
            "expected portfolio + reporting node after round-trip"
        );
        assert!(
            reopened
                .graph
                .nodes
                .values()
                .any(|node| node.node_type == type_id::PERFORMANCE_ANALYTICS),
            "performance reporting node missing after reopen"
        );
        assert_eq!(
            reopened.graph.connections.len(),
            1,
            "wealth → report wire should survive reopen"
        );
    }

    #[test]
    fn export_round_trip_preserves_ta_analytics_nodes() {
        use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

        let mut graph = GraphDescription::new("ta_save");
        let mut spy = NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        );
        spy.properties.insert(
            "symbol".to_string(),
            graphy::JsonValue::String("SPY".to_string()),
        );
        graph.add_node(spy);
        graph.add_node(NodeInstance::new(
            "ta_ear",
            type_id::TA_TREND,
            Position::new(120.0, 0.0),
        ));
        let mut fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(240.0, 0.0),
        );
        fund.properties.insert(
            "name".to_string(),
            graphy::JsonValue::String("ear_fund".to_string()),
        );
        graph.add_node(fund);
        graph.add_connection(Connection {
            source_node: "spy".to_string(),
            source_pin: "close".to_string(),
            target_node: "ta_ear".to_string(),
            target_pin: "source_stream".to_string(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_ear".to_string(),
            source_pin: "result".to_string(),
            target_node: "ear_fund".to_string(),
            target_pin: "signal_0".to_string(),
            connection_type: ConnectionType::Data,
        });

        let mut doc = FinanceWorkspaceDocument::new(graph);
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("ta_roundtrip.usda");
        export_document(&mut doc, &path, None).expect("export");

        let usda = std::fs::read_to_string(&path).expect("read usda");
        assert!(
            usda.contains("def Scope \"Analytics\""),
            "TA prims must export under Analytics scope: {usda}"
        );
        assert!(
            usda.contains("def OtlTaUberSignal \"ta_ear\""),
            "missing TA prim block: {usda}"
        );

        let reopened = import_document(&path).expect("reopen");
        assert_eq!(
            reopened.graph.nodes.len(),
            3,
            "expected asset + TA + portfolio after round-trip"
        );
        assert!(
            reopened
                .graph
                .nodes
                .values()
                .any(|node| node.node_type == type_id::TA_TREND),
            "TA node missing after reopen"
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
