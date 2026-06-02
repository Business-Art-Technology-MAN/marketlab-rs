//! Stable-graph execution engine compiled from USD stage topology.

use std::collections::HashMap;
use std::fmt;

use petgraph::algo::toposort;
use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::Direction;
use thiserror::Error;

use super::portfolio::{
    closures_from_upstream_legs, integrate_portfolio, normalize_asset_quote_key, AssetQuote,
    ClosureLegKind,
    PortfolioIntegratorConfig, PortfolioIntegrationResult, PortfolioOtlTransformFn,
    PortfolioTrackingFrame, SymbolicOtlClosure,
};
use crate::OtlObjectKind;

/// Thread-safe compiled signal transform closure.
pub type SignalTransformFn = dyn Fn(&[f64]) -> Vec<f64> + Send + Sync;

/// Thread-safe multi-AOV transform closure.
pub type MultiSignalTransformFn = dyn Fn(&[f64]) -> Vec<Vec<f64>> + Send + Sync;

/// Thread-safe node execution payload (prim path keyed separately in the engine).
pub enum ExecutionNode {
    DataInput { symbol: String },
    SignalTransform {
        expression: String,
        compiled_fn: Option<Box<SignalTransformFn>>,
        compiled_multi: Option<(Box<MultiSignalTransformFn>, Vec<String>)>,
    },
    PortfolioSink {
        method: String,
        initial_capital: f64,
        otl_script: String,
        otl_hook: Option<Box<PortfolioOtlTransformFn>>,
    },
}

/// One prim-to-prim relationship edge in the compile spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCompileWire {
    pub source_prim_path: String,
    pub target_prim_path: String,
    pub relationship: String,
}

/// Passive USD prim used when compiling from a stage snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageGraphPrim {
    pub path: String,
    pub type_name: String,
    pub attributes: HashMap<String, String>,
}

/// Declarative USD stage snapshot used to build a [`MarketLabGraphEngine`].
#[derive(Clone, Debug, Default)]
pub struct StageGraphSnapshot {
    pub prims: Vec<StageGraphPrim>,
    pub wires: Vec<GraphCompileWire>,
}

/// Declarative compile spec with explicit node payloads and wires.
#[derive(Debug, Default)]
pub struct GraphCompileSpec {
    pub nodes: Vec<(String, ExecutionNode)>,
    pub wires: Vec<GraphCompileWire>,
}

/// Time-sampled attribute stream written back into workspace render state.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedAttributeStream {
    pub prim_path: String,
    pub attribute: String,
    pub samples: Vec<(f64, f64)>,
}

/// Timeline sweep output including optional portfolio tracking matrices keyed by prim path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineExecutionResult {
    pub streams: Vec<ComputedAttributeStream>,
    pub portfolio_results: HashMap<String, PortfolioIntegrationResult>,
}

#[derive(Clone, Debug)]
struct NodeRuntimeOutput {
    scalar: Vec<f64>,
    asset_symbol: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GraphEngineError {
    #[error("graph compile spec has no nodes")]
    EmptyGraph,
    #[error("dependency cycle detected in stage graph")]
    CycleDetected,
    #[error("unknown prim path `{0}`")]
    UnknownPrimPath(String),
    #[error("unsupported prim type `{type_name}` at `{path}`")]
    UnsupportedPrimType { type_name: String, path: String },
    #[error("OTL script compile failed for `{path}`: {message}")]
    ScriptCompileError { path: String, message: String },
    #[error("OTL object kind `{object_kind:?}` incompatible with prim `{prim_path}`")]
    ObjectKindMismatch {
        prim_path: String,
        object_kind: OtlObjectKind,
    },
}

/// Symbolic closure extraction context (bar-independent compile phase).
#[derive(Clone, Debug, Default)]
pub struct EvaluationContext {
    pub timeline_len: usize,
}

/// Compiled USD stage graph with deterministic topological execution order.
pub struct MarketLabGraphEngine {
    graph: StableGraph<ExecutionNode, ()>,
    prim_to_index: HashMap<String, NodeIndex>,
    execution_order: Vec<NodeIndex>,
}

impl MarketLabGraphEngine {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            prim_to_index: HashMap::new(),
            execution_order: Vec::new(),
        }
    }

    pub fn graph(&self) -> &StableGraph<ExecutionNode, ()> {
        &self.graph
    }

    pub fn prim_to_index(&self) -> &HashMap<String, NodeIndex> {
        &self.prim_to_index
    }

    pub fn execution_order(&self) -> &[NodeIndex] {
        &self.execution_order
    }

    pub fn compile(spec: GraphCompileSpec) -> Result<Self, GraphEngineError> {
        if spec.nodes.is_empty() {
            return Err(GraphEngineError::EmptyGraph);
        }

        let mut engine = Self::new();
        for (prim_path, node) in spec.nodes {
            let index = engine.graph.add_node(node);
            engine.prim_to_index.insert(prim_path, index);
        }

        for wire in spec.wires {
            engine.connect(&wire.source_prim_path, &wire.target_prim_path)?;
        }

        engine.execution_order =
            toposort(&engine.graph, None).map_err(|_| GraphEngineError::CycleDetected)?;
        Ok(engine)
    }
}

