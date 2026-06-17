//! Stable-graph execution engine compiled from USD stage topology.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

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
use crate::compiler::{CompiledProgramTier, ObjectCodegenRegistry};
use crate::engine::{PrecomputedMatrixCache, DEFAULT_COVARIANCE_LOOKBACK};
use crate::compile_object_tier;
use crate::engine::{
    compute_execution_levels, is_parallel_tier_signal, merge_parallel_signal_outcomes,
    run_parallel_signal_batch, evaluate_compiled_tier, MarketTimelineWindow, ParallelSignalJob,
    ParallelSweepContext, SharedPriceColumn,
};
use crate::execution_matrix::{ExecutionContext, GraphSeriesMatrix, RuntimeEngineError};
use crate::frontend::{
    apply_alpha_conviction, compile_object_program as parse_otl_object_program,
    conviction_scale_from_signal_series, resolve_runtime_script_source,
};
use crate::{OtlObjectKind, OtlProgram};

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
        tier_kind: Option<OtlObjectKind>,
        tier_program: Option<OtlProgram>,
    },
    PortfolioSink {
        method: String,
        initial_capital: f64,
        otl_script: String,
        otl_hook: Option<Box<PortfolioOtlTransformFn>>,
        tier_kind: Option<OtlObjectKind>,
        tier_program: Option<OtlProgram>,
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

/// Composed financial-asset metadata flattened from the taxonomy library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposedAssetMeta {
    pub symbol: String,
    pub asset_class: String,
    pub category: String,
    pub sub_category: String,
    pub is_active: bool,
    pub sector: String,
    pub industry: String,
    pub market_cap_class: String,
    pub currency: String,
    pub country: String,
    pub user_label: String,
}

/// O(1) prim-path → dense asset-vector slot index for timeline sweeps.
#[derive(Clone, Debug, Default)]
pub struct PathBindingIndex {
    pub asset_slots: HashMap<String, usize>,
    pub ordered_prim_paths: Vec<String>,
}

impl PathBindingIndex {
    pub fn slot_for_prim(&self, prim_path: &str) -> Option<usize> {
        self.asset_slots.get(prim_path).copied()
    }
}

/// Declarative USD stage snapshot used to build a [`MarketLabGraphEngine`].
#[derive(Clone, Debug, Default)]
pub struct StageGraphSnapshot {
    pub prims: Vec<StageGraphPrim>,
    pub wires: Vec<GraphCompileWire>,
    pub path_bindings: PathBindingIndex,
    pub asset_registry: HashMap<String, ComposedAssetMeta>,
}

/// Declarative compile spec with explicit node payloads and wires.
#[derive(Debug, Default)]
pub struct GraphCompileSpec {
    pub nodes: Vec<(String, ExecutionNode)>,
    pub wires: Vec<GraphCompileWire>,
}

/// Dense per-bar attribute stream written back into workspace render state.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedAttributeStream {
    pub prim_path: String,
    pub attribute: String,
    pub values: Vec<f64>,
}

impl ComputedAttributeStream {
    pub fn value_at(&self, frame: usize) -> Option<f64> {
        self.values.get(frame).copied()
    }
}

/// Token/string attribute stream (e.g. `outputs:weights` path:weight encodings per bar).
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedTokenStream {
    pub prim_path: String,
    pub attribute: String,
    pub samples: Vec<(f64, String)>,
}

/// Timeline sweep output including optional portfolio tracking matrices keyed by prim path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineExecutionResult {
    pub streams: Vec<ComputedAttributeStream>,
    pub token_streams: Vec<ComputedTokenStream>,
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

/// Symbolic closure extraction context (requires asset vectors for live evaluation).
#[derive(Clone, Debug, Default)]
pub struct EvaluationContext {
    pub timeline_len: usize,
    pub asset_vectors: HashMap<String, Arc<[f64]>>,
}

/// Reusable buffers for [`MarketLabGraphEngine::sweep`] (no per-bar heap traffic).
#[derive(Default)]
struct TimelineSweepScratch {
    node_outputs: HashMap<NodeIndex, NodeRuntimeOutput>,
    streams: Vec<ComputedAttributeStream>,
    token_streams: Vec<ComputedTokenStream>,
    portfolio_results: HashMap<String, PortfolioIntegrationResult>,
    upstream_scratch: Vec<f64>,
}

/// Compiled USD stage graph with deterministic topological execution order.
pub struct MarketLabGraphEngine {
    graph: StableGraph<ExecutionNode, ()>,
    prim_to_index: HashMap<String, NodeIndex>,
    /// Dense `NodeIndex` → absolute prim path (built at compile time).
    node_prim_paths: Vec<Arc<str>>,
    execution_order: Vec<NodeIndex>,
    /// Nodes grouped by longest-path depth for parallel signal sweeps within a level.
    execution_levels: Vec<Vec<NodeIndex>>,
    /// OTL tier engines compiled once in [`Self::compile_otl_scripts`] and reused each sweep.
    tier_sweep_cache: HashMap<NodeIndex, CompiledProgramTier>,
    sweep_scratch: TimelineSweepScratch,
}

impl MarketLabGraphEngine {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            prim_to_index: HashMap::new(),
            node_prim_paths: Vec::new(),
            execution_order: Vec::new(),
            execution_levels: Vec::new(),
            tier_sweep_cache: HashMap::new(),
            sweep_scratch: TimelineSweepScratch::default(),
        }
    }

    /// Cached OTL tier engines keyed by graph node (populated by [`Self::compile_otl_scripts`]).
    pub fn tier_sweep_cache(&self) -> &HashMap<NodeIndex, CompiledProgramTier> {
        &self.tier_sweep_cache
    }

    pub fn execution_levels(&self) -> &[Vec<NodeIndex>] {
        &self.execution_levels
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
        engine.execution_levels =
            compute_execution_levels(&engine.graph, &engine.execution_order);
        engine.node_prim_paths =
            build_node_prim_paths(&engine.graph, &engine.prim_to_index);
        Ok(engine)
    }
}

