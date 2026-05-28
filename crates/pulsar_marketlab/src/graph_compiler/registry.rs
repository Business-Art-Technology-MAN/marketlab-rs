//! Three-tiered node registry taxonomy and port wiring validation.

use super::{PipelineGraphSnapshot, VisualNode};

/// Explicit architectural tier for each canvas node.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    /// Tier 1: non-executable OpenUSD structural adaptor (prim or layer path).
    AssetAdaptor { prim_path: String },
    /// Tier 2: executable OTL shader closure or stdlib node (`mix`, `clamp`, `step`).
    OtlShader { script: String },
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
        Self::asset_adaptor(format!("/assets/{ticker}"))
    }

    pub fn asset_adaptor_from_csv_path(path: &str) -> Self {
        let stem = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .trim_end_matches(".csv");
        Self::asset_adaptor(format!("/assets/{}", stem.to_ascii_uppercase()))
    }

    pub fn otl_shader(script: impl Into<String>) -> Self {
        Self::OtlShader {
            script: script.into(),
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
            Self::OtlShader { script } => Some(script.as_str()),
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

pub fn sync_otl_shader_aov_ports(node: &mut VisualNode) {
    if !node.node_type.is_otl_shader() {
        return;
    }
    let numeric_label = node
        .outputs
        .first()
        .cloned()
        .unwrap_or_else(|| "TA Out".to_string());
    node.outputs = vec![numeric_label];
    for aov in &node.aov_outputs {
        node.outputs.push(format!("AOV: {aov}"));
    }
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
        NodeType::OtlShader { .. } => {
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
            | (NodeType::OtlShader { .. }, NodeType::OtlShader { .. })
            | (NodeType::OtlShader { .. }, NodeType::TerminalIntegrator { .. })
    )
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
    wire_kinds_compatible(from_kind, to_kind) && tier_topology_allows(from_node, to_node)
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