fn resolve_prim_otl_expression(prim: &StageGraphPrim) -> String {
    if let Some(path) = prim
        .attributes
        .get("inputs:script_compiled_path")
        .map(|path| path.as_str().trim())
        .filter(|path| !path.is_empty())
    {
        if let Ok(asset) = super::binary::load_compiled_asset_from_path(path) {
            if let Ok(script) = asset.bytecode_as_script_source() {
                return script.to_string();
            }
        }
    }
    prim.attributes
        .get("inputs:script_src")
        .cloned()
        .unwrap_or_default()
}

impl MarketLabGraphEngine {
    pub fn compile_from_stage(snapshot: &StageGraphSnapshot) -> Result<Self, GraphEngineError> {
        let mut nodes = Vec::with_capacity(snapshot.prims.len());
        for prim in &snapshot.prims {
            let node = match prim.type_name.as_str() {
                "FinancialAsset" => ExecutionNode::DataInput {
                    symbol: prim
                        .attributes
                        .get("inputs:symbol")
                        .cloned()
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| prim.path.clone()),
                },
                "OtlOperator" | "OtlTaUberSignal" => ExecutionNode::SignalTransform {
                    expression: resolve_prim_otl_expression(prim),
                    compiled_fn: None,
                    compiled_multi: None,
                },
                "PortfolioIntegrator" => ExecutionNode::PortfolioSink {
                    method: prim
                        .attributes
                        .get("inputs:id")
                        .cloned()
                        .unwrap_or_else(|| "Allocation::EqualWeight".to_string()),
                    initial_capital: prim
                        .attributes
                        .get("inputs:initial_capital")
                        .and_then(|value| value.parse::<f64>().ok())
                        .unwrap_or(10_000_000.0),
                    otl_script: resolve_prim_otl_expression(prim),
                    otl_hook: None,
                },
                other => {
                    return Err(GraphEngineError::UnsupportedPrimType {
                        type_name: other.to_string(),
                        path: prim.path.clone(),
                    });
                }
            };
            nodes.push((prim.path.clone(), node));
        }

        let wires: Vec<GraphCompileWire> = snapshot
            .wires
            .iter()
            .filter(|wire| is_execution_relationship(&wire.relationship))
            .cloned()
            .collect();