fn build_node_prim_paths(
    graph: &StableGraph<ExecutionNode, ()>,
    prim_to_index: &HashMap<String, NodeIndex>,
) -> Vec<Arc<str>> {
    let mut paths = vec![Arc::from(""); graph.node_count()];
    for (path, &index) in prim_to_index {
        let slot = index.index();
        if slot < paths.len() {
            paths[slot] = Arc::from(path.as_str());
        }
    }
    paths
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
                    // Canonical execution key: absolute prim path (matches asset vector map).
                    symbol: prim.path.clone(),
                },
                "OtlOperator" | "OtlTaUberSignal" => ExecutionNode::SignalTransform {
                    expression: resolve_prim_otl_expression(prim),
                    compiled_fn: None,
                    compiled_multi: None,
                    tier_kind: None,
                    tier_program: None,
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
                    tier_kind: None,
                    tier_program: None,
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

    /// Compile from a canvas-built graph snapshot (same IR as [`Self::compile_from_stage`]).
    pub fn compile_from_canvas(snapshot: &StageGraphSnapshot) -> Result<Self, GraphEngineError> {
        Self::compile_from_stage(snapshot)
    }

    /// Compile every `inputs:script_src` expression into a vectorized closure or tier program.
    pub fn compile_otl_scripts(mut self) -> Result<Self, GraphEngineError> {
        let prim_paths: Vec<String> = self.prim_to_index.keys().cloned().collect();
        for prim_path in prim_paths {
            let Some(index) = self.prim_to_index.get(&prim_path).copied() else {
                continue;
            };
            let Some(node) = self.graph.node_weight_mut(index) else {
                continue;
            };

            match node {
                ExecutionNode::SignalTransform {
                    expression,
                    compiled_fn,
                    compiled_multi,
                    tier_kind,
                    tier_program,
                } => {
                    let source = expression.trim();
                    let resolved = if source.is_empty() {
                        resolve_runtime_script_source("raw")
                    } else {
                        resolve_runtime_script_source(source)
                    }
                    .map_err(|err| {
                        GraphEngineError::ScriptCompileError {
                            path: prim_path.clone(),
                            message: err.to_string(),
                        }
                    })?;

                    let object_source = if is_three_tier_kind(resolved.kind) {
                        source.to_string()
                    } else {
                        super::script_resolve::wrap_series_script_as_signal_source(
                            resolved.runtime_script.trim(),
                        )
                    };
                    let program = parse_otl_object_program(&object_source).map_err(|err| {
                        GraphEngineError::ScriptCompileError {
                            path: prim_path.clone(),
                            message: err.to_string(),
                        }
                    })?;
                    *tier_kind = Some(OtlObjectKind::Signal);
                    *tier_program = Some(program);
                    *compiled_fn = None;
                    *compiled_multi = None;
                }
                ExecutionNode::PortfolioSink {
                    otl_script,
                    tier_kind,
                    tier_program,
                    ..
                } => {
                    if otl_script.trim().is_empty() {
                        *tier_kind = None;
                        *tier_program = None;
                        continue;
                    }
                    let source = otl_script.trim().to_string();
                    let resolved = resolve_runtime_script_source(&source).map_err(|err| {
                        GraphEngineError::ScriptCompileError {
                            path: prim_path.clone(),
                            message: err.to_string(),
                        }
                    })?;
                    *otl_script = resolved.runtime_script.clone();

                    if matches!(
                        resolved.kind,
                        OtlObjectKind::Allocator | OtlObjectKind::Portfolio
                    ) {
                        let program = parse_otl_object_program(&source).map_err(|err| {
                            GraphEngineError::ScriptCompileError {
                                path: prim_path.clone(),
                                message: err.to_string(),
                            }
                        })?;
                        *tier_kind = Some(resolved.kind);
                        *tier_program = Some(program);
                    } else {
                        *tier_kind = None;
                        *tier_program = None;
                    }
                }
                _ => {}
            }
        }
        self.populate_tier_sweep_cache()?;
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
                tier_kind,
                tier_program,
                ..
            } => {
                *compiled_fn = Some(compiled);
                *compiled_multi = None;
                *tier_kind = None;
                *tier_program = None;
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
        if ctx.timeline_len == 0 {
            return Ok(Vec::new());
        }

        let index = self
            .prim_to_index
            .get(prim_path)
            .copied()
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(prim_path.to_string()))?;

        let node = self
            .graph
            .node_weight(index)
            .ok_or_else(|| GraphEngineError::UnknownPrimPath(prim_path.to_string()))?;

        let node_outputs = self.simulate_node_outputs(&ctx.asset_vectors, ctx.timeline_len, index)?;

        match (object_kind, node) {
            (
                OtlObjectKind::Signal | OtlObjectKind::LegacyShader,
                ExecutionNode::SignalTransform {
                    expression,
                    tier_kind: _,
                    tier_program: _,
                    ..
                },
            ) => {
                let output = node_outputs.get(&index).ok_or_else(|| {
                    GraphEngineError::UnknownPrimPath(prim_path.to_string())
                })?;
                let asset_id = infer_upstream_asset_symbol(&self.graph, index, &node_outputs)
                    .map(|symbol| normalize_asset_quote_key(&symbol))
                    .unwrap_or_else(|| normalize_asset_quote_key(prim_path));
                let scale = conviction_scale_from_signal_series(&output.scalar);
                let weight = apply_alpha_conviction(1.0, expression, scale);
                Ok(vec![SymbolicOtlClosure {
                    asset_id,
                    direction: super::portfolio::DirectionalDistribution::MarketLong,
                    closure_raw_weight: weight,
                    signal_series: output.scalar.clone(),
                    leg_kind: ClosureLegKind::Asset,
                }])
            }
            (
                OtlObjectKind::Portfolio | OtlObjectKind::Allocator,
                ExecutionNode::PortfolioSink {
                    method,
                    otl_script,
                    tier_kind: _,
                    tier_program: _,
                    ..
                },
            ) => {
                let quotes = build_asset_quotes_from_vectors(&ctx.asset_vectors);
                let legs = collect_upstream_legs(
                    &self.graph,
                    index,
                    &node_outputs,
                    &self.node_prim_paths,
                    &quotes,
                );
                let mut closures = closures_from_upstream_legs(&legs, method);
                for closure in &mut closures {
                    let scale = conviction_scale_from_signal_series(&closure.signal_series);
                    closure.closure_raw_weight =
                        apply_alpha_conviction(closure.closure_raw_weight, otl_script, scale);
                }
                Ok(closures)
            }
            (kind, _) => Err(GraphEngineError::ObjectKindMismatch {
                prim_path: prim_path.to_string(),
                object_kind: kind,
            }),
        }
    }

    fn simulate_node_outputs(
        &self,
        asset_vectors: &HashMap<String, Arc<[f64]>>,
        timeline_len: usize,
        through_index: NodeIndex,
    ) -> Result<HashMap<NodeIndex, NodeRuntimeOutput>, GraphEngineError> {
        let index_to_path: HashMap<NodeIndex, String> = self
            .prim_to_index
            .iter()
            .map(|(path, index)| (*index, path.clone()))
            .collect();
        let mut node_outputs: HashMap<NodeIndex, NodeRuntimeOutput> = HashMap::new();

        for index in &self.execution_order {
            let _prim_path = index_to_path
                .get(index)
                .cloned()
                .unwrap_or_else(|| format!("node_{index:?}"));

            let runtime = match self.graph.node_weight(*index) {
                Some(ExecutionNode::DataInput { symbol }) => {
                    let scalar: Vec<f64> = asset_vectors
                        .get(symbol)
                        .map(|series| series.iter().copied().take(timeline_len).collect())
                        .unwrap_or_else(|| vec![0.0; timeline_len]);
                    NodeRuntimeOutput {
                        scalar: pad_or_trim(scalar, timeline_len),
                        asset_symbol: Some(symbol.clone()),
                    }
                }
                Some(ExecutionNode::SignalTransform {
                    expression,
                    compiled_fn: _,
                    compiled_multi: _,
                    tier_kind: _,
                    tier_program: _,
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
                    let scalar = passthrough_signal(&upstream, timeline_len, expression);
                    let asset_symbol = infer_upstream_asset_symbol(&self.graph, *index, &node_outputs);
                    NodeRuntimeOutput {
                        scalar: pad_or_trim(scalar, timeline_len),
                        asset_symbol,
                    }
                }
                Some(ExecutionNode::PortfolioSink { .. }) => NodeRuntimeOutput {
                    scalar: vec![0.0; timeline_len],
                    asset_symbol: None,
                },
                None => NodeRuntimeOutput {
                    scalar: vec![0.0; timeline_len],
                    asset_symbol: None,
                },
            };

            node_outputs.insert(*index, runtime);
            if *index == through_index {
                break;
            }
        }

        Ok(node_outputs)
    }

    /// Run signal-tier nodes in parallel for one level (call after upstream nodes in the level ran).
    fn run_parallel_signal_level(
        &mut self,
        timeline_len: usize,
        indices: &[NodeIndex],
        tier_workspace: &mut TimelineTierWorkspace,
    ) -> HashSet<NodeIndex> {
        let mut completed = HashSet::new();
        let parallel_ctx = ParallelSweepContext {
            timeline_len,
            initial_capital: tier_workspace.exec_ctx.initial_capital,
            allocation_method: tier_workspace.exec_ctx.allocation_method.clone(),
            asset_quotes: tier_workspace.exec_ctx.asset_quotes.clone(),
        };

        let mut jobs = Vec::new();
        for &index in indices {
            let Some(node) = self.graph.node_weight(index) else {
                continue;
            };
            let ExecutionNode::SignalTransform {
                expression,
                tier_kind: Some(OtlObjectKind::Signal),
                ..
            } = node
            else {
                continue;
            };
            if !self.tier_sweep_cache.contains_key(&index) {
                continue;
            }
            let prim_path = self.prim_path_for_index(index).to_string();
            gather_upstream_into(
                &mut self.sweep_scratch.upstream_scratch,
                &self.graph,
                index,
                &self.sweep_scratch.node_outputs,
            );
            let column = tier_workspace.reserve_signal_column();
            jobs.push(ParallelSignalJob {
                node_index: index,
                prim_path,
                column,
                upstream: pad_or_trim(self.sweep_scratch.upstream_scratch.clone(), timeline_len),
                expression: expression.clone(),
            });
        }

        if jobs.len() < 2 {
            for job in jobs {
                tier_workspace.release_reserved_column(job.column);
            }
            return completed;
        }

        let mut tier_forks: Vec<CompiledProgramTier> = jobs
            .iter()
            .map(|job| {
                self.tier_sweep_cache
                    .get(&job.node_index)
                    .map(CompiledProgramTier::fork_for_sweep)
                    .unwrap_or_else(|| {
                        panic!(
                            "missing cached signal tier for node {:?}",
                            job.node_index
                        )
                    })
            })
            .collect();
        let outcomes = run_parallel_signal_batch(&jobs, &mut tier_forks, &parallel_ctx);
        merge_parallel_signal_outcomes(&mut tier_workspace.matrix, &outcomes);

        for (job, outcome) in jobs.iter().zip(outcomes.iter()) {
            if outcome.error.is_some() {
                tier_workspace.release_reserved_column(job.column);
                continue;
            }
            tier_workspace.commit_signal_column(job.node_index, job.column);
            let attribute = signal_stream_attribute(&job.expression);
            push_scalar_stream(
                &mut self.sweep_scratch.streams,
                &job.prim_path,
                &attribute,
                &outcome.convictions,
            );
            let asset_symbol = infer_upstream_asset_symbol(
                &self.graph,
                job.node_index,
                &self.sweep_scratch.node_outputs,
            );
            self.sweep_scratch.node_outputs.insert(
                job.node_index,
                NodeRuntimeOutput {
                    scalar: pad_or_trim(outcome.convictions.clone(), timeline_len),
                    asset_symbol,
                },
            );
            completed.insert(job.node_index);
        }

        completed
    }

    /// Activate timeline buffers (allocations permitted). Keys must be absolute prim paths.
    pub fn activate_timeline(
        asset_vectors: HashMap<String, SharedPriceColumn>,
        timeline_len: usize,
    ) -> MarketTimelineWindow {
        MarketTimelineWindow::activate(asset_vectors, timeline_len)
    }

    /// Zero-allocation sweep entry point after [`Self::activate_timeline`] and graph compile.
    ///
    /// Closure trees and OTL tiers must already be baked (see [`Self::compile_otl_scripts`]).
    pub fn sweep(&mut self, window: &MarketTimelineWindow) -> TimelineExecutionResult {
        let timeline_len = window.timeline_len();
        if timeline_len == 0 || self.execution_order.is_empty() {
            return TimelineExecutionResult::default();
        }

        self.sweep_scratch.node_outputs.clear();
        self.sweep_scratch.streams.clear();
        self.sweep_scratch.portfolio_results.clear();

        let quotes = build_asset_quotes_from_window(window);
        let asset_vectors = asset_vectors_from_window(window);
        let mut tier_workspace = TimelineTierWorkspace::new(
            &self.graph,
            timeline_len,
            quotes,
            asset_vectors,
        );

        let execution_levels: Vec<Vec<NodeIndex>> =
            self.execution_levels.iter().cloned().collect();
        for level in execution_levels {
            let mut parallel_eligible = Vec::new();
            for index in level.iter().copied() {
                if let Some(node) = self.graph.node_weight(index) {
                    if is_parallel_tier_signal(node) {
                        parallel_eligible.push(index);
                    }
                }
            }
            let run_parallel_batch = parallel_eligible.len() >= 2;
            let parallel_set: HashSet<NodeIndex> = parallel_eligible.iter().copied().collect();

            for index in level {
                if run_parallel_batch && parallel_set.contains(&index) {
                    continue;
                }
                let prim_path = self
                    .node_prim_paths
                    .get(index.index())
                    .map(|path| path.to_string())
                    .filter(|path| !path.is_empty())
                    .unwrap_or_else(|| "node".to_string());
                let runtime = self.execute_timeline_node(
                    index,
                    &prim_path,
                    timeline_len,
                    window,
                    &mut tier_workspace,
                );
                self.sweep_scratch.node_outputs.insert(index, runtime);
            }

            if run_parallel_batch {
                let _ = self.run_parallel_signal_level(
                    timeline_len,
                    &parallel_eligible,
                    &mut tier_workspace,
                );
            }
        }

        TimelineExecutionResult {
            streams: std::mem::take(&mut self.sweep_scratch.streams),
            token_streams: std::mem::take(&mut self.sweep_scratch.token_streams),
            portfolio_results: std::mem::take(&mut self.sweep_scratch.portfolio_results),
        }
    }

    /// Convenience: activate window then sweep (allocates once at the boundary).
    pub fn execute_timeline(
        &mut self,
        asset_vectors: HashMap<String, SharedPriceColumn>,
        timeline_len: usize,
    ) -> TimelineExecutionResult {
        let window = Self::activate_timeline(asset_vectors, timeline_len);
        self.sweep(&window)
    }

    fn prim_path_for_index(&self, index: NodeIndex) -> &str {
        self.node_prim_paths
            .get(index.index())
            .map(|path| path.as_ref())
            .filter(|path| !path.is_empty())
            .unwrap_or("node")
    }

    fn execute_timeline_node(
        &mut self,
        index: NodeIndex,
        prim_path: &str,
        timeline_len: usize,
        window: &MarketTimelineWindow,
        tier_workspace: &mut TimelineTierWorkspace,
    ) -> NodeRuntimeOutput {
        match self.graph.node_weight(index) {
                Some(ExecutionNode::DataInput { symbol }) => {
                    let scalar = window
                        .series_at_path(symbol)
                        .map(|series| series.to_vec())
                        .unwrap_or_else(|| vec![0.0; timeline_len]);
                    push_scalar_stream(
                        &mut self.sweep_scratch.streams,
                        prim_path,
                        "outputs:price",
                        &scalar,
                    );
                    NodeRuntimeOutput {
                        scalar,
                        asset_symbol: Some(symbol.clone()),
                    }
                }
                Some(ExecutionNode::SignalTransform {
                    expression,
                    compiled_fn: _,
                    compiled_multi: _,
                    tier_kind,
                    tier_program: _,
                }) => {
                    gather_upstream_into(
                        &mut self.sweep_scratch.upstream_scratch,
                        &self.graph,
                        index,
                        &self.sweep_scratch.node_outputs,
                    );
                    let upstream = self.sweep_scratch.upstream_scratch.as_slice();

                    let scalar = if matches!(tier_kind, Some(OtlObjectKind::Signal)) {
                        self.tier_sweep_cache
                            .get_mut(&index)
                            .ok_or_else(|| GraphEngineError::ScriptCompileError {
                                path: prim_path.to_string(),
                                message:
                                    "missing cached signal tier; call compile_otl_scripts()"
                                        .to_string(),
                            })
                            .and_then(|tier| {
                                tier_workspace.run_signal_node(
                                    prim_path,
                                    index,
                                    tier,
                                    pad_or_trim_slice(upstream, timeline_len),
                                )
                            })
                            .unwrap_or_else(|_| {
                                tier_workspace.rollback_signal_attempt();
                                pad_or_trim_slice(upstream, timeline_len).to_vec()
                            })
                    } else {
                        vec![0.0; timeline_len]
                    };
                    let attribute = signal_stream_attribute(expression);
                    push_scalar_stream(
                        &mut self.sweep_scratch.streams,
                        prim_path,
                        &attribute,
                        &scalar,
                    );
                    NodeRuntimeOutput {
                        scalar: pad_or_trim(scalar, timeline_len),
                        asset_symbol: infer_upstream_asset_symbol(
                            &self.graph,
                            index,
                            &self.sweep_scratch.node_outputs,
                        ),
                    }
                }
                Some(ExecutionNode::PortfolioSink {
                    method,
                    initial_capital,
                    otl_script,
                    otl_hook,
                    tier_kind,
                    tier_program,
                }) => {
                    if let (Some(kind), Some(_program)) = (tier_kind, tier_program.as_ref()) {
                        if *kind == OtlObjectKind::Allocator {
                            let upstream_indices: Vec<NodeIndex> = self
                                .graph
                                .neighbors_directed(index, Direction::Incoming)
                                .collect();
                            let allocator_result = self
                                .tier_sweep_cache
                                .get_mut(&index)
                                .ok_or_else(|| GraphEngineError::ScriptCompileError {
                                    path: prim_path.to_string(),
                                    message:
                                        "missing cached allocator tier; call compile_otl_scripts()"
                                            .to_string(),
                                })
                                .and_then(|tier| {
                                    tier_workspace.run_allocator_node(
                                        &prim_path,
                                        tier,
                                        method,
                                        *initial_capital,
                                        &upstream_indices,
                                    )
                                });
                            if let Ok(weight_series) = allocator_result {
                                push_scalar_stream(
                                    &mut self.sweep_scratch.streams,
                                    prim_path,
                                    "outputs:allocator_weights",
                                    &weight_series,
                                );
                                NodeRuntimeOutput {
                                    scalar: pad_or_trim(weight_series, timeline_len),
                                    asset_symbol: None,
                                }
                            } else {
                                tier_workspace.rollback_allocator_attempt();
                                legacy_portfolio_runtime(
                                    &self.graph,
                                    index,
                                    &self.sweep_scratch.node_outputs,
                                    &self.node_prim_paths,
                                    method,
                                    *initial_capital,
                                    otl_script,
                                    otl_hook.as_deref(),
                                    timeline_len,
                                    &tier_workspace.exec_ctx.asset_quotes,
                                    prim_path,
                                    &mut self.sweep_scratch.streams,
                                    &mut self.sweep_scratch.token_streams,
                                    &mut self.sweep_scratch.portfolio_results,
                                )
                            }
                        } else if *kind == OtlObjectKind::Portfolio {
                            let upstream_indices: Vec<NodeIndex> = self
                                .graph
                                .neighbors_directed(index, Direction::Incoming)
                                .collect();
                            let portfolio_result = self
                                .tier_sweep_cache
                                .get_mut(&index)
                                .ok_or_else(|| GraphEngineError::ScriptCompileError {
                                    path: prim_path.to_string(),
                                    message:
                                        "missing cached portfolio tier; call compile_otl_scripts()"
                                            .to_string(),
                                })
                                .and_then(|tier| {
                                    tier_workspace.run_portfolio_node(
                                        &prim_path,
                                        tier,
                                        method,
                                        *initial_capital,
                                        &upstream_indices,
                                    )
                                });
                            if let Ok((wealth, cash, weight_encodings)) = portfolio_result {
                                push_scalar_stream(
                                    &mut self.sweep_scratch.streams,
                                    prim_path,
                                    "outputs:portfolio_wealth",
                                    &wealth,
                                );
                                push_scalar_stream(
                                    &mut self.sweep_scratch.streams,
                                    prim_path,
                                    "outputs:portfolio_cash",
                                    &cash,
                                );
                                push_token_stream(
                                    &mut self.sweep_scratch.token_streams,
                                    prim_path,
                                    "outputs:weights",
                                    &weight_encodings,
                                );
                                let integration = PortfolioIntegrationResult {
                                    wealth_series: wealth.clone(),
                                    tracking_matrix: Vec::new(),
                                };
                                self.sweep_scratch
                                    .portfolio_results
                                    .insert(prim_path.to_string(), integration);
                                NodeRuntimeOutput {
                                    scalar: pad_or_trim(wealth, timeline_len),
                                    asset_symbol: None,
                                }
                            } else {
                                tier_workspace.rollback_portfolio_attempt();
                                legacy_portfolio_runtime(
                                    &self.graph,
                                    index,
                                    &self.sweep_scratch.node_outputs,
                                    &self.node_prim_paths,
                                    method,
                                    *initial_capital,
                                    otl_script,
                                    otl_hook.as_deref(),
                                    timeline_len,
                                    &tier_workspace.exec_ctx.asset_quotes,
                                    prim_path,
                                    &mut self.sweep_scratch.streams,
                                    &mut self.sweep_scratch.token_streams,
                                    &mut self.sweep_scratch.portfolio_results,
                                )
                            }
                        } else {
                            legacy_portfolio_runtime(
                                &self.graph,
                                index,
                                &self.sweep_scratch.node_outputs,
                                &self.node_prim_paths,
                                method,
                                *initial_capital,
                                otl_script,
                                otl_hook.as_deref(),
                                timeline_len,
                                &tier_workspace.exec_ctx.asset_quotes,
                                prim_path,
                                &mut self.sweep_scratch.streams,
                                &mut self.sweep_scratch.token_streams,
                                &mut self.sweep_scratch.portfolio_results,
                            )
                        }
                    } else {
                        legacy_portfolio_runtime(
                            &self.graph,
                            index,
                            &self.sweep_scratch.node_outputs,
                            &self.node_prim_paths,
                            method,
                            *initial_capital,
                            otl_script,
                            otl_hook.as_deref(),
                            timeline_len,
                            &tier_workspace.exec_ctx.asset_quotes,
                            prim_path,
                            &mut self.sweep_scratch.streams,
                            &mut self.sweep_scratch.token_streams,
                            &mut self.sweep_scratch.portfolio_results,
                        )
                    }
                }
            None => NodeRuntimeOutput {
                scalar: vec![0.0; timeline_len],
                asset_symbol: None,
            },
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
                tier_kind,
                tier_program,
            } => f
                .debug_struct("SignalTransform")
                .field("expression", expression)
                .field("compiled_fn", &compiled_fn.as_ref().map(|_| "<fn>"))
                .field(
                    "compiled_multi",
                    &compiled_multi.as_ref().map(|_| "<multi-fn>"),
                )
                .field("tier_kind", tier_kind)
                .field("tier_program", &tier_program.as_ref().map(|p| p.objects.len()))
                .finish(),
            Self::PortfolioSink {
                method,
                initial_capital,
                otl_script,
                otl_hook,
                tier_kind,
                tier_program,
            } => f
                .debug_struct("PortfolioSink")
                .field("method", method)
                .field("initial_capital", initial_capital)
                .field("otl_script", otl_script)
                .field("otl_hook", &otl_hook.as_ref().map(|_| "<fn>"))
                .field("tier_kind", tier_kind)
                .field("tier_program", &tier_program.as_ref().map(|p| p.objects.len()))
                .finish(),
        }
    }
}

