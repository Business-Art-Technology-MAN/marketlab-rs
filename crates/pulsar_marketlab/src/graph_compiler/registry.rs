//! Three-tiered node registry taxonomy and port wiring validation.

use pulsar_marketlab_core::{
    display_name_for_script, infer_archetype_from_algorithm, load_compiled_asset_from_path,
    parse_script_signature, resolve_otl_script_src, ScriptSignature, TaArchetype,
    TaUberSignalConfig, OtlScriptContext,
};

use super::{NodeConnection, PipelineGraphSnapshot, VisualNode};

/// Explicit architectural tier for each canvas node.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    /// Tier 1: non-executable OpenUSD structural adaptor (prim or layer path).
    AssetAdaptor { prim_path: String },
    /// Tier 2: generic OTL formula node (`mix`, `clamp`, custom expressions).
    OtlShader {
        script: String,
        /// Optional `.otc` cache path (`inputs:script_compiled_path` on stage).
        compiled_path: Option<String>,
    },
    /// Tier 2: unified TA uber-signal (immutable ports per archetype).
    TaUberSignal { config: TaUberSignalConfig },
    /// Tier 3: terminal exporter / integrator (`portfolio`, `vector_ta`, `spreadsheet`).
    TerminalIntegrator { engine_target: String },
}

impl NodeType {
    pub fn asset_adaptor(prim_path: impl Into<String>) -> Self {
        Self::AssetAdaptor {
            prim_path: prim_path.into(),
        }
    }

    pub fn asset_adaptor_from_label(label: &str) -> Self {
        let ticker = label
            .split_whitespace()
            .find(|token| !token.eq_ignore_ascii_case("csv") && !token.eq_ignore_ascii_case("asset"))
            .unwrap_or("ASSET");
        Self::asset_adaptor(format!("/MarketLab/{ticker}"))
    }

    pub fn asset_adaptor_from_csv_path(path: &str) -> Self {
        let stem = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .trim_end_matches(".csv");
        Self::asset_adaptor(format!("/MarketLab/{}", stem.to_ascii_uppercase()))
    }

    pub fn otl_shader(script: impl Into<String>) -> Self {
        Self::OtlShader {
            script: script.into(),
            compiled_path: None,
        }
    }