        Self::compile(GraphCompileSpec { nodes, wires })?.compile_otl_scripts()
    }

    /// Compile every `inputs:script_src` expression into a vectorized closure.
    pub fn compile_otl_scripts(mut self) -> Result<Self, GraphEngineError> {
        let prim_paths: Vec<String> = self.prim_to_index.keys().cloned().collect();
        for prim_path in prim_paths {
            let Some(index) = self.prim_to_index.get(&prim_path).copied() else {
                continue;
            };
            let Some(node) = self.graph.node_weight_mut(index) else {
                continue;
            };
            let ExecutionNode::SignalTransform {
                expression,
                compiled_fn,
                compiled_multi,
            } = node
            else {
                continue;
            };

            let source = expression.trim();
            if source.is_empty() {
                *compiled_fn = Some(Box::new(|input| input.to_vec()));
                *compiled_multi = None;
                continue;
            }

            match super::script_resolve::compile_unified_script(source).map_err(|err| {
                GraphEngineError::ScriptCompileError {
                    path: prim_path.clone(),
                    message: err.to_string(),
                }
            })? {
                super::compiler::CompiledSeries::Single(closure) => {
                    *compiled_fn = Some(closure);
                    *compiled_multi = None;
                }
                super::compiler::CompiledSeries::Multi(closure, attributes) => {
                    *compiled_fn = None;
                    *compiled_multi = Some((closure, attributes));
                }
            }
        }
        Ok(self)
    }

    pub fn connect(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
    ) -> Result<(), GraphEngineError> {
        let source = *self
            .prim_to_index
            .get(source_prim_path)
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(source_prim_path.to_string()))?;
        let target = *self
            .prim_to_index
            .get(target_prim_path)
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(target_prim_path.to_string()))?;
        self.graph.add_edge(source, target, ());
        Ok(())
    }

    pub fn set_signal_compiled_fn(
        &mut self,
        prim_path: &str,
        compiled: Box<SignalTransformFn>,
    ) -> Result<(), GraphEngineError> {
        let index = *self
            .prim_to_index
            .get(prim_path)
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(prim_path.to_string()))?;
        let node = self
            .graph
            .node_weight_mut(index)
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(prim_path.to_string()))?;
        match node {
            ExecutionNode::SignalTransform {
                compiled_fn,
                compiled_multi,
                ..
            } => {
                *compiled_fn = Some(compiled);
                *compiled_multi = None;
            }
            _ => {
                return Err(GraphEngineError::UnsupportedPrimType {
                    type_name: "non-OtlOperator".to_string(),
                    path: prim_path.to_string(),
                });
            }
        }
        Ok(())
    }

    pub fn execution_order_prim_paths(&self) -> Vec<String> {
        self.execution_order
            .iter()
            .filter_map(|index| {
                self.prim_to_index
                    .iter()
                    .find_map(|(path, idx)| if idx == index { Some(path.clone()) } else { None })
            })
            .collect()
    }

    /// Extract symbolic OTL closures for a prim using the declared three-tier object kind.
    pub fn evaluate_node_closures(
        &self,
        prim_path: &str,
        object_kind: OtlObjectKind,
        ctx: &EvaluationContext,
    ) -> Result<Vec<SymbolicOtlClosure>, GraphEngineError> {
        let index = self
            .prim_to_index
            .get(prim_path)
            .copied()
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(prim_path.to_string()))?;

        let node = self
            .graph
            .node_weight(index)
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(prim_path.to_string()))?;

        let _timeline_len = ctx.timeline_len;
        match (object_kind, node) {
            (OtlObjectKind::Portfolio, ExecutionNode::PortfolioSink { .. }) => Ok(vec![]),
            (OtlObjectKind::Signal | OtlObjectKind::LegacyShader, ExecutionNode::SignalTransform { .. }) => {
                Ok(vec![])
            }
            (OtlObjectKind::Allocator, ExecutionNode::PortfolioSink { .. }) => Ok(vec![]),
            (kind, _) => Err(GraphEngineError::ObjectKindMismatch {
                prim_path: prim_path.to_string(),
                object_kind: kind,
            }),
        }
    }

    /// Execute compiled closures across `timeline_len` bars using fresh asset vectors.
    pub fn execute_timeline(
        &self,
        asset_vectors: &HashMap<String, Vec<f64>>,
        timeline_len: usize,
    ) -> TimelineExecutionResult {
        if timeline_len == 0 || self.execution_order.is_empty() {
            return TimelineExecutionResult::default();
        }

        let index_to_path: HashMap<NodeIndex, String> = self
            .prim_to_index
            .iter()
            .map(|(path, index)| (*index, path.clone()))
            .collect();

        let mut node_outputs: HashMap<NodeIndex, NodeRuntimeOutput> = HashMap::new();
        let mut streams: Vec<ComputedAttributeStream> = Vec::new();
        let mut portfolio_results: HashMap<String, PortfolioIntegrationResult> = HashMap::new();
        let quotes = build_asset_quotes(asset_vectors, timeline_len);

        for index in &self.execution_order {
            let prim_path = index_to_path
                .get(index)
                .cloned()
                .unwrap_or_else(|| format!("node_{index:?}"));

            let runtime = match self.graph.node_weight(*index) {
                Some(ExecutionNode::DataInput { symbol }) => {
                    let scalar = asset_vectors
                        .get(symbol)
                        .cloned()
                        .unwrap_or_else(|| vec![0.0; timeline_len])
                        .into_iter()
                        .take(timeline_len)
                        .collect::<Vec<_>>();
                    let samples = scalar
                        .iter()
                        .enumerate()
                        .map(|(bar, value)| (bar as f64, *value))
                        .collect();
                    streams.push(ComputedAttributeStream {
                        prim_path: prim_path.clone(),
                        attribute: "outputs:price".to_string(),
                        samples,
                    });
                    NodeRuntimeOutput {
                        scalar: pad_or_trim(scalar, timeline_len),
                        asset_symbol: Some(symbol.clone()),
                    }
                }
                Some(ExecutionNode::SignalTransform {
                    expression,
                    compiled_fn,
                    compiled_multi,
                }) => {
                    let upstream: Vec<f64> = self
                        .graph
                        .neighbors_directed(*index, Direction::Incoming)
                        .flat_map(|upstream_index| {
                            node_outputs
                                .get(&upstream_index)
                                .map(|output| output.scalar.clone())
                                .unwrap_or_default()
                        })
                        .collect();

                    let scalar = if let Some((run, attributes)) = compiled_multi.as_ref() {
                        let channels = run(&upstream);
                        let primary = channels.first().cloned().unwrap_or_else(|| {
                            passthrough_signal(&upstream, timeline_len, expression)
                        });
                        for (channel_index, values) in channels.into_iter().enumerate() {
                            let attribute = attributes
                                .get(channel_index)
                                .cloned()
                                .unwrap_or_else(|| format!("outputs:channel_{channel_index}"));
                            let samples = values
                                .iter()
                                .enumerate()
                                .map(|(bar, value)| (bar as f64, *value))
                                .collect();
                            streams.push(ComputedAttributeStream {
                                prim_path: prim_path.clone(),
                                attribute,
                                samples,
                            });
                        }
                        primary
                    } else {
                        let values = if let Some(run) = compiled_fn.as_ref() {
                            run(&upstream)
                        } else {
                            passthrough_signal(&upstream, timeline_len, expression)
                        };
                        let attribute = if expression.contains("ta::") || expression.contains("input")
                        {
                            "outputs:result".to_string()
                        } else {
                            "outputs:signal".to_string()
                        };
                        let samples = values
                            .iter()
                            .enumerate()
                            .map(|(bar, value)| (bar as f64, *value))
                            .collect();
                        streams.push(ComputedAttributeStream {
                            prim_path: prim_path.clone(),
                            attribute,
                            samples,
                        });
                        values
                    };

                    let asset_symbol = infer_upstream_asset_symbol(
                        &self.graph,
                        *index,
                        &node_outputs,
                    );
                    NodeRuntimeOutput {
                        scalar: pad_or_trim(scalar, timeline_len),
                        asset_symbol,
                    }
                }
                Some(ExecutionNode::PortfolioSink {
                    method,
                    initial_capital,
                    otl_script,
                    otl_hook,
                }) => {
                    let legs = collect_upstream_legs(
                        &self.graph,
                        *index,
                        &node_outputs,
                        &index_to_path,
                    );
                    let closures =
                        closures_from_upstream_legs(&legs, method);
                    let config = PortfolioIntegratorConfig {
                        allocation_method: method.clone(),
                        initial_capital: *initial_capital,
                        otl_script: otl_script.clone(),
                    };
                    let integration = integrate_portfolio(
                        &closures,
                        &quotes,
                        timeline_len,
                        &config,
                        otl_hook.as_deref(),
                    );
                    append_portfolio_tracking_streams(
                        &prim_path,
                        &integration.tracking_matrix,
                        &mut streams,
                    );
                    let samples = integration
                        .wealth_series
                        .iter()
                        .enumerate()
                        .map(|(bar, value)| (bar as f64, *value))
                        .collect();
                    streams.push(ComputedAttributeStream {
                        prim_path: prim_path.clone(),
                        attribute: "outputs:portfolio_wealth".to_string(),
                        samples,
                    });
                    portfolio_results.insert(prim_path.clone(), integration.clone());
                    NodeRuntimeOutput {
                        scalar: pad_or_trim(integration.wealth_series, timeline_len),
                        asset_symbol: None,
                    }
                }
                None => NodeRuntimeOutput {
                    scalar: vec![0.0; timeline_len],
                    asset_symbol: None,
                },
            };

            node_outputs.insert(*index, runtime);
        }

        TimelineExecutionResult {
            streams,
            portfolio_results,
        }
    }
}