fn is_three_tier_kind(kind: OtlObjectKind) -> bool {
    matches!(
        kind,
        OtlObjectKind::Signal | OtlObjectKind::Allocator | OtlObjectKind::Portfolio
    )
}

fn runtime_to_graph_error(prim_path: &str, err: RuntimeEngineError) -> GraphEngineError {
    GraphEngineError::ScriptCompileError {
        path: prim_path.to_string(),
        message: err.to_string(),
    }
}

fn signal_stream_attribute(expression: &str) -> String {
    if expression.contains("ta::") || expression.contains("input") {
        "outputs:result".to_string()
    } else {
        "outputs:signal".to_string()
    }
}

/// Remove any streams already emitted for a prim (tier partial/failed attempts).
fn clear_prim_streams(streams: &mut Vec<ComputedAttributeStream>, prim_path: &str) {
    streams.retain(|stream| stream.prim_path != prim_path);
}

fn clear_prim_token_streams(streams: &mut Vec<ComputedTokenStream>, prim_path: &str) {
    streams.retain(|stream| stream.prim_path != prim_path);
}

fn push_token_stream(
    streams: &mut Vec<ComputedTokenStream>,
    prim_path: &str,
    attribute: &str,
    encodings: &[String],
) {
    if encodings.is_empty() {
        return;
    }
    let samples = encodings
        .iter()
        .enumerate()
        .map(|(bar, value)| (bar as f64, value.clone()))
        .collect();
    streams.push(ComputedTokenStream {
        prim_path: prim_path.to_string(),
        attribute: attribute.to_string(),
        samples,
    });
}