    pub fn otl_compiled_path(&self) -> Option<&str> {
        match self {
            Self::OtlShader { compiled_path, .. } => compiled_path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty()),
            _ => None,
        }
    }

    pub fn ta_uber_signal(config: TaUberSignalConfig) -> Self {
        Self::TaUberSignal { config }
    }

    pub fn ta_uber_signal_new(archetype: TaArchetype) -> Self {
        Self::TaUberSignal {
            config: TaUberSignalConfig::new(archetype),
        }
    }

    pub fn terminal_integrator(engine_target: impl Into<String>) -> Self {
        Self::TerminalIntegrator {
            engine_target: engine_target.into(),
        }
    }

    pub fn portfolio() -> Self {
        Self::terminal_integrator("portfolio")
    }

    pub fn is_asset_adaptor(&self) -> bool {
        matches!(self, Self::AssetAdaptor { .. })
    }

    pub fn is_otl_shader(&self) -> bool {
        matches!(self, Self::OtlShader { .. })
    }

    pub fn is_ta_uber_signal(&self) -> bool {
        matches!(self, Self::TaUberSignal { .. })
    }

    pub fn is_executable_signal(&self) -> bool {
        self.is_otl_shader() || self.is_ta_uber_signal()
    }

    #[allow(dead_code)]
    pub fn is_terminal_integrator(&self) -> bool {
        matches!(self, Self::TerminalIntegrator { .. })
    }

    pub fn is_portfolio(&self) -> bool {
        matches!(
            self,
            Self::TerminalIntegrator { engine_target } if engine_target == "portfolio"
        )
    }

    pub fn displays_price_chart(&self) -> bool {
        self.is_asset_adaptor()
    }

    #[allow(dead_code)]
    pub fn prim_path(&self) -> Option<&str> {
        match self {
            Self::AssetAdaptor { prim_path } => Some(prim_path.as_str()),
            _ => None,
        }
    }

    pub fn script(&self) -> Option<&str> {
        match self {
            Self::OtlShader { script, .. } => Some(script.as_str()),
            _ => None,
        }
    }

    pub fn ta_uber_config(&self) -> Option<&TaUberSignalConfig> {
        match self {
            Self::TaUberSignal { config } => Some(config),
            _ => None,
        }
    }

    pub fn ta_uber_config_mut(&mut self) -> Option<&mut TaUberSignalConfig> {
        match self {
            Self::TaUberSignal { config } => Some(config),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn engine_target(&self) -> Option<&str> {
        match self {
            Self::TerminalIntegrator { engine_target } => Some(engine_target.as_str()),
            _ => None,
        }
    }
}

/// Semantic wire payload carried by a node port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortWireKind {
    /// Tier 1 OpenUSD prim or layer path reference.
    StructuralPath,
    /// Tier 2 numeric OTL signal (scalar/vector).
    NumericSignal,
    /// Tier 2 arbitrary output variable exported by an OTL shader.
    Aov,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireValidationError {
    pub from_node_id: usize,
    pub to_node_id: usize,
    pub message: String,
}

/// Apply immutable TA port labels for the node's archetype (never call on algorithm change).
pub fn apply_canonical_ta_ports(node: &mut VisualNode) {
    let Some(config) = node.node_type.ta_uber_config() else {
        return;
    };
    node.inputs = config
        .archetype
        .canonical_input_ports()
        .iter()
        .map(|label| (*label).to_string())
        .collect();
    node.outputs = config
        .archetype
        .canonical_output_ports()
        .iter()
        .map(|label| (*label).to_string())
        .collect();
}

pub fn sync_otl_shader_aov_ports(node: &mut VisualNode) {
    if !node.node_type.is_otl_shader() {
        return;
    }
    let numeric_label = node
        .outputs
        .first()
        .cloned()
        .unwrap_or_else(|| "signal".to_string());
    node.outputs = vec![numeric_label];
    for aov in &node.aov_outputs {
        node.outputs.push(format!("AOV: {aov}"));
    }
}

/// Rebuild OTL shader ports from a compiled `.otc` manifest JSON block.
pub fn sync_otl_shader_ports_from_manifest(
    node: &mut VisualNode,
    manifest_json: &str,
    connections: &mut Vec<NodeConnection>,
) -> Vec<WireValidationError> {
    let signature = parse_manifest_ports(manifest_json);
    let pseudo_script = if signature.inputs.is_empty() && signature.outputs.is_empty() {
        String::new()
    } else {
        format!(
            "void manifest_stub({}) {{}}",
            signature
                .inputs
                .iter()
                .chain(signature.outputs.iter())
                .map(|name| format!("float {name}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    sync_otl_shader_ports_from_script(node, &pseudo_script, connections)
}

fn parse_manifest_ports(manifest_json: &str) -> ScriptSignature {
    let mut signature = ScriptSignature::default();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(manifest_json) else {
        return signature;
    };
    if let Some(inputs) = value.get("inputs").and_then(|v| v.as_array()) {
        for entry in inputs {
            if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
                signature.inputs.push(name.to_string());
            }
        }
    }
    if let Some(outputs) = value.get("outputs").and_then(|v| v.as_array()) {
        for entry in outputs {
            if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
                signature.outputs.push(name.to_string());
            }
        }
    }
    signature
}

/// Apply a validated `.otc` asset to an OTL shader node (ports + script slot, no source lex pass).
pub fn apply_compiled_otc_asset_to_node(
    node: &mut VisualNode,
    asset: &pulsar_marketlab_core::OtcCompiledAsset,
    connections: &mut Vec<NodeConnection>,
) -> Result<Vec<WireValidationError>, pulsar_marketlab_core::OtcError> {
    if !node.node_type.is_otl_shader() {
        return Ok(Vec::new());
    }
    let script = asset.bytecode_as_script_source()?.to_string();
    if let NodeType::OtlShader { script: slot, .. } = &mut node.node_type {
        *slot = script.clone();
    }
    node.dsl_formula = Some(script.clone());
    node.name = display_name_for_script(&script, &node.name);
    Ok(sync_otl_shader_ports_from_manifest(
        node,
        &asset.manifest_json,
        connections,
    ))
}

/// Rebuild OTL shader input/output sockets from an OSL/C-style script signature.
pub fn sync_otl_shader_ports_from_script(
    node: &mut VisualNode,
    script: &str,
    connections: &mut Vec<NodeConnection>,
) -> Vec<WireValidationError> {
    if !node.node_type.is_otl_shader() {
        return Vec::new();
    }

    let signature = parse_script_signature(script);
    node.name = display_name_for_script(script, &node.name);
    if !signature.inputs.is_empty() {
        node.inputs = signature.inputs.clone();
    } else if node.inputs.is_empty() {
        node.inputs = vec!["source_stream".to_string()];
    }

    let numeric_outputs = if signature.outputs.is_empty() {
        vec![node
            .outputs
            .first()
            .cloned()
            .unwrap_or_else(|| "signal".to_string())]
    } else {
        signature.outputs.clone()
    };
    node.outputs = numeric_outputs;
    sync_otl_shader_aov_ports(node);

    let node_id = node.id;
    let input_port_count = node.inputs.len();
    let numeric_output_count = node
        .outputs
        .len()
        .saturating_sub(node.aov_outputs.len());

    let mut errors = Vec::new();
    connections.retain(|connection| {
        if connection.from_node_id == node_id && connection.from_port_idx >= numeric_output_count {
            errors.push(WireValidationError {
                from_node_id: connection.from_node_id,
                to_node_id: connection.to_node_id,
                message: format!(
                    "pruned stale output wire on port {} after OTL signature change",
                    connection.from_port_idx
                ),
            });
            return false;
        }
        if connection.to_node_id == node_id && connection.to_port_idx >= input_port_count {
            errors.push(WireValidationError {
                from_node_id: connection.from_node_id,
                to_node_id: connection.to_node_id,
                message: format!(
                    "pruned stale input wire on port {} after OTL signature change",
                    connection.to_port_idx
                ),
            });
            return false;
        }
        true
    });
    errors
}

pub fn otl_script_context(node: &VisualNode) -> OtlScriptContext<'_> {
    OtlScriptContext {
        dsl_formula: node.dsl_formula.as_deref(),
        node_script: node.node_type.script(),
        indicator_id: node
            .node_type
            .ta_uber_config()
            .map(|config| config.algorithm.as_str()),
        lookback_period: node
            .node_type
            .ta_uber_config()
            .map(|config| config.period)
            .unwrap_or(14),
        uber_config: node.node_type.ta_uber_config(),
    }
}

/// Canonical OTL source for canvas, USDA, playhead eval, and graph engine.
pub fn resolved_otl_script(node: &VisualNode) -> String {
    if let Some(path) = node.node_type.otl_compiled_path() {
        if let Ok(asset) = load_compiled_asset_from_path(path) {
            if let Ok(script) = asset.bytecode_as_script_source() {
                return script.to_string();
            }
        }
    }
    resolve_otl_script_src(&otl_script_context(node))
}

pub fn effective_otl_script(node: &VisualNode) -> Option<&str> {
    if let Some(formula) = node
        .dsl_formula
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(formula);
    }
    node.node_type.script().filter(|text| !text.is_empty())
}

pub fn output_port_kind(node: &VisualNode, port_idx: usize) -> Option<PortWireKind> {
    if port_idx >= node.outputs.len() {
        return None;
    }
    match &node.node_type {
        NodeType::AssetAdaptor { .. } => Some(PortWireKind::StructuralPath),
        NodeType::TaUberSignal { config } => {
            let numeric_outputs = config.archetype.canonical_output_ports().len();
            if port_idx < numeric_outputs {
                Some(PortWireKind::NumericSignal)
            } else {
                Some(PortWireKind::Aov)
            }
        }
        NodeType::OtlShader { .. } => {
            let numeric_outputs = node
                .outputs
                .len()
                .saturating_sub(node.aov_outputs.len());
            if port_idx < numeric_outputs {
                Some(PortWireKind::NumericSignal)
            } else {
                Some(PortWireKind::Aov)
            }
        }
        NodeType::TerminalIntegrator { .. } => Some(PortWireKind::NumericSignal),
    }
}

pub fn input_port_kind(node: &VisualNode, port_idx: usize) -> Option<PortWireKind> {
    if port_idx >= node.inputs.len() {
        return None;
    }
    match &node.node_type {
        NodeType::AssetAdaptor { .. } => None,
        NodeType::TaUberSignal { .. } | NodeType::OtlShader { .. } => {
            if port_idx == 0 {
                Some(PortWireKind::StructuralPath)
            } else {
                Some(PortWireKind::NumericSignal)
            }
        }
        NodeType::TerminalIntegrator { .. } => {
            let label = node.inputs.get(port_idx)?.to_ascii_lowercase();
            if label.starts_with("aov:") || label.starts_with("aov ") {
                Some(PortWireKind::Aov)
            } else {
                Some(PortWireKind::NumericSignal)
            }
        }
    }
}

pub fn wire_kinds_compatible(from: PortWireKind, to: PortWireKind) -> bool {
    from == to
}

pub fn tier_topology_allows(from: &VisualNode, to: &VisualNode) -> bool {
    matches!(
        (&from.node_type, &to.node_type),
        (NodeType::AssetAdaptor { .. }, NodeType::OtlShader { .. })
            | (NodeType::AssetAdaptor { .. }, NodeType::TaUberSignal { .. })
            | (NodeType::OtlShader { .. }, NodeType::OtlShader { .. })
            | (NodeType::OtlShader { .. }, NodeType::TaUberSignal { .. })
            | (NodeType::TaUberSignal { .. }, NodeType::OtlShader { .. })
            | (NodeType::TaUberSignal { .. }, NodeType::TaUberSignal { .. })
            | (NodeType::OtlShader { .. }, NodeType::TerminalIntegrator { .. })
            | (NodeType::TaUberSignal { .. }, NodeType::TerminalIntegrator { .. })
    ) || (from.node_type.is_asset_adaptor() && to.node_type.is_portfolio())
        || (from.node_type.is_portfolio() && to.node_type.is_portfolio())
}

pub fn connection_is_valid(
    from_node: &VisualNode,
    from_port_idx: usize,
    to_node: &VisualNode,
    to_port_idx: usize,
) -> bool {
    let Some(from_kind) = output_port_kind(from_node, from_port_idx) else {
        return false;
    };
    let Some(to_kind) = input_port_kind(to_node, to_port_idx) else {
        return false;
    };
    let kinds_compatible = wire_kinds_compatible(from_kind, to_kind)
        || (from_kind == PortWireKind::StructuralPath
            && to_kind == PortWireKind::NumericSignal
            && from_node.node_type.is_asset_adaptor()
            && (to_node.node_type.is_executable_signal() || to_node.node_type.is_portfolio()))
        || (from_kind == PortWireKind::NumericSignal
            && to_kind == PortWireKind::NumericSignal
            && from_node.node_type.is_ta_uber_signal()
            && to_node.node_type.is_ta_uber_signal());
    kinds_compatible && tier_topology_allows(from_node, to_node)
}

pub fn validate_graph_wiring(snapshot: &PipelineGraphSnapshot) -> Vec<WireValidationError> {
    let nodes_by_id: std::collections::HashMap<usize, &VisualNode> =
        snapshot.nodes.iter().map(|node| (node.id, node)).collect();

    snapshot
        .connections
        .iter()
        .filter_map(|connection| {
            let from_node = nodes_by_id.get(&connection.from_node_id)?;
            let to_node = nodes_by_id.get(&connection.to_node_id)?;
            if connection_is_valid(from_node, connection.from_port_idx, to_node, connection.to_port_idx)
            {
                return None;
            }

            let from_kind = output_port_kind(from_node, connection.from_port_idx);
            let to_kind = input_port_kind(to_node, connection.to_port_idx);
            let message = match (from_kind, to_kind) {
                (Some(from_kind), Some(to_kind)) if from_kind != to_kind => format!(
                    "incompatible wire kinds: {from_kind:?} -> {to_kind:?} (structural paths cannot feed numeric ports)"
                ),
                _ => format!(
                    "invalid tier topology or port index (from {}:{} -> {}:{})",
                    connection.from_node_id,
                    connection.from_port_idx,
                    connection.to_node_id,
                    connection.to_port_idx
                ),
            };

            Some(WireValidationError {
                from_node_id: connection.from_node_id,
                to_node_id: connection.to_node_id,
                message,
            })
        })
        .collect()
}

/// Return only connections that pass tier and port wire-kind validation.
pub fn validated_connections(snapshot: &PipelineGraphSnapshot) -> Vec<NodeConnection> {
    let nodes_by_id: std::collections::HashMap<usize, &VisualNode> =
        snapshot.nodes.iter().map(|node| (node.id, node)).collect();

    snapshot
        .connections
        .iter()
        .filter(|connection| {
            let Some(from_node) = nodes_by_id.get(&connection.from_node_id) else {
                return false;
            };
            let Some(to_node) = nodes_by_id.get(&connection.to_node_id) else {
                return false;
            };
            connection_is_valid(
                from_node,
                connection.from_port_idx,
                to_node,
                connection.to_port_idx,
            )
        })
        .cloned()
        .collect()
}

/// Build a TA uber node from a legacy indicator id (hydrate / migration).
pub fn ta_uber_from_legacy_indicator(indicator_id: &str, period: u32) -> TaUberSignalConfig {
    let archetype = infer_archetype_from_algorithm(indicator_id);
    let mut config = TaUberSignalConfig::new(archetype);
    config.algorithm = indicator_id.to_string();
    config.period = period.max(1);
    config.normalize_algorithm();
    config
}