impl fmt::Debug for ExecutionNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataInput { symbol } => f.debug_struct("DataInput").field("symbol", symbol).finish(),
            Self::SignalTransform {
                expression,
                compiled_fn,
                compiled_multi,
            } => f
                .debug_struct("SignalTransform")
                .field("expression", expression)
                .field("compiled_fn", &compiled_fn.as_ref().map(|_| "<fn>"))
                .field(
                    "compiled_multi",
                    &compiled_multi.as_ref().map(|_| "<multi-fn>"),
                )
                .finish(),
            Self::PortfolioSink {
                method,
                initial_capital,
                otl_script,
                otl_hook,
            } => f
                .debug_struct("PortfolioSink")
                .field("method", method)
                .field("initial_capital", initial_capital)
                .field("otl_script", otl_script)
                .field("otl_hook", &otl_hook.as_ref().map(|_| "<fn>"))
                .finish(),
        }
    }
}

fn is_execution_relationship(relationship: &str) -> bool {
    matches!(
        relationship,
        "inputs:underlying" | "inputs:sources" | "inputs:constituents"
    )
}

fn pad_or_trim(values: Vec<f64>, timeline_len: usize) -> Vec<f64> {
    if values.len() == timeline_len {
        return values;
    }
    if values.len() > timeline_len {
        return values.into_iter().take(timeline_len).collect();
    }
    let mut padded = values;
    padded.resize(timeline_len, 0.0);
    padded
}