fn push_scalar_stream(
    streams: &mut Vec<ComputedAttributeStream>,
    prim_path: &str,
    attribute: &str,
    values: &[f64],
) {
    streams.push(ComputedAttributeStream {
        prim_path: prim_path.to_string(),
        attribute: attribute.to_string(),
        values: values.to_vec(),
    });
}

fn legacy_portfolio_runtime(
    graph: &StableGraph<ExecutionNode, ()>,
    index: NodeIndex,
    node_outputs: &HashMap<NodeIndex, NodeRuntimeOutput>,
    node_prim_paths: &[Arc<str>],
    method: &str,
    initial_capital: f64,
    otl_script: &str,
    otl_hook: Option<&PortfolioOtlTransformFn>,
    timeline_len: usize,
    quotes: &HashMap<String, AssetQuote>,
    prim_path: &str,
    streams: &mut Vec<ComputedAttributeStream>,
    token_streams: &mut Vec<ComputedTokenStream>,
    portfolio_results: &mut HashMap<String, PortfolioIntegrationResult>,
) -> NodeRuntimeOutput {
    clear_prim_streams(streams, prim_path);
    clear_prim_token_streams(token_streams, prim_path);
    let legs = collect_upstream_legs(
        graph,
        index,
        node_outputs,
        node_prim_paths,
        quotes,
    );
    let mut closures = closures_from_upstream_legs(&legs, method);
    for closure in &mut closures {
        let scale = conviction_scale_from_signal_series(&closure.signal_series);
        closure.closure_raw_weight =
            apply_alpha_conviction(closure.closure_raw_weight, otl_script, scale);
    }
    let config = PortfolioIntegratorConfig {
        allocation_method: method.to_string(),
        initial_capital,
        otl_script: otl_script.to_string(),
    };
    let integration = integrate_portfolio(
        &closures,
        quotes,
        timeline_len,
        &config,
        otl_hook,
    );
    append_portfolio_tracking_streams(prim_path, &integration.tracking_matrix, streams);
    push_scalar_stream(
        streams,
        prim_path,
        "outputs:portfolio_wealth",
        &integration.wealth_series,
    );
    let weight_encodings =
        crate::per_bar_weight_encodings(&integration.tracking_matrix);
    push_token_stream(
        token_streams,
        prim_path,
        "outputs:weights",
        &weight_encodings,
    );
    portfolio_results.insert(prim_path.to_string(), integration.clone());
    NodeRuntimeOutput {
        scalar: pad_or_trim(integration.wealth_series, timeline_len),
        asset_symbol: None,
    }
}

