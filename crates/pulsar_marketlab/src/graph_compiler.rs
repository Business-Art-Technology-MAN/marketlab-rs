//! Graph configuration, shared snapshots, and DAG compilation.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pulsar_marketlab::execution_engine::ExecutionGraph;
use pulsar_marketlab::signal_dsl::evaluate_formula;
use pulsar_marketlab::technical_analysis::{
    clamp_ta_lookback, compute_ta_latest_with_params, MarketSeriesWindow,
};

pub(crate) fn is_price_source_node(node: &VisualNode) -> bool {
    matches!(node.node_type, NodeType::Asset | NodeType::Portfolio)
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
        .filter(|node| node.node_type == NodeType::Asset)
        .filter(|node| matches!(node.asset_source, Some(AssetSourceType::Csv { .. })))
        .map(|node| node.id)
        .collect()
}

pub fn portfolio_signal_port_label(port_idx: usize) -> String {
    format!("Signal In {port_idx}")
}

/// TA node ids with an active wire into any Portfolio input socket.
pub fn portfolio_wired_ta_node_ids(graph: &PipelineGraphSnapshot) -> HashSet<usize> {
    graph
        .connections
        .iter()
        .filter_map(|connection| {
            let to_node = graph
                .nodes
                .iter()
                .find(|node| node.id == connection.to_node_id)?;
            if to_node.node_type != NodeType::Portfolio {
                return None;
            }
            let from_node = graph
                .nodes
                .iter()
                .find(|node| node.id == connection.from_node_id)?;
            if from_node.node_type == NodeType::TechnicalAnalysis {
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

pub fn upstream_asset_for_ta_node(ta_node_id: usize, graph: &PipelineGraphSnapshot) -> Option<usize> {
    graph.connections.iter().find_map(|connection| {
        if connection.to_node_id != ta_node_id {
            return None;
        }
        let asset = graph
            .nodes
            .iter()
            .find(|node| node.id == connection.from_node_id)?;
        if asset.node_type == NodeType::Asset {
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
        let snapshot = PipelineGraphSnapshot {
            nodes,
            connections,
            execution_order: Vec::new(),
            dag_valid: true,
        };
        let (execution_order, dag_valid) = compile_graph_to_dag(&snapshot);
        Self(Arc::new(Mutex::new(PipelineGraphState {
            snapshot: PipelineGraphSnapshot {
                execution_order,
                dag_valid,
                ..snapshot
            },
            revision: 0,
        })))
    }

    pub fn replace(&self, nodes: Vec<VisualNode>, connections: Vec<NodeConnection>) {
        if let Ok(mut guard) = self.0.lock() {
            guard.revision = guard.revision.saturating_add(1);
            let base = PipelineGraphSnapshot {
                nodes,
                connections,
                execution_order: Vec::new(),
                dag_valid: true,
            };
            let (execution_order, dag_valid) = compile_graph_to_dag(&base);
            guard.snapshot = PipelineGraphSnapshot {
                execution_order,
                dag_valid,
                ..base
            };
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
}
pub const NODE_CHART_HEIGHT: f32 = 52.0;

pub(crate) const NODE_WIDTH: f32 = 220.0;
pub(crate) const NODE_HEADER_HEIGHT: f32 = 36.0;
const NODE_GRADE_HEIGHT: f32 = 32.0;
pub(crate) const NODE_TA_LABEL_HEIGHT: f32 = 20.0;
pub(crate) const NODE_PORTFOLIO_METRICS_HEIGHT: f32 = 72.0;
pub(crate) const NODE_PORTS_PADDING: f32 = 8.0;
pub(crate) const PORT_ROW_HEIGHT: f32 = 24.0;
pub(crate) const WIRE_PORT_HIT_RADIUS: f32 = 22.0;
pub(crate) const CONNECTION_STROKE_WIDTH: f32 = 2.0;
pub(crate) const MIN_ZOOM: f32 = 0.15;
pub(crate) const MAX_ZOOM: f32 = 3.0;
pub(crate) const ZOOM_WHEEL_SENSITIVITY: f32 = 0.002;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Asset,
    Portfolio,
    TechnicalAnalysis,
}

impl NodeType {
    pub(crate) fn displays_price_chart(self) -> bool {
        matches!(self, NodeType::Asset)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum NodeGradeType {
    Scalar,
    Vector,
    Trivector,
}

/// Polymorphic asset ingestion source bound to a visual pipeline node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetSourceType {
    Csv { path: String },
}

#[derive(Debug, Clone)]
pub struct VisualNode {
    pub id: usize,
    pub name: String,
    pub node_type: NodeType,
    pub grade: NodeGradeType,
    /// VectorTA indicator id from the registry (e.g. `rsi`, `macd`, `alma`).
    pub ta_indicator_id: Option<String>,
    /// Lookback period passed to VectorTA `period` param (sidebar slider).
    pub ta_lookback_period: u32,
    /// OSL-inspired formula evaluated against the upstream market window when set.
    pub dsl_formula: Option<String>,
    /// When set, a background CSV playback loop streams Yahoo Finance rows for this node.
    pub asset_source: Option<AssetSourceType>,
    pub x: f32,
    pub y: f32,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}
#[derive(Debug, Clone)]
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
        .find(|node| node.id == portfolio_id && node.node_type == NodeType::Portfolio)
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

pub(crate) fn connection_is_valid(
    from_node: &VisualNode,
    from_port_idx: usize,
    to_node: &VisualNode,
    to_port_idx: usize,
) -> bool {
    if from_port_idx >= from_node.outputs.len() || to_port_idx >= to_node.inputs.len() {
        return false;
    }
    matches!(
        (from_node.node_type, to_node.node_type),
        (NodeType::Asset, NodeType::TechnicalAnalysis)
            | (NodeType::TechnicalAnalysis, NodeType::Portfolio)
            | (NodeType::Asset, NodeType::Portfolio)
    )
}
pub(crate) fn ta_lookback_for_node(node: &VisualNode) -> usize {
    clamp_ta_lookback(node.ta_lookback_period as usize)
}

pub fn ta_compute_for_node(node: &VisualNode, window: &MarketSeriesWindow) -> Option<f64> {
    let lookback = ta_lookback_for_node(node);
    if let Some(formula) = node.dsl_formula.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return evaluate_formula(formula, window, lookback)
            .ok()
            .map(f64::from);
    }
    let indicator_id = node.ta_indicator_id.as_deref()?;
    compute_ta_latest_with_params(indicator_id, window, lookback)
}

#[cfg(test)]
mod graph_compiler_dsl_tests {
    use super::*;

    #[test]
    fn ta_compute_prefers_dsl_formula_over_indicator_id() {
        let node = VisualNode {
            id: 2,
            name: "Custom".into(),
            node_type: NodeType::TechnicalAnalysis,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: Some("sma".into()),
            ta_lookback_period: 14,
            dsl_formula: Some("close - sma(3)".into()),
            asset_source: None,
            x: 0.0,
            y: 0.0,
            inputs: vec!["In".into()],
            outputs: vec!["Out".into()],
        };
        let mut window = MarketSeriesWindow::default();
        for close in [100.0, 102.0, 101.0, 105.0, 104.0] {
            window.push_close_only(close);
        }
        let value = ta_compute_for_node(&node, &window).expect("dsl value");
        assert!((value - 0.666_667).abs() < 0.01);
    }
}

pub const NODE_SPAWN_STAGGER_X: f32 = 260.0;
pub const NODE_SPAWN_STAGGER_Y: f32 = 48.0;
pub(crate) fn node_shows_price_chart(node: &VisualNode) -> bool {
    node.node_type.displays_price_chart()
}

pub(crate) fn node_body_height_world(node: &VisualNode, include_chart: bool) -> f32 {
    let mut height = NODE_HEADER_HEIGHT + NODE_GRADE_HEIGHT;
    if node.node_type == NodeType::TechnicalAnalysis {
        height += NODE_TA_LABEL_HEIGHT;
    }
    if node.node_type == NodeType::Portfolio {
        height += NODE_PORTFOLIO_METRICS_HEIGHT;
    }
    if include_chart && node_shows_price_chart(node) {
        height += NODE_CHART_HEIGHT;
    }
    height
}

pub(crate) fn input_port_world_center(node: &VisualNode, port_idx: usize, include_chart: bool) -> (f32, f32) {
    let y = node.y
        + node_body_height_world(node, include_chart)
        + NODE_PORTS_PADDING
        + port_idx as f32 * PORT_ROW_HEIGHT
        + PORT_ROW_HEIGHT * 0.5;
    (node.x + 12.0, y)
}

pub(crate) fn output_port_world_center(node: &VisualNode, port_idx: usize, include_chart: bool) -> (f32, f32) {
    let y = node.y
        + node_body_height_world(node, include_chart)
        + NODE_PORTS_PADDING
        + port_idx as f32 * PORT_ROW_HEIGHT
        + PORT_ROW_HEIGHT * 0.5;
    (node.x + NODE_WIDTH - 12.0, y)
}