fn passthrough_signal(upstream: &[f64], timeline_len: usize, expression: &str) -> Vec<f64> {
    if let Ok(closure) = super::compiler::compile_script(expression.trim()) {
        return pad_or_trim(closure(upstream), timeline_len);
    }
    if upstream.len() >= timeline_len {
        return upstream.iter().copied().take(timeline_len).collect();
    }
    if !upstream.is_empty() {
        return pad_or_trim(upstream.to_vec(), timeline_len);
    }
    vec![0.0; timeline_len]
}

fn build_asset_quotes(
    asset_vectors: &HashMap<String, Vec<f64>>,
    timeline_len: usize,
) -> HashMap<String, AssetQuote> {
    asset_vectors
        .iter()
        .map(|(symbol, prices)| {
            (
                symbol.clone(),
                AssetQuote {
                    price_series: pad_or_trim(prices.clone(), timeline_len),
                    contract_multiplier: 1.0,
                },
            )
        })
        .collect()
}

fn collect_upstream_legs(
    graph: &StableGraph<ExecutionNode, ()>,
    index: NodeIndex,
    node_outputs: &HashMap<NodeIndex, NodeRuntimeOutput>,
    index_to_path: &HashMap<NodeIndex, String>,
) -> Vec<(String, Vec<f64>, ClosureLegKind)> {
    graph
        .neighbors_directed(index, Direction::Incoming)
        .map(|upstream_index| {
            let leg_kind = graph
                .node_weight(upstream_index)
                .map(|node| match node {
                    ExecutionNode::PortfolioSink { .. } => ClosureLegKind::SubPortfolio,
                    _ => ClosureLegKind::Asset,
                })
                .unwrap_or(ClosureLegKind::Asset);
            let output = node_outputs.get(&upstream_index);
            let asset_id = normalize_asset_quote_key(
                &output
                    .and_then(|o| o.asset_symbol.clone())
                    .or_else(|| {
                        graph.node_weight(upstream_index).and_then(|node| match node {
                            ExecutionNode::DataInput { symbol } => Some(symbol.clone()),
                            _ => None,
                        })
                    })
                    .unwrap_or_else(|| {
                        index_to_path
                            .get(&upstream_index)
                            .cloned()
                            .unwrap_or_else(|| format!("leg_{upstream_index:?}"))
                    }),
            );
            let series = output.map(|o| o.scalar.clone()).unwrap_or_default();
            (asset_id, series, leg_kind)
        })
        .collect()
}