struct TimelineTierWorkspace {
    matrix: GraphSeriesMatrix,
    exec_ctx: ExecutionContext,
    signal_column_by_node: HashMap<NodeIndex, usize>,
    next_signal_column: usize,
    /// Column reserved for the in-flight signal sweep (rolled back on failure).
    pending_signal_column: Option<usize>,
}

impl TimelineTierWorkspace {
    fn new(
        graph: &StableGraph<ExecutionNode, ()>,
        timeline_len: usize,
        quotes: HashMap<String, AssetQuote>,
        asset_vectors: HashMap<String, Arc<[f64]>>,
    ) -> Self {
        let (signal_cols, allocator_legs) = estimate_tier_matrix_capacity(graph);
        let mut exec_ctx = ExecutionContext::new(
            timeline_len,
            10_000_000.0,
            "Allocation::EqualWeight",
            quotes,
            asset_vectors.clone(),
        );
        exec_ctx.attach_covariance_cache(build_covariance_cache(
            &asset_vectors,
            timeline_len,
        ));
        Self {
            matrix: GraphSeriesMatrix::with_capacity(timeline_len, signal_cols, allocator_legs),
            exec_ctx,
            signal_column_by_node: HashMap::new(),
            next_signal_column: 0,
            pending_signal_column: None,
        }
    }

    fn reserve_signal_column(&mut self) -> usize {
        let column = self.next_signal_column;
        self.next_signal_column += 1;
        self.pending_signal_column = Some(column);
        self.matrix.clear_signal_column(column);
        column
    }

    fn release_reserved_column(&mut self, column: usize) {
        self.matrix.clear_signal_column(column);
        if self.pending_signal_column == Some(column) {
            self.pending_signal_column = None;
        }
        if self.next_signal_column > 0 && column == self.next_signal_column - 1 {
            self.next_signal_column -= 1;
        }
    }

    fn commit_signal_column(&mut self, node_index: NodeIndex, column: usize) {
        self.pending_signal_column = None;
        self.signal_column_by_node.insert(node_index, column);
    }

    fn rollback_signal_attempt(&mut self) {
        if let Some(column) = self.pending_signal_column.take() {
            self.matrix.clear_signal_column(column);
        }
    }

    fn rollback_allocator_attempt(&mut self) {
        for leg in 0..self.matrix.allocator_leg_count() {
            self.matrix.clear_allocator_leg(leg);
        }
    }

    fn rollback_portfolio_attempt(&mut self) {
        self.matrix.clear_portfolio_metrics();
    }

