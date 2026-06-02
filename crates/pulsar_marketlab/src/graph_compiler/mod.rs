//! Graph configuration, shared snapshots, and DAG compilation.

mod otl_stdlib_catalog;
mod otl_registry;
mod registry;

#[cfg(test)]
mod tests;

pub use otl_stdlib_catalog::{OtlStdlibPreset, OTL_STDLIB_PRESETS};
pub use otl_registry::{
    otl_object_kind_for_node, otl_object_registry_snapshot, register_otl_object,
    register_otl_object_from_source, OtlObjectRegistry, RegistryError,
};
pub use registry::{
    apply_canonical_ta_ports, connection_is_valid, effective_otl_script, input_port_kind,
    output_port_kind, otl_script_context, resolved_otl_script, sync_otl_shader_aov_ports,
    apply_compiled_otc_asset_to_node, sync_otl_shader_ports_from_manifest,
    sync_otl_shader_ports_from_script, ta_uber_from_legacy_indicator, validate_graph_wiring,
    validated_connections, NodeType, PortWireKind, WireValidationError,
};
pub use pulsar_marketlab_core::{
    algorithm_display_label, hyperparameter_visibility, node_display_name, TaArchetype,
    TaHyperparamVisibility, TaUberSignalConfig,
};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use pulsar_marketlab::execution_engine::ExecutionGraph;
use pulsar_marketlab::signal_dsl::evaluate_formula;
use pulsar_marketlab::technical_analysis::{
    clamp_ta_lookback, MarketSeriesWindow,
};

pub(crate) fn is_price_source_node(node: &VisualNode) -> bool {
    node.node_type.is_asset_adaptor() || node.node_type.is_portfolio()
}

pub(crate) fn upstream_node_at_port(
    node_id: usize,
    port_idx: usize,
    connections: &[NodeConnection],
) -> Option<usize> {
    connections
        .iter()
        .find(|connection| connection.to_node_id == node_id && connection.to_port_idx == port_idx)
        .map(|connection| connection.from_node_id)
}

pub(crate) fn upstream_price_source_node_id_parts(
    node_id: usize,
    port_idx: usize,
    nodes: &[VisualNode],
    connections: &[NodeConnection],
) -> Option<usize> {
    let upstream_id = upstream_node_at_port(node_id, port_idx, connections)?;
    nodes
        .iter()
        .find(|node| node.id == upstream_id && is_price_source_node(node))
        .map(|node| node.id)
}

pub fn csv_backed_asset_ids(graph: &PipelineGraphSnapshot) -> HashSet<usize> {
    graph
        .nodes
        .iter()
        .filter(|node| node.node_type.is_asset_adaptor())
        .filter(|node| matches!(node.asset_source, Some(AssetSourceType::Csv { .. })))
        .map(|node| node.id)
        .collect()
}

pub fn portfolio_signal_port_label(port_idx: usize) -> String {
    format!("Signal In {port_idx}")
}