fn infer_upstream_asset_symbol(
    graph: &StableGraph<ExecutionNode, ()>,
    index: NodeIndex,
    node_outputs: &HashMap<NodeIndex, NodeRuntimeOutput>,
) -> Option<String> {
    let mut stack: Vec<NodeIndex> = graph.neighbors_directed(index, Direction::Incoming).collect();
    while let Some(upstream_index) = stack.pop() {
        if let Some(symbol) = node_outputs
            .get(&upstream_index)
            .and_then(|output| output.asset_symbol.clone())
        {
            return Some(symbol);
        }
        if let Some(ExecutionNode::DataInput { symbol }) = graph.node_weight(upstream_index) {
            return Some(symbol.clone());
        }
        stack.extend(
            graph
                .neighbors_directed(upstream_index, Direction::Incoming),
        );
    }
    None
}

fn append_portfolio_tracking_streams(
    prim_path: &str,
    matrix: &[PortfolioTrackingFrame],
    streams: &mut Vec<ComputedAttributeStream>,
) {
    let mut altered_weights: HashMap<String, Vec<(f64, f64)>> = HashMap::new();
    let mut calculated_units: HashMap<String, Vec<(f64, f64)>> = HashMap::new();
    let mut investment_returns: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

    for frame in matrix {
        let ts = frame.timestamp as f64;
        altered_weights
            .entry(frame.asset_id.clone())
            .or_default()
            .push((ts, frame.altered_portfolio_weight));
        calculated_units
            .entry(frame.asset_id.clone())
            .or_default()
            .push((ts, frame.calculated_units));
        investment_returns
            .entry(frame.asset_id.clone())
            .or_default()
            .push((ts, frame.investment_return));
    }

    for (asset_id, samples) in altered_weights {
        streams.push(ComputedAttributeStream {
            prim_path: prim_path.to_string(),
            attribute: format!("outputs:tracking:altered_weight:{asset_id}"),
            samples,
        });
    }
    for (asset_id, samples) in calculated_units {
        streams.push(ComputedAttributeStream {
            prim_path: prim_path.to_string(),
            attribute: format!("outputs:tracking:calculated_units:{asset_id}"),
            samples,
        });
    }
    for (asset_id, samples) in investment_returns {
        streams.push(ComputedAttributeStream {
            prim_path: prim_path.to_string(),
            attribute: format!("outputs:tracking:investment_return:{asset_id}"),
            samples,
        });
    }
}