    /// Runs a signal-tier sweep using this node's upstream series (set fresh each topo step).
    fn run_signal_node(
        &mut self,
        prim_path: &str,
        node_index: NodeIndex,
        tier: &mut CompiledProgramTier,
        upstream: Vec<f64>,
    ) -> Result<Vec<f64>, GraphEngineError> {
        let column = self.reserve_signal_column();
        if column >= self.matrix.signal_column_count() {
            return Err(runtime_to_graph_error(
                prim_path,
                RuntimeEngineError::MissingSignalColumn { column_index: column },
            ));
        }
        self.exec_ctx.signal_upstream = upstream;
        self.exec_ctx.signal_output_column = column;

        if let Err(err) = evaluate_compiled_tier(&mut self.exec_ctx, tier, &mut self.matrix) {
            self.rollback_signal_attempt();
            return Err(runtime_to_graph_error(prim_path, err));
        }
        self.commit_signal_column(node_index, column);
        Ok(self.matrix.signal_column_slice(column).to_vec())
    }

    fn run_allocator_node(
        &mut self,
        prim_path: &str,
        tier: &mut CompiledProgramTier,
        method: &str,
        initial_capital: f64,
        upstream_indices: &[NodeIndex],
    ) -> Result<Vec<f64>, GraphEngineError> {
        if upstream_indices.is_empty() {
            return Err(runtime_to_graph_error(
                prim_path,
                RuntimeEngineError::MissingSignalColumn { column_index: 0 },
            ));
        }
        self.exec_ctx.allocation_method = method.to_string();
        self.exec_ctx.initial_capital = initial_capital;
        for leg in 0..self.matrix.allocator_leg_count() {
            self.matrix.clear_allocator_leg(leg);
        }
        if let Err(err) = evaluate_compiled_tier(&mut self.exec_ctx, tier, &mut self.matrix) {
            self.rollback_allocator_attempt();
            return Err(runtime_to_graph_error(prim_path, err));
        }
        Ok(self.matrix.allocator_leg_slice(0).to_vec())
    }

    fn seed_allocator_weights_from_signals(&mut self, upstream_indices: &[NodeIndex]) {
        for (leg, upstream_index) in upstream_indices.iter().enumerate() {
            let Some(column) = self.signal_column_by_node.get(upstream_index) else {
                continue;
            };
            for bar in 0..self.matrix.bar_count() {
                let weight = self
                    .matrix
                    .read_signal(*column, bar)
                    .unwrap_or(0.0)
                    .abs();
                self.matrix.write_allocator_weight(leg, bar, weight);
            }
        }
    }

    fn run_portfolio_node(
        &mut self,
        prim_path: &str,
        tier: &mut CompiledProgramTier,
        method: &str,
        initial_capital: f64,
        upstream_indices: &[NodeIndex],
    ) -> Result<(Vec<f64>, Vec<f64>, Vec<String>), GraphEngineError> {
        self.exec_ctx.allocation_method = method.to_string();
        self.exec_ctx.initial_capital = initial_capital;
        self.seed_allocator_weights_from_signals(upstream_indices);
        self.matrix.clear_portfolio_metrics();
        if let Err(err) = evaluate_compiled_tier(&mut self.exec_ctx, tier, &mut self.matrix) {
            self.rollback_portfolio_attempt();
            return Err(runtime_to_graph_error(prim_path, err));
        }
        let weight_encodings = match tier {
            CompiledProgramTier::Portfolio(engine) => engine.weight_encodings().to_vec(),
            _ => Vec::new(),
        };
        Ok((
            self.matrix.nav_series().to_vec(),
            self.matrix.cash_series().to_vec(),
            weight_encodings,
        ))
    }
}

fn predict_signal_column_map(
    graph: &StableGraph<ExecutionNode, ()>,
    execution_order: &[NodeIndex],
) -> HashMap<NodeIndex, usize> {
    let mut map = HashMap::new();
    let mut next = 0usize;
    for &index in execution_order {
        if matches!(
            graph.node_weight(index),
            Some(ExecutionNode::SignalTransform {
                tier_kind: Some(OtlObjectKind::Signal),
                ..
            })
        ) {
            map.insert(index, next);
            next += 1;
        }
    }
    map
}

fn build_covariance_cache(
    asset_vectors: &HashMap<String, Arc<[f64]>>,
    timeline_len: usize,
) -> Arc<PrecomputedMatrixCache> {
    let mut asset_paths: Vec<String> = asset_vectors.keys().cloned().collect();
    asset_paths.sort();
    let price_refs: HashMap<String, &[f64]> = asset_vectors
        .iter()
        .map(|(path, series)| (path.clone(), series.as_ref()))
        .collect();
    Arc::new(PrecomputedMatrixCache::build_from_vectors(
        &asset_paths,
        &price_refs,
        timeline_len,
        DEFAULT_COVARIANCE_LOOKBACK,
    ))
}

fn build_tier_compile_registry(
    graph: &StableGraph<ExecutionNode, ()>,
    node_index: NodeIndex,
    signal_columns: &HashMap<NodeIndex, usize>,
    initial_capital: f64,
    allocation_method: &str,
    asset_quotes: &HashMap<String, AssetQuote>,
    portfolio_prim_path: &str,
    index_to_path: &HashMap<NodeIndex, String>,
) -> ObjectCodegenRegistry {
    let mut registry = ObjectCodegenRegistry {
        allocation_method: allocation_method.to_string(),
        initial_capital,
        asset_quotes: asset_quotes.clone(),
        ..ObjectCodegenRegistry::default()
    };
    registry.portfolio_prim_path = Some(Arc::from(portfolio_prim_path));
    let upstream_indices: Vec<NodeIndex> = graph
        .neighbors_directed(node_index, Direction::Incoming)
        .collect();
    for (leg, upstream_index) in upstream_indices.iter().enumerate() {
        if let Some(path) = index_to_path.get(upstream_index) {
            registry
                .source_prim_paths
                .push(Arc::from(path.as_str()));
        }
        if let Some(&column) = signal_columns.get(upstream_index) {
            registry.register_signal_column(format!("leg_{leg}"), column);
            registry.register_signal_column("legs", column);
        }
    }
    registry
}

impl MarketLabGraphEngine {
    fn populate_tier_sweep_cache(&mut self) -> Result<(), GraphEngineError> {
        self.tier_sweep_cache.clear();
        let signal_columns = predict_signal_column_map(&self.graph, &self.execution_order);
        let index_to_path: HashMap<NodeIndex, String> = self
            .prim_to_index
            .iter()
            .map(|(path, node_index)| (*node_index, path.clone()))
            .collect();

        for &index in &self.execution_order {
            let Some(node) = self.graph.node_weight(index) else {
                continue;
            };
            let (tier_kind, tier_program) = match node {
                ExecutionNode::SignalTransform {
                    tier_kind,
                    tier_program,
                    ..
                }
                | ExecutionNode::PortfolioSink {
                    tier_kind,
                    tier_program,
                    ..
                } => (tier_kind, tier_program),
                _ => continue,
            };
            let Some(kind) = tier_kind else {
                continue;
            };
            let Some(program) = tier_program.clone() else {
                continue;
            };
            if !is_three_tier_kind(*kind) {
                continue;
            }
            let prim_path = index_to_path
                .get(&index)
                .cloned()
                .unwrap_or_else(|| format!("node_{index:?}"));

            let registry = build_tier_compile_registry(
                &self.graph,
                index,
                &signal_columns,
                10_000_000.0,
                "Allocation::EqualWeight",
                &HashMap::new(),
                &prim_path,
                &index_to_path,
            );
            let tier = compile_object_tier(&program, &registry).map_err(|err| {
                GraphEngineError::ScriptCompileError {
                    path: prim_path,
                    message: err.to_string(),
                }
            })?;
            self.tier_sweep_cache.insert(index, tier);
        }
        Ok(())
    }
}