/// OTL shader node ids with an active wire into any portfolio integrator socket.
pub fn portfolio_wired_ta_node_ids(graph: &PipelineGraphSnapshot) -> HashSet<usize> {
    graph
        .connections
        .iter()
        .filter_map(|connection| {
            let to_node = graph
                .nodes
                .iter()
                .find(|node| node.id == connection.to_node_id)?;
            if !to_node.node_type.is_portfolio() {
                return None;
            }
            let from_node = graph
                .nodes
                .iter()
                .find(|node| node.id == connection.from_node_id)?;
            if from_node.node_type.is_ta_uber_signal() {
                Some(from_node.id)
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct PipelineGraphSnapshot {
    pub nodes: Vec<VisualNode>,
    pub connections: Vec<NodeConnection>,
    /// Topologically sorted node ids for deterministic execution (parents before children).
    pub execution_order: Vec<usize>,
    /// False when the canvas contains a dependency cycle.
    pub dag_valid: bool,
    /// False when any connection violates tier or port wire-kind rules.
    pub wiring_valid: bool,
    pub wiring_errors: Vec<WireValidationError>,
}

pub fn compile_graph_to_dag(snapshot: &PipelineGraphSnapshot) -> (Vec<usize>, bool) {
    let mut graph = ExecutionGraph::new();
    for node in &snapshot.nodes {
        graph.add_node(node.id.to_string());
    }
    for connection in &snapshot.connections {
        graph.add_edge(
            connection.from_node_id.to_string(),
            connection.to_node_id.to_string(),
        );
    }
    match graph.compile_execution_order() {
        Ok(order) => (
            order
                .iter()
                .filter_map(|id| id.parse::<usize>().ok())
                .collect(),
            true,
        ),
        Err(_) => (Vec::new(), false),
    }
}

/// Ensure portfolio nodes declare enough `Signal In N` sockets for wired connections.
pub fn sync_portfolio_input_ports_from_connections(
    nodes: &mut [VisualNode],
    connections: &[NodeConnection],
) {
    for node in nodes.iter_mut() {
        if !node.node_type.is_portfolio() {
            continue;
        }
        let max_used_port = connections
            .iter()
            .filter(|connection| connection.to_node_id == node.id)
            .map(|connection| connection.to_port_idx)
            .max();
        let minimum_ports = match max_used_port {
            Some(max_port) => max_port + 2,
            None => 1,
        };
        while node.inputs.len() < minimum_ports {
            node.inputs.push(portfolio_signal_port_label(node.inputs.len()));
        }
    }
}

fn finalize_snapshot(mut snapshot: PipelineGraphSnapshot) -> PipelineGraphSnapshot {
    sync_portfolio_input_ports_from_connections(&mut snapshot.nodes, &snapshot.connections);
    let portfolio_ids: Vec<usize> = snapshot
        .nodes
        .iter()
        .filter(|node| node.node_type.is_portfolio())
        .map(|node| node.id)
        .collect();
    for portfolio_id in portfolio_ids {
        portfolio_ensure_spare_input_port(&mut snapshot.nodes, &snapshot.connections, portfolio_id);
    }
    let wiring_errors = validate_graph_wiring(&snapshot);
    snapshot.wiring_valid = wiring_errors.is_empty();
    snapshot.wiring_errors = wiring_errors;
    let (execution_order, dag_valid) = compile_graph_to_dag(&snapshot);
    snapshot.execution_order = execution_order;
    snapshot.dag_valid = dag_valid;
    snapshot
}

pub fn upstream_asset_for_ta_node(ta_node_id: usize, graph: &PipelineGraphSnapshot) -> Option<usize> {
    graph.connections.iter().find_map(|connection| {
        if connection.to_node_id != ta_node_id {
            return None;
        }
        let asset = graph
            .nodes
            .iter()
            .find(|node| node.id == connection.from_node_id)?;
        if asset.node_type.is_asset_adaptor() {
            Some(asset.id)
        } else {
            None
        }
    })
}

#[derive(Debug)]
struct PipelineGraphState {
    snapshot: PipelineGraphSnapshot,
    revision: u64,
}

#[derive(Clone)]
pub struct SharedPipelineGraph(Arc<Mutex<PipelineGraphState>>);

impl SharedPipelineGraph {
    pub fn new(nodes: Vec<VisualNode>, connections: Vec<NodeConnection>) -> Self {
        let snapshot = finalize_snapshot(PipelineGraphSnapshot {
            nodes,
            connections,
            execution_order: Vec::new(),
            dag_valid: true,
            wiring_valid: true,
            wiring_errors: Vec::new(),
        });
        Self(Arc::new(Mutex::new(PipelineGraphState {
            snapshot,
            revision: 0,
        })))
    }

    pub fn replace(&self, nodes: Vec<VisualNode>, connections: Vec<NodeConnection>) {
        if let Ok(mut guard) = self.0.lock() {
            guard.revision = guard.revision.saturating_add(1);
            guard.snapshot = finalize_snapshot(PipelineGraphSnapshot {
                nodes,
                connections,
                execution_order: Vec::new(),
                dag_valid: true,
                wiring_valid: true,
                wiring_errors: Vec::new(),
            });
        }
    }

    pub fn revision(&self) -> u64 {
        self.0
            .lock()
            .map(|guard| guard.revision)
            .unwrap_or(0)
    }

    pub fn snapshot(&self) -> PipelineGraphSnapshot {
        self.0
            .lock()
            .map(|guard| guard.snapshot.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().snapshot.clone())
    }
}

/// Thread-safe CSV path registry shared between the UI workspace and the asset feeder.
#[derive(Clone)]
pub struct SharedCsvAssetPaths(Arc<Mutex<CsvAssetPathRegistry>>);

#[derive(Debug, Default)]
struct CsvAssetPathRegistry {
    paths: HashMap<usize, String>,
    revision: u64,
}

impl SharedCsvAssetPaths {
    pub fn from_nodes(nodes: &[VisualNode]) -> Self {
        let mut paths = HashMap::new();
        for node in nodes {
            if let Some(AssetSourceType::Csv { path }) = &node.asset_source {
                paths.insert(node.id, path.clone());
            }
        }
        Self(Arc::new(Mutex::new(CsvAssetPathRegistry { paths, revision: 0 })))
    }

    pub fn set_path(&self, node_id: usize, path: String) {
        if let Ok(mut guard) = self.0.lock() {
            guard.revision = guard.revision.saturating_add(1);
            guard.paths.insert(node_id, path);
        }
    }

    pub fn revision(&self) -> u64 {
        self.0
            .lock()
            .map(|guard| guard.revision)
            .unwrap_or(0)
    }

    pub fn snapshot(&self) -> HashMap<usize, String> {
        self.0
            .lock()
            .map(|guard| guard.paths.clone())
            .unwrap_or_default()
    }

    pub fn replace_from_nodes(&self, nodes: &[VisualNode]) {
        if let Ok(mut guard) = self.0.lock() {
            guard.revision = guard.revision.saturating_add(1);
            guard.paths.clear();
            for node in nodes {
                if let Some(AssetSourceType::Csv { path }) = &node.asset_source {
                    guard.paths.insert(node.id, path.clone());
                }
            }
        }
    }
}

pub const NODE_CHART_HEIGHT: f32 = 52.0;

pub(crate) const NODE_WIDTH: f32 = 220.0;
pub(crate) const NODE_HEADER_HEIGHT: f32 = 36.0;
const NODE_GRADE_HEIGHT: f32 = 32.0;
const NODE_OTL_PARAM_ROW_HEIGHT: f32 = 26.0;
const NODE_OTL_PARAM_HEADER_HEIGHT: f32 = 18.0;
pub(crate) const NODE_TA_LABEL_HEIGHT: f32 = 20.0;
pub(crate) const NODE_PORTFOLIO_METRICS_HEIGHT: f32 = 72.0;
pub(crate) const NODE_PORTS_PADDING: f32 = 8.0;
pub(crate) const PORT_ROW_HEIGHT: f32 = 24.0;
pub(crate) const WIRE_PORT_HIT_RADIUS: f32 = 22.0;
pub(crate) const CONNECTION_STROKE_WIDTH: f32 = 2.0;
pub(crate) const MIN_ZOOM: f32 = 0.15;
pub(crate) const MAX_ZOOM: f32 = 3.0;
pub(crate) const ZOOM_WHEEL_SENSITIVITY: f32 = 0.002;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum NodeGradeType {
    Scalar,
    Vector,
    Trivector,
}

/// Polymorphic asset ingestion source bound to a visual pipeline node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetSourceType {
    Csv { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualNode {
    pub id: usize,
    pub name: String,
    pub node_type: NodeType,
    pub grade: NodeGradeType,
    /// Portfolio allocation strategy token (`inputs:id` on portfolio prims).
    pub portfolio_allocation_id: Option<String>,
    /// OSL-inspired formula evaluated against the upstream market window when set.
    pub dsl_formula: Option<String>,
    /// OTL arbitrary output variable names exposed as dedicated AOV output ports.
    pub aov_outputs: Vec<String>,
    /// When set, a background CSV playback loop streams Yahoo Finance rows for this node.
    pub asset_source: Option<AssetSourceType>,
    pub x: f32,
    pub y: f32,
    /// Blender capsule mode: pill shell only, active sockets on perimeter.
    pub collapsed: bool,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConnection {
    pub from_node_id: usize,
    pub from_port_idx: usize,
    pub to_node_id: usize,
    pub to_port_idx: usize,
}

pub(crate) fn portfolio_input_port_free(
    connections: &[NodeConnection],
    portfolio_id: usize,
    port_idx: usize,
) -> bool {
    !connections
        .iter()
        .any(|connection| connection.to_node_id == portfolio_id && connection.to_port_idx == port_idx)
}

pub(crate) fn portfolio_resolve_input_port(
    nodes: &mut Vec<VisualNode>,
    connections: &[NodeConnection],
    portfolio_id: usize,
    preferred_port: usize,
) -> Option<usize> {
    let inputs_len = nodes.iter().find(|node| node.id == portfolio_id)?.inputs.len();
    if preferred_port < inputs_len
        && portfolio_input_port_free(connections, portfolio_id, preferred_port)
    {
        return Some(preferred_port);
    }
    for port_idx in 0..inputs_len {
        if portfolio_input_port_free(connections, portfolio_id, port_idx) {
            return Some(port_idx);
        }
    }
    let node = nodes.iter_mut().find(|node| node.id == portfolio_id)?;
    let port_idx = node.inputs.len();
    node.inputs.push(portfolio_signal_port_label(port_idx));
    Some(port_idx)
}

pub(crate) fn portfolio_ensure_spare_input_port(
    nodes: &mut Vec<VisualNode>,
    connections: &[NodeConnection],
    portfolio_id: usize,
) {
    let Some(node) = nodes
        .iter()
        .find(|node| node.id == portfolio_id && node.node_type.is_portfolio())
    else {
        return;
    };
    let all_wired = node.inputs.iter().enumerate().all(|(port_idx, _)| {
        !portfolio_input_port_free(connections, portfolio_id, port_idx)
    });
    if all_wired {
        let port_idx = node.inputs.len();
        if let Some(node) = nodes.iter_mut().find(|node| node.id == portfolio_id) {
            node.inputs.push(portfolio_signal_port_label(port_idx));
        }
    }
}

pub(crate) fn portfolio_wired_source_count(connections: &[NodeConnection], portfolio_id: usize) -> usize {
    connections
        .iter()
        .filter(|connection| connection.to_node_id == portfolio_id)
        .count()
}

pub(crate) fn ta_lookback_for_node(node: &VisualNode) -> usize {
    let period = node
        .overlay_period()
        .unwrap_or(14) as usize;
    clamp_ta_lookback(period)
}

pub fn ta_compute_for_node(node: &VisualNode, window: &MarketSeriesWindow) -> Option<f64> {
    if !node.node_type.is_executable_signal() {
        return None;
    }
    let lookback = ta_lookback_for_node(node);
    let script = resolved_otl_script(node);
    evaluate_formula(&script, window, lookback)
        .ok()
        .map(f64::from)
}

impl VisualNode {
    /// Active algorithm id from `TaUberSignalConfig` (chart / engine overlay vocabulary).
    pub fn overlay_algorithm(&self) -> Option<&str> {
        self.node_type
            .ta_uber_config()
            .map(|config| config.algorithm.as_str())
    }

    /// Hyperparameter period from `TaUberSignalConfig`.
    pub fn overlay_period(&self) -> Option<u32> {
        self.node_type.ta_uber_config().map(|config| config.period)
    }

    pub fn set_overlay_algorithm(&mut self, algorithm: impl Into<String>) {
        if let Some(config) = self.node_type.ta_uber_config_mut() {
            config.algorithm = algorithm.into();
            config.normalize_algorithm();
        }
    }

    pub fn set_overlay_period(&mut self, period: u32) {
        if let Some(config) = self.node_type.ta_uber_config_mut() {
            config.period = period.max(1);
        }
    }
}

pub const NODE_SPAWN_STAGGER_X: f32 = 260.0;
pub const NODE_SPAWN_STAGGER_Y: f32 = 48.0;

pub(crate) fn node_shows_price_chart(node: &VisualNode) -> bool {
    node.node_type.displays_price_chart()
}

pub(crate) fn node_body_height_world(node: &VisualNode, include_chart: bool) -> f32 {
    let mut height = NODE_HEADER_HEIGHT + NODE_GRADE_HEIGHT;
    if node.node_type.is_ta_uber_signal() {
        height += NODE_TA_LABEL_HEIGHT;
    }
    if node.node_type.is_otl_shader() {
        if let Some(script) = effective_otl_script(node) {
            let uniform_count = pulsar_marketlab_core::parse_script_scalar_uniforms(script).len();
            if uniform_count > 0 {
                height += NODE_OTL_PARAM_HEADER_HEIGHT + uniform_count as f32 * NODE_OTL_PARAM_ROW_HEIGHT;
            }
        }
    }
    if node.node_type.is_portfolio() {
        height += NODE_PORTFOLIO_METRICS_HEIGHT;
    }
    if include_chart && node_shows_price_chart(node) {
        height += NODE_CHART_HEIGHT;
    }
    height
}

pub(crate) fn input_port_world_center(
    node: &VisualNode,
    port_idx: usize,
    include_chart: bool,
    connections: &[NodeConnection],
) -> (f32, f32) {
    if node.collapsed {
        let connected: Vec<usize> = connections
            .iter()
            .filter(|connection| connection.to_node_id == node.id)
            .map(|connection| connection.to_port_idx)
            .collect();
        if let Some(slot) = connected.iter().position(|&idx| idx == port_idx) {
            return pulsar_marketlab_ui::theme::capsule_socket_world_center(
                node.x,
                node.y,
                pulsar_marketlab_ui::theme::CapsuleSocketSide::Input,
                slot,
                connected.len(),
            );
        }
        return pulsar_marketlab_ui::theme::capsule_socket_world_center(
            node.x,
            node.y,
            pulsar_marketlab_ui::theme::CapsuleSocketSide::Input,
            0,
            1,
        );
    }
    let y = node.y
        + node_body_height_world(node, include_chart)
        + NODE_PORTS_PADDING
        + port_idx as f32 * PORT_ROW_HEIGHT
        + PORT_ROW_HEIGHT * 0.5;
    (node.x + 12.0, y)
}

pub(crate) fn output_port_world_center(
    node: &VisualNode,
    port_idx: usize,
    include_chart: bool,
    connections: &[NodeConnection],
) -> (f32, f32) {
    if node.collapsed {
        let connected: Vec<usize> = connections
            .iter()
            .filter(|connection| connection.from_node_id == node.id)
            .map(|connection| connection.from_port_idx)
            .collect();
        if let Some(slot) = connected.iter().position(|&idx| idx == port_idx) {
            return pulsar_marketlab_ui::theme::capsule_socket_world_center(
                node.x,
                node.y,
                pulsar_marketlab_ui::theme::CapsuleSocketSide::Output,
                slot,
                connected.len(),
            );
        }
        return pulsar_marketlab_ui::theme::capsule_socket_world_center(
            node.x,
            node.y,
            pulsar_marketlab_ui::theme::CapsuleSocketSide::Output,
            0,
            1,
        );
    }
    let y = node.y
        + node_body_height_world(node, include_chart)
        + NODE_PORTS_PADDING
        + port_idx as f32 * PORT_ROW_HEIGHT
        + PORT_ROW_HEIGHT * 0.5;
    (node.x + NODE_WIDTH - 12.0, y)
}

pub(crate) fn input_port_is_wired(
    node: &VisualNode,
    port_idx: usize,
    connections: &[NodeConnection],
) -> bool {
    connections
        .iter()
        .any(|connection| connection.to_node_id == node.id && connection.to_port_idx == port_idx)
}

pub(crate) fn output_port_is_wired(
    node: &VisualNode,
    port_idx: usize,
    connections: &[NodeConnection],
) -> bool {
    connections
        .iter()
        .any(|connection| {
            connection.from_node_id == node.id && connection.from_port_idx == port_idx
        })
}