impl fmt::Debug for MarketLabGraphEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketLabGraphEngine")
            .field("node_count", &self.graph.node_count())
            .field("edge_count", &self.graph.edge_count())
            .field("execution_order", &self.execution_order_prim_paths())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage_snapshot() -> StageGraphSnapshot {
        StageGraphSnapshot {
            prims: vec![
                StageGraphPrim {
                    path: "/assets/SPY".to_string(),
                    type_name: "FinancialAsset".to_string(),
                    attributes: HashMap::from([(
                        "inputs:symbol".to_string(),
                        "SPY".to_string(),
                    )]),
                },
                StageGraphPrim {
                    path: "/analytics/rsi".to_string(),
                    type_name: "OtlOperator".to_string(),
                    attributes: HashMap::from([(
                        "inputs:script_src".to_string(),
                        "identity".to_string(),
                    )]),
                },
                StageGraphPrim {
                    path: "/portfolios/main".to_string(),
                    type_name: "PortfolioIntegrator".to_string(),
                    attributes: HashMap::from([
                        (
                            "inputs:id".to_string(),
                            "Allocation::EqualWeight".to_string(),
                        ),
                        ("inputs:initial_capital".to_string(), "1000000".to_string()),
                    ]),
                },
            ],
            wires: vec![
                GraphCompileWire {
                    source_prim_path: "/assets/SPY".to_string(),
                    target_prim_path: "/analytics/rsi".to_string(),
                    relationship: "inputs:underlying".to_string(),
                },
                GraphCompileWire {
                    source_prim_path: "/analytics/rsi".to_string(),
                    target_prim_path: "/portfolios/main".to_string(),
                    relationship: "inputs:sources".to_string(),
                },
            ],
        }
    }

    #[test]
    fn compile_from_stage_orders_asset_before_portfolio() {
        let engine =
            MarketLabGraphEngine::compile_from_stage(&stage_snapshot()).expect("valid dag");
        assert_eq!(
            engine.execution_order_prim_paths(),
            vec![
                "/assets/SPY".to_string(),
                "/analytics/rsi".to_string(),
                "/portfolios/main".to_string(),
            ]
        );
    }

    #[test]
    fn compile_and_execute_asset_to_portfolio_chain() {
        let spec = GraphCompileSpec {
            nodes: vec![
                (
                    "/assets/SPY".to_string(),
                    ExecutionNode::DataInput {
                        symbol: "SPY".to_string(),
                    },
                ),
                (
                    "/analytics/rsi".to_string(),
                    ExecutionNode::SignalTransform {
                        expression: String::new(),
                        compiled_fn: None,
                        compiled_multi: None,
                    },
                ),
                (
                    "/portfolios/main".to_string(),
                    ExecutionNode::PortfolioSink {
                        method: "Allocation::EqualWeight".to_string(),
                        initial_capital: 1_000_000.0,
                        otl_script: String::new(),
                        otl_hook: None,
                    },
                ),
            ],
            wires: stage_snapshot().wires,
        };

        let engine = MarketLabGraphEngine::compile(spec).expect("valid dag");
        let mut assets = HashMap::new();
        assets.insert("SPY".to_string(), vec![100.0, 101.0, 102.0]);
        let result = engine.execute_timeline(&assets, 3);
        let streams = result.streams;
        assert!(streams
            .iter()
            .any(|stream| stream.attribute == "outputs:signal"));
        assert!(streams
            .iter()
            .any(|stream| stream.attribute == "outputs:portfolio_wealth"));
        assert!(result
            .portfolio_results
            .get("/portfolios/main")
            .is_some_and(|integration| !integration.tracking_matrix.is_empty()));
    }

    #[test]
    fn compile_rejects_cycles() {
        let spec = GraphCompileSpec {
            nodes: vec![
                (
                    "/assets/A".to_string(),
                    ExecutionNode::DataInput {
                        symbol: "A".to_string(),
                    },
                ),
                (
                    "/assets/B".to_string(),
                    ExecutionNode::DataInput {
                        symbol: "B".to_string(),
                    },
                ),
            ],
            wires: vec![
                GraphCompileWire {
                    source_prim_path: "/assets/A".to_string(),
                    target_prim_path: "/assets/B".to_string(),
                    relationship: "inputs:underlying".to_string(),
                },
                GraphCompileWire {
                    source_prim_path: "/assets/B".to_string(),
                    target_prim_path: "/assets/A".to_string(),
                    relationship: "inputs:underlying".to_string(),
                },
            ],
        };
        assert!(matches!(
            MarketLabGraphEngine::compile(spec),
            Err(GraphEngineError::CycleDetected)
        ));
    }

    #[test]
    fn compile_from_stage_compiles_script_src() {
        let mut snapshot = stage_snapshot();
        snapshot.prims[1]
            .attributes
            .insert("inputs:script_src".to_string(), "sma(data, 3)".to_string());
        let engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("compile with otl");
        let assets = HashMap::from([("SPY".to_string(), vec![1.0, 2.0, 3.0, 4.0, 5.0])]);
        let result = engine.execute_timeline(&assets, 5);
        let signal = result
            .streams
            .iter()
            .find(|stream| stream.prim_path == "/analytics/rsi")
            .expect("signal stream");
        assert!(signal.samples[0].1.is_nan());
        assert_eq!(signal.samples[2].1, 2.0);
    }

    #[test]
    fn identity_closure_executes_on_timeline() {
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&stage_snapshot()).expect("compile");
        engine
            .set_signal_compiled_fn("/analytics/rsi", Box::new(|input| input.to_vec()))
            .expect("attach closure");

        let assets = HashMap::from([("SPY".to_string(), vec![10.0, 20.0, 30.0])]);
        let result = engine.execute_timeline(&assets, 3);
        let signal = result
            .streams
            .iter()
            .find(|stream| stream.prim_path == "/analytics/rsi")
            .expect("signal stream");
        assert_eq!(signal.samples.len(), 3);
        assert_eq!(signal.samples[1].1, 20.0);
    }
}