fn estimate_tier_matrix_capacity(graph: &StableGraph<ExecutionNode, ()>) -> (usize, usize) {
    let mut signal_cols = 0usize;
    let mut allocator_legs = 1usize;
    for node in graph.node_weights() {
        match node {
            ExecutionNode::SignalTransform {
                tier_kind: Some(OtlObjectKind::Signal),
                ..
            } => signal_cols += 1,
            ExecutionNode::PortfolioSink {
                tier_kind: Some(OtlObjectKind::Allocator),
                tier_program: Some(program),
                ..
            } => {
                if let Some(object) = program.primary_object() {
                    let inputs = object
                        .inputs
                        .iter()
                        .filter(|port| port.ty.is_closure())
                        .count();
                    allocator_legs = allocator_legs.max(inputs.max(1));
                }
            }
            _ => {}
        }
    }
    (signal_cols.max(1), allocator_legs.max(1))
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

fn pad_or_trim_slice(upstream: &[f64], timeline_len: usize) -> Vec<f64> {
    if upstream.len() >= timeline_len {
        return upstream.iter().copied().take(timeline_len).collect();
    }
    let mut padded = upstream.to_vec();
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
        return pad_or_trim_slice(upstream, timeline_len);
    }
    vec![0.0; timeline_len]
}

fn collect_upstream_legs(
    graph: &StableGraph<ExecutionNode, ()>,
    index: NodeIndex,
    node_outputs: &HashMap<NodeIndex, NodeRuntimeOutput>,
    node_prim_paths: &[Arc<str>],
    quotes: &HashMap<String, AssetQuote>,
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
            let asset_id = output
                .and_then(|o| o.asset_symbol.clone())
                .or_else(|| {
                    graph.node_weight(upstream_index).and_then(|node| match node {
                        ExecutionNode::DataInput { symbol } => Some(symbol.clone()),
                        _ => None,
                    })
                })
                .or_else(|| {
                    node_prim_paths
                        .get(upstream_index.index())
                        .filter(|path| !path.is_empty())
                        .map(|path| path.to_string())
                })
                .unwrap_or_else(|| format!("leg_{upstream_index:?}"));
            let series = output.map(|o| o.scalar.clone()).unwrap_or_default();
            let series = if leg_kind == ClosureLegKind::Asset {
                trade_gate_series_from_ta_upstream(
                    graph,
                    upstream_index,
                    &asset_id,
                    series,
                    node_outputs,
                    quotes,
                )
            } else {
                series
            };
            (asset_id, series, leg_kind)
        })
        .collect()
}

/// Convert price-scale TA indicators into {-1,0,1} trade gates for portfolio sweeps.
fn trade_gate_series_from_ta_upstream(
    graph: &StableGraph<ExecutionNode, ()>,
    upstream_index: NodeIndex,
    asset_id: &str,
    indicator_series: Vec<f64>,
    node_outputs: &HashMap<NodeIndex, NodeRuntimeOutput>,
    quotes: &HashMap<String, AssetQuote>,
) -> Vec<f64> {
    let Some(ExecutionNode::SignalTransform { expression, .. }) =
        graph.node_weight(upstream_index)
    else {
        return indicator_series;
    };
    if indicator_series.is_empty() || !expression.contains("ta::") {
        return indicator_series;
    }

    if indicator_series.iter().all(|value| {
        *value == -1.0 || *value == 0.0 || *value == 1.0
    }) && indicator_series.iter().any(|value| *value != 0.0)
    {
        return indicator_series;
    }

    let price_series = graph
        .neighbors_directed(upstream_index, Direction::Incoming)
        .find_map(|neighbor| node_outputs.get(&neighbor).map(|output| output.scalar.clone()))
        .or_else(|| {
            quotes
                .get(asset_id)
                .map(|quote| quote.price_series.as_ref().to_vec())
        })
        .unwrap_or_default();
    let len = price_series.len().min(indicator_series.len());
    if len < 2 {
        return indicator_series;
    }
    let price_series = &price_series[..len];
    let indicator_series = &indicator_series[..len];

    let expression = expression.to_ascii_lowercase();
    if expression.contains("ta::sma")
        || expression.contains("ta::ema")
        || expression.contains("ta::wma")
        || expression.contains("ta::hma")
        || expression.contains("ta::tema")
    {
        return price_series
            .iter()
            .zip(indicator_series.iter())
            .map(|(price, indicator)| {
                if !price.is_finite() || !indicator.is_finite() {
                    0.0
                } else if *price > *indicator {
                    1.0
                } else if *price < *indicator {
                    -1.0
                } else {
                    0.0
                }
            })
            .collect();
    }

    if expression.contains("ta::rsi") || expression.contains("ta::cci") {
        return indicator_series
            .iter()
            .map(|value| {
                if !value.is_finite() {
                    0.0
                } else if *value > 50.0 {
                    1.0
                } else if *value < 50.0 {
                    -1.0
                } else {
                    0.0
                }
            })
            .collect();
    }

    indicator_series.to_vec()
}

fn gather_upstream_into(
    scratch: &mut Vec<f64>,
    graph: &StableGraph<ExecutionNode, ()>,
    index: NodeIndex,
    node_outputs: &HashMap<NodeIndex, NodeRuntimeOutput>,
) {
    scratch.clear();
    for upstream_index in graph.neighbors_directed(index, Direction::Incoming) {
        if let Some(output) = node_outputs.get(&upstream_index) {
            scratch.extend_from_slice(&output.scalar);
        }
    }
}

fn build_asset_quotes_from_window(window: &MarketTimelineWindow) -> HashMap<String, AssetQuote> {
    let mut quotes = HashMap::new();
    for (path, series) in window.price_vectors().iter() {
        quotes.insert(
            path.to_string(),
            AssetQuote {
                price_series: Arc::clone(series),
                contract_multiplier: 1.0,
            },
        );
    }
    quotes
}

fn build_asset_quotes_from_vectors(
    asset_vectors: &HashMap<String, Arc<[f64]>>,
) -> HashMap<String, AssetQuote> {
    asset_vectors
        .iter()
        .map(|(path, series)| {
            (
                path.clone(),
                AssetQuote {
                    price_series: Arc::clone(series),
                    contract_multiplier: 1.0,
                },
            )
        })
        .collect()
}

fn asset_vectors_from_window(window: &MarketTimelineWindow) -> HashMap<String, Arc<[f64]>> {
    window
        .price_vectors()
        .iter()
        .map(|(path, series)| (path.to_string(), Arc::clone(series)))
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
    let mut altered_weights: HashMap<String, Vec<f64>> = HashMap::new();
    let mut calculated_units: HashMap<String, Vec<f64>> = HashMap::new();
    let mut investment_returns: HashMap<String, Vec<f64>> = HashMap::new();

    for frame in matrix {
        altered_weights
            .entry(frame.asset_id.clone())
            .or_default()
            .push(frame.altered_portfolio_weight);
        calculated_units
            .entry(frame.asset_id.clone())
            .or_default()
            .push(frame.calculated_units);
        investment_returns
            .entry(frame.asset_id.clone())
            .or_default()
            .push(frame.investment_return);
    }

    for (asset_id, values) in altered_weights {
        streams.push(ComputedAttributeStream {
            prim_path: prim_path.to_string(),
            attribute: format!("outputs:tracking:altered_weight:{asset_id}"),
            values,
        });
    }
    for (asset_id, values) in calculated_units {
        streams.push(ComputedAttributeStream {
            prim_path: prim_path.to_string(),
            attribute: format!("outputs:tracking:calculated_units:{asset_id}"),
            values,
        });
    }
    for (asset_id, values) in investment_returns {
        streams.push(ComputedAttributeStream {
            prim_path: prim_path.to_string(),
            attribute: format!("outputs:tracking:investment_return:{asset_id}"),
            values,
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

    fn price_cols(vectors: HashMap<String, Vec<f64>>) -> HashMap<String, SharedPriceColumn> {
        crate::engine::shared_columns_from_vec(vectors)
    }

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
            path_bindings: PathBindingIndex::default(),
            asset_registry: HashMap::new(),
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
                        tier_kind: None,
                        tier_program: None,
                    },
                ),
                (
                    "/portfolios/main".to_string(),
                    ExecutionNode::PortfolioSink {
                        method: "Allocation::EqualWeight".to_string(),
                        initial_capital: 1_000_000.0,
                        otl_script: String::new(),
                        otl_hook: None,
                        tier_kind: None,
                        tier_program: None,
                    },
                ),
            ],
            wires: stage_snapshot().wires,
        };

        let mut engine = MarketLabGraphEngine::compile(spec).expect("valid dag");
        let mut assets = HashMap::new();
        assets.insert("/assets/SPY".to_string(), vec![100.0, 101.0, 102.0]);
        let result = engine.execute_timeline(price_cols(assets), 3);
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
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("compile with otl");
        let assets = HashMap::from([("/assets/SPY".to_string(), vec![1.0, 2.0, 3.0, 4.0, 5.0])]);
        let result = engine.execute_timeline(price_cols(assets), 5);
        let signal = result
            .streams
            .iter()
            .find(|stream| {
                stream.prim_path == "/analytics/rsi"
                    && stream.attribute == "outputs:signal"
            })
            .expect("signal stream");
        assert_eq!(signal.values.len(), 5);
        assert!(signal.values.iter().any(|value| value.is_finite() && *value > 0.0));
    }

    #[test]
    fn tier_signal_sma_has_no_lookahead_in_timeline() {
        let mut snapshot = stage_snapshot();
        snapshot.prims[1].attributes.insert(
            "inputs:script_src".to_string(),
            r#"
signal trend_alpha(input closure raw, output closure gated) {
    gated = sma(data, 5);
}
"#
            .trim()
            .to_string(),
        );
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("compile tier signal");
        let baseline_prices: Vec<f64> = (0..25).map(|bar| (bar as f64) + 1.0).collect();
        let baseline_assets = HashMap::from([("/assets/SPY".to_string(), baseline_prices.clone())]);
        let baseline = engine
            .execute_timeline(price_cols(baseline_assets), 25)
            .streams
            .iter()
            .find(|stream| stream.prim_path == "/analytics/rsi")
            .expect("baseline stream")
            .values
            .clone();

        for bar in 4..25 {
            let mut poisoned = baseline_prices.clone();
            for value in poisoned.iter_mut().skip(bar + 1) {
                *value = 999.0;
            }
            let poisoned_assets = HashMap::from([("/assets/SPY".to_string(), poisoned)]);
            let perturbed_value = engine
                .execute_timeline(price_cols(poisoned_assets), 25)
                .streams
                .iter()
                .find(|stream| stream.prim_path == "/analytics/rsi")
                .expect("perturbed stream")
                .values[bar];
            assert_eq!(
                baseline[bar], perturbed_value,
                "tier sma at bar {bar} must not depend on future prices"
            );
        }
    }

    #[test]
    fn parallel_waves_run_two_tier_signals_at_same_level() {
        let signal_src = r#"
signal pass(input closure raw, output closure gated) {
    gated = data;
}
"#;
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
                (
                    "/sig/a".to_string(),
                    ExecutionNode::SignalTransform {
                        expression: signal_src.to_string(),
                        compiled_fn: None,
                        compiled_multi: None,
                        tier_kind: None,
                        tier_program: None,
                    },
                ),
                (
                    "/sig/b".to_string(),
                    ExecutionNode::SignalTransform {
                        expression: signal_src.to_string(),
                        compiled_fn: None,
                        compiled_multi: None,
                        tier_kind: None,
                        tier_program: None,
                    },
                ),
            ],
            wires: vec![
                GraphCompileWire {
                    source_prim_path: "/assets/A".to_string(),
                    target_prim_path: "/sig/a".to_string(),
                    relationship: "inputs:underlying".to_string(),
                },
                GraphCompileWire {
                    source_prim_path: "/assets/B".to_string(),
                    target_prim_path: "/sig/b".to_string(),
                    relationship: "inputs:underlying".to_string(),
                },
            ],
        };
        let mut engine = MarketLabGraphEngine::compile(spec).expect("compile graph");
        engine = engine.compile_otl_scripts().expect("compile tier scripts");
        let levels = engine.execution_levels();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1].len(), 2);

        let assets = HashMap::from([
            ("A".to_string(), (0..15).map(|bar| bar as f64).collect::<Vec<_>>()),
            ("B".to_string(), (0..15).map(|bar| 100.0 + bar as f64).collect::<Vec<_>>()),
        ]);
        let result = engine.execute_timeline(price_cols(assets), 15);
        let a = result
            .streams
            .iter()
            .find(|stream| stream.prim_path == "/sig/a")
            .expect("signal a");
        let b = result
            .streams
            .iter()
            .find(|stream| stream.prim_path == "/sig/b")
            .expect("signal b");
        assert!(
            a.values
                .iter()
                .zip(&b.values)
                .any(|(left, right)| (left - right).abs() > 1e-9),
            "parallel sweeps must reflect distinct upstream series"
        );
    }

    #[test]
    fn execute_timeline_runs_three_tier_signal_program() {
        let mut snapshot = stage_snapshot();
        snapshot.prims[1].attributes.insert(
            "inputs:script_src".to_string(),
            r#"
signal trend_alpha(
    input closure raw,
    output closure gated
) {
    gated = sma(data, 5);
}
"#
            .trim()
            .to_string(),
        );
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&snapshot).expect("compile tier signal");
        let prices: Vec<f64> = (0..20).map(|bar| 100.0 + bar as f64).collect();
        let assets = HashMap::from([("/assets/SPY".to_string(), prices)]);
        let result = engine.execute_timeline(price_cols(assets), 20);
        let signal = result
            .streams
            .iter()
            .find(|stream| stream.prim_path == "/analytics/rsi")
            .expect("tier signal stream");
        assert!(
            signal
                .values
                .iter()
                .any(|value| value.is_finite() && *value != 0.0),
            "tier sweep should populate conviction samples"
        );
    }

    #[test]
    fn evaluate_node_closures_extracts_live_signal_closure() {
        let mut engine =
            MarketLabGraphEngine::compile_from_stage(&stage_snapshot()).expect("compile");
        engine
            .set_signal_compiled_fn("/analytics/rsi", Box::new(|input| input.to_vec()))
            .expect("attach closure");

        let ctx = EvaluationContext {
            timeline_len: 3,
            asset_vectors: HashMap::from([(
                "/assets/SPY".to_string(),
                Arc::from([10.0, 20.0, 30.0]),
            )]),
        };
        let closures = engine
            .evaluate_node_closures(
                "/analytics/rsi",
                OtlObjectKind::LegacyShader,
                &ctx,
            )
            .expect("closures");
        assert_eq!(closures.len(), 1);
        assert_eq!(closures[0].asset_id, "SPY");
        assert!(closures[0].closure_raw_weight > 0.0);
        assert_eq!(closures[0].signal_series.len(), 3);
    }
}
