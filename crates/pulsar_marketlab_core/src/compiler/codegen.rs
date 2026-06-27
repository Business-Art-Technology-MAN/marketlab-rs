//! Compile OTL three-tier object AST into bar-sweep execution engines.

use std::collections::HashMap;
use std::sync::Arc;

use nalgebra::DMatrix;

use crate::engine::{
    allocation_weights_from_covariance, fill_subcovariance_block, uses_covariance_optimizer,
    PrecomputedMatrixCache,
};
use crate::execution_matrix::{ExecutionContext, GraphSeriesMatrix, RuntimeEngineError};
use crate::{
    apply_alpha_conviction, compile_unified_script, compute_allocation_weights,
    conviction_scale_from_signal_series, AssetQuote, ClosureLegKind, CompileError, CompiledSeries,
    DirectionalDistribution, OtlObjectDeclaration, OtlObjectKind, OtlProgram, PortDirection,
    SeriesClosure, Statement, SymbolicOtlClosure,
};

/// Column and upstream-series lookup table for wiring OTL object ports.
#[derive(Debug, Clone, Default)]
pub struct ObjectCodegenRegistry {
    pub signal_columns: HashMap<String, usize>,
    pub upstream_series: Vec<Vec<f64>>,
    pub allocation_method: String,
    pub initial_capital: f64,
    pub asset_quotes: HashMap<String, AssetQuote>,
    /// Upstream `inputs:sources` prim paths (structural; resolved at compile time only).
    pub source_prim_paths: Vec<Arc<str>>,
    /// Portfolio integrator prim receiving `outputs:weights` side-channel logs.
    pub portfolio_prim_path: Option<Arc<str>>,
}

impl ObjectCodegenRegistry {
    pub fn register_signal_column(&mut self, port_name: impl Into<String>, column_index: usize) {
        self.signal_columns.insert(port_name.into(), column_index);
    }

    pub fn upstream_at(&self, index: usize) -> Option<&[f64]> {
        self.upstream_series.get(index).map(Vec::as_slice)
    }
}

/// Compiled tier-specific vector sweep engine.
#[derive(Debug)]
pub enum CompiledProgramTier {
    Signal(SignalExecutionEngine),
    Allocator(AllocatorExecutionEngine),
    Portfolio(PortfolioExecutionEngine),
}

impl CompiledProgramTier {
    /// Independent sweep state for parallel execution (reuses compiled closures).
    pub fn fork_for_sweep(&self) -> Self {
        match self {
            Self::Signal(engine) => Self::Signal(engine.fork_for_sweep()),
            Self::Allocator(engine) => Self::Allocator(engine.fork_for_sweep()),
            Self::Portfolio(engine) => Self::Portfolio(engine.fork_for_sweep()),
        }
    }
}

type SharedSeriesFn = Arc<dyn Fn(&[f64]) -> Vec<f64> + Send + Sync>;

fn series_is_discrete_gate(series: &[f64]) -> bool {
    !series.is_empty()
        && series.iter().all(|value| {
            value.is_finite() && matches!(*value, -1.0 | 0.0 | 1.0)
        })
}

/// Signal tier: alpha conviction stream over the full timeline.
pub struct SignalExecutionEngine {
    convictions: Vec<f64>,
    series_fn: SharedSeriesFn,
    alpha_script: String,
}

impl std::fmt::Debug for SignalExecutionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalExecutionEngine")
            .field("convictions", &self.convictions)
            .field("series_fn", &"<series closure>")
            .field("alpha_script", &self.alpha_script)
            .finish()
    }
}

impl SignalExecutionEngine {
    /// Precompute the full conviction vector in one pass. Series intrinsics (e.g. `sma`) are
    /// causal on `upstream`; per-bar [`Self::execute_at_bar`] only indexes this buffer.
    pub fn prepare(&mut self, upstream: &[f64], bar_count: usize) {
        self.convictions = pad_or_trim((self.series_fn)(upstream), bar_count);
        if !series_is_discrete_gate(&self.convictions) {
            let scale = conviction_scale_from_signal_series(&self.convictions);
            for value in &mut self.convictions {
                *value = apply_alpha_conviction(*value, &self.alpha_script, scale);
            }
        }
    }

    pub fn execute_at_bar(&self, bar_idx: usize, _ctx: &ExecutionContext) -> f64 {
        self.convictions
            .get(bar_idx)
            .copied()
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
    }

    pub fn convictions(&self) -> &[f64] {
        &self.convictions
    }

    pub fn fork_for_sweep(&self) -> Self {
        Self {
            convictions: Vec::new(),
            series_fn: Arc::clone(&self.series_fn),
            alpha_script: self.alpha_script.clone(),
        }
    }
}

/// Allocator tier: blend upstream signal columns into target weights.
#[derive(Debug)]
pub struct AllocatorExecutionEngine {
    signal_column_indices: Vec<usize>,
    blend_weights: Vec<f64>,
    pub(crate) leg_weights: Vec<Vec<f64>>,
    allocation_method: String,
    alpha_script: String,
    leg_signals_scratch: Vec<f64>,
    weights_scratch: Vec<f64>,
    closures_scratch: Vec<SymbolicOtlClosure>,
}

impl AllocatorExecutionEngine {
    pub fn allocate_capital_at_bar(
        &mut self,
        bar_idx: usize,
        ctx: &ExecutionContext,
        output_matrix: &mut GraphSeriesMatrix,
    ) -> Result<(), RuntimeEngineError> {
        if bar_idx >= ctx.timeline_length() {
            return Ok(());
        }

        self.leg_signals_scratch.clear();
        for column_index in &self.signal_column_indices {
            let conviction = output_matrix
                .read_signal(*column_index, bar_idx)
                .ok_or(RuntimeEngineError::MissingSignalColumn {
                    column_index: *column_index,
                })?;
            self.leg_signals_scratch.push(conviction);
        }

        let leg_count = self.leg_signals_scratch.len().max(1);
        self.weights_scratch.resize(leg_count, 0.0);
        if self.blend_weights.len() == leg_count {
            self.weights_scratch.copy_from_slice(&self.blend_weights);
        } else {
            let uniform = 1.0 / leg_count as f64;
            self.weights_scratch.fill(uniform);
        }

        let total_signal: f64 = self.leg_signals_scratch.iter().map(|value| value.abs()).sum();
        if total_signal > f64::EPSILON {
            for (weight, signal) in self
                .weights_scratch
                .iter_mut()
                .zip(self.leg_signals_scratch.iter())
            {
                *weight *= signal.abs() / total_signal;
            }
        }

        for (index, signal) in self.leg_signals_scratch.iter().enumerate() {
            let closure = &mut self.closures_scratch[index];
            closure.closure_raw_weight = self.weights_scratch[index];
            if closure.signal_series.is_empty() {
                closure.signal_series.push(*signal);
            } else {
                closure.signal_series[0] = *signal;
            }
        }

        let normalized = compute_allocation_weights(
            &self.allocation_method,
            &self.closures_scratch,
            &ctx.asset_quotes,
        );
        for (leg_index, weight) in normalized.iter().enumerate() {
            let signal = self.leg_signals_scratch[leg_index];
            let scaled = apply_alpha_conviction(
                *weight,
                &self.alpha_script,
                conviction_scale_from_signal_series(&[signal]),
            );
            if let Some(buffer) = self.leg_weights.get_mut(leg_index) {
                buffer[bar_idx] = scaled;
                output_matrix.write_allocator_weight(leg_index, bar_idx, scaled);
            }
        }
        Ok(())
    }

    pub fn fork_for_sweep(&self) -> Self {
        Self {
            signal_column_indices: self.signal_column_indices.clone(),
            blend_weights: self.blend_weights.clone(),
            leg_weights: self
                .leg_weights
                .iter()
                .map(|buffer| vec![0.0; buffer.len()])
                .collect(),
            allocation_method: self.allocation_method.clone(),
            alpha_script: self.alpha_script.clone(),
            leg_signals_scratch: vec![0.0; self.leg_signals_scratch.len()],
            weights_scratch: vec![0.0; self.weights_scratch.len()],
            closures_scratch: self.closures_scratch.clone(),
        }
    }
}

/// Portfolio tier: NAV, cash, and drawdown vectors from allocator weights.
#[derive(Debug)]
pub struct PortfolioExecutionEngine {
    allocator_leg_count: usize,
    pub(crate) initial_capital: f64,
    pub(crate) nav: Vec<f64>,
    pub(crate) cash: Vec<f64>,
    pub(crate) drawdown: Vec<f64>,
    alpha_script: String,
    /// Structural source identities (compile-time); used only inside bar evaluation.
    source_prim_paths: Vec<Arc<str>>,
    track_prim_path: Arc<str>,
    /// Per-bar encoded `outputs:weights` strings (filled during deferred sweep only).
    pub(crate) weight_encodings: Vec<String>,
    /// Reused scratch for normalized leg weights (no per-bar map allocation).
    weights_scratch: Vec<f64>,
    allocation_method: String,
    /// Row-major `leg_count × leg_count` sub-covariance scratch (no per-bar matrix alloc).
    subcov_scratch: Vec<f64>,
}

impl PortfolioExecutionEngine {
    pub fn track_portfolio_metrics_at_bar(
        &mut self,
        bar_idx: usize,
        ctx: &ExecutionContext,
        output_matrix: &mut GraphSeriesMatrix,
    ) -> Result<(), RuntimeEngineError> {
        let bar_count = ctx.timeline_length();
        if bar_idx >= bar_count {
            return Ok(());
        }

        let mut deployed = 0.0_f64;
        for leg in 0..self.allocator_leg_count {
            let weight = output_matrix
                .read_allocator_weight(leg, bar_idx)
                .ok_or(RuntimeEngineError::MissingAllocatorStream { leg_index: leg })?;
            deployed += weight.abs();
        }
        deployed = deployed.clamp(0.0, 1.0);

        let prior_nav = if bar_idx == 0 {
            self.initial_capital
        } else {
            self.nav[bar_idx - 1]
        };
        let prior_cash = if bar_idx == 0 {
            self.initial_capital
        } else {
            self.cash[bar_idx - 1]
        };

        let cash_target = prior_nav * (1.0 - deployed);
        let mut cash = prior_cash * 0.25 + cash_target * 0.75;
        if self.alpha_script.contains("drawdown") {
            let peak = self.nav[..bar_idx].iter().copied().fold(prior_nav, f64::max);
            let dd = if peak > f64::EPSILON {
                (peak - prior_nav) / peak
            } else {
                0.0
            };
            cash *= (1.0 - dd * 0.5).clamp(0.1, 1.0);
        }

        let nav = prior_nav + (self.initial_capital * deployed - prior_cash + cash) * 0.001;
        let peak = self.nav[..=bar_idx]
            .iter()
            .copied()
            .fold(nav, f64::max)
            .max(self.initial_capital);
        let dd = if peak > f64::EPSILON {
            (peak - nav) / peak
        } else {
            0.0
        };

        self.nav[bar_idx] = nav;
        self.cash[bar_idx] = cash;
        self.drawdown[bar_idx] = dd;
        output_matrix.write_nav(bar_idx, nav);
        output_matrix.write_cash(bar_idx, cash);

        // Deferred weight serialization (never runs at compile time).
        self.record_weight_encoding_at_bar(bar_idx, output_matrix, ctx);
        Ok(())
    }

    fn record_weight_encoding_at_bar(
        &mut self,
        bar_idx: usize,
        output_matrix: &GraphSeriesMatrix,
        ctx: &ExecutionContext,
    ) {
        let leg_count = self
            .source_prim_paths
            .len()
            .min(self.allocator_leg_count);
        if leg_count == 0 {
            if let Some(slot) = self.weight_encodings.get_mut(bar_idx) {
                slot.clear();
            }
            return;
        }

        if self.weights_scratch.len() < leg_count {
            self.weights_scratch.resize(leg_count, 0.0);
        }

        let method = if ctx.allocation_method.is_empty() {
            self.allocation_method.clone()
        } else {
            ctx.allocation_method.clone()
        };

        let cache = ctx.covariance_cache.clone();
        let mut applied_covariance = false;
        if uses_covariance_optimizer(&method) {
            if let Some(tensor) = cache.as_ref() {
                applied_covariance =
                    self.apply_covariance_weights(bar_idx, tensor, &method, leg_count);
            }
        }

        if !applied_covariance {
            let mut sum = 0.0_f64;
            for leg in 0..leg_count {
                let weight = output_matrix
                    .read_allocator_weight(leg, bar_idx)
                    .unwrap_or(0.0)
                    .abs();
                self.weights_scratch[leg] = weight;
                sum += weight;
            }

            if sum > f64::EPSILON {
                for value in &mut self.weights_scratch[..leg_count] {
                    *value /= sum;
                }
            } else {
                let uniform = 1.0 / leg_count as f64;
                self.weights_scratch[..leg_count].fill(uniform);
            }
        }

        let encoded = crate::serialize_portfolio_weights_from_slices(
            &self.source_prim_paths[..leg_count],
            &self.weights_scratch[..leg_count],
        );

        if let Some(slot) = self.weight_encodings.get_mut(bar_idx) {
            if slot.capacity() < encoded.len() {
                slot.reserve(encoded.len().saturating_sub(slot.capacity()));
            }
            slot.clear();
            slot.push_str(&encoded);
        }

        if let Some(track) = &ctx.weight_track {
            track(bar_idx, self.track_prim_path.as_ref(), &encoded);
        }
    }

    fn apply_covariance_weights(
        &mut self,
        bar_idx: usize,
        cache: &PrecomputedMatrixCache,
        method: &str,
        leg_count: usize,
    ) -> bool {
        let full = cache.matrix_at(bar_idx);
        let needed = leg_count * leg_count;
        if self.subcov_scratch.len() < needed {
            self.subcov_scratch.resize(needed, 0.0);
        }
        fill_subcovariance_block(
            full,
            &cache.path_to_index,
            &self.source_prim_paths[..leg_count],
            &mut self.subcov_scratch,
            leg_count,
        );
        let cov_view =
            DMatrix::from_row_slice(leg_count, leg_count, &self.subcov_scratch[..needed]);
        allocation_weights_from_covariance(method, &cov_view, &mut self.weights_scratch[..leg_count]);
        true
    }

    pub fn weight_encodings(&self) -> &[String] {
        &self.weight_encodings
    }

    pub fn nav_series(&self) -> &[f64] {
        &self.nav
    }

    pub fn cash_series(&self) -> &[f64] {
        &self.cash
    }

    pub fn fork_for_sweep(&self) -> Self {
        let bar_len = self.nav.len().max(1);
        Self {
            allocator_leg_count: self.allocator_leg_count,
            initial_capital: self.initial_capital,
            nav: vec![self.initial_capital; bar_len],
            cash: vec![self.initial_capital; bar_len],
            drawdown: vec![0.0; bar_len],
            alpha_script: self.alpha_script.clone(),
            source_prim_paths: self.source_prim_paths.clone(),
            track_prim_path: Arc::clone(&self.track_prim_path),
            weight_encodings: vec![String::new(); bar_len],
            weights_scratch: self.weights_scratch.clone(),
            allocation_method: self.allocation_method.clone(),
            subcov_scratch: self.subcov_scratch.clone(),
        }
    }
}

/// Compile the primary OTL object in `ast_program` into a tier execution engine.
pub fn compile_object_program(
    ast_program: &OtlProgram,
    registry: &ObjectCodegenRegistry,
) -> Result<CompiledProgramTier, CompileError> {
    let object = ast_program
        .primary_object()
        .ok_or(CompileError::EmptyInput)?;

    match object.kind {
        OtlObjectKind::Signal => Ok(CompiledProgramTier::Signal(compile_signal_engine(
            object, registry,
        )?)),
        OtlObjectKind::Allocator => Ok(CompiledProgramTier::Allocator(
            compile_allocator_engine(object, registry)?,
        )),
        OtlObjectKind::Portfolio => Ok(CompiledProgramTier::Portfolio(
            compile_portfolio_engine(object, registry)?,
        )),
        OtlObjectKind::LegacyShader => Ok(CompiledProgramTier::Signal(compile_signal_engine(
            object, registry,
        )?)),
    }
}

fn compile_signal_engine(
    object: &OtlObjectDeclaration,
    registry: &ObjectCodegenRegistry,
) -> Result<SignalExecutionEngine, CompileError> {
    let script = body_to_runtime_expression(object);
    let series_fn = Arc::from(compile_series_closure(&script)?);
    let _upstream = registry.upstream_at(0).unwrap_or(&[]);
    Ok(SignalExecutionEngine {
        convictions: Vec::new(),
        series_fn,
        alpha_script: script,
    })
}

fn compile_allocator_engine(
    object: &OtlObjectDeclaration,
    registry: &ObjectCodegenRegistry,
) -> Result<AllocatorExecutionEngine, CompileError> {
    let signal_column_indices = resolve_signal_inputs(object, registry)?;
    let blend_weights = parse_blend_weights(&body_to_runtime_script(object), signal_column_indices.len());
    let bar_hint = registry
        .upstream_series
        .first()
        .map(|series| series.len())
        .unwrap_or(0);
    let leg_weights = (0..signal_column_indices.len())
        .map(|_| vec![0.0; bar_hint.max(1)])
        .collect();

    let leg_count = signal_column_indices.len().max(1);
    let closures_scratch = (0..leg_count)
        .map(|index| SymbolicOtlClosure {
            asset_id: format!("leg_{index}"),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 0.0,
            signal_series: vec![0.0],
            leg_kind: ClosureLegKind::Asset,
        })
        .collect();
    Ok(AllocatorExecutionEngine {
        signal_column_indices,
        blend_weights,
        leg_weights,
        allocation_method: registry.allocation_method.clone(),
        alpha_script: body_to_runtime_script(object),
        leg_signals_scratch: vec![0.0; leg_count],
        weights_scratch: vec![0.0; leg_count],
        closures_scratch,
    })
}

fn compile_portfolio_engine(
    object: &OtlObjectDeclaration,
    registry: &ObjectCodegenRegistry,
) -> Result<PortfolioExecutionEngine, CompileError> {
    let leg_count = registry
        .source_prim_paths
        .len()
        .max(
            object
                .inputs
                .iter()
                .filter(|port| port.direction == PortDirection::Input && port.ty.is_closure())
                .count(),
        )
        .max(registry.signal_columns.len())
        .max(1);
    let bar_hint = registry
        .upstream_series
        .first()
        .map(|series| series.len())
        .unwrap_or(0);
    let len = bar_hint.max(1);
    let track_prim_path = registry
        .portfolio_prim_path
        .clone()
        .unwrap_or_else(|| Arc::from("/MarketLab/Portfolios/unknown"));
    let source_prim_paths = if registry.source_prim_paths.is_empty() {
        (0..leg_count)
            .map(|index| Arc::from(format!("leg_{index}")))
            .collect()
    } else {
        registry.source_prim_paths.clone()
    };
    Ok(PortfolioExecutionEngine {
        allocator_leg_count: leg_count,
        initial_capital: registry.initial_capital,
        nav: vec![registry.initial_capital; len],
        cash: vec![registry.initial_capital; len],
        drawdown: vec![0.0; len],
        alpha_script: body_to_runtime_script(object),
        source_prim_paths,
        track_prim_path,
        weight_encodings: vec![String::new(); len],
        weights_scratch: vec![0.0; leg_count],
        allocation_method: registry.allocation_method.clone(),
        subcov_scratch: vec![0.0; leg_count * leg_count],
    })
}

fn resolve_signal_inputs(
    object: &OtlObjectDeclaration,
    registry: &ObjectCodegenRegistry,
) -> Result<Vec<usize>, CompileError> {
    let mut indices = Vec::new();
    for input in object
        .inputs
        .iter()
        .filter(|port| port.direction == PortDirection::Input && port.ty.is_closure())
    {
        if let Some(index) = registry.signal_columns.get(&input.name).copied() {
            indices.push(index);
        } else if input.ty.is_closure_array() {
            let mut array_indices: Vec<usize> = registry
                .signal_columns
                .values()
                .copied()
                .collect();
            array_indices.sort_unstable();
            if array_indices.is_empty() {
                return Err(CompileError::UnknownIdentifier(input.name.clone()));
            }
            indices.extend(array_indices);
        } else {
            return Err(CompileError::UnknownIdentifier(input.name.clone()));
        }
    }
    if indices.is_empty() {
        let fallback: Vec<usize> = registry.signal_columns.values().copied().collect();
        if fallback.is_empty() {
            return Err(CompileError::UnknownIdentifier("signal".to_string()));
        }
        Ok(fallback)
    } else {
        Ok(indices)
    }
}

fn compile_series_closure(script: &str) -> Result<SeriesClosure, CompileError> {
    match compile_unified_script(script)? {
        CompiledSeries::Single(closure) => Ok(closure),
        CompiledSeries::Multi(closure, _) => Ok(Box::new(move |input| {
            closure(input)
                .into_iter()
                .next()
                .unwrap_or_default()
        })),
    }
}

fn body_to_runtime_script(object: &OtlObjectDeclaration) -> String {
    object
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::Assign { target, expr } => Some(format!("{target} = {expr}")),
            Statement::Return { expr } => Some(expr.clone()),
            Statement::Raw { text } => Some(text.clone()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_to_runtime_expression(object: &OtlObjectDeclaration) -> String {
    for statement in &object.body {
        match statement {
            Statement::Assign { expr, .. } | Statement::Return { expr } => {
                return expr.trim().trim_end_matches(';').trim().to_string();
            }
            Statement::Raw { text } if !text.trim().is_empty() => {
                return text.clone();
            }
            _ => {}
        }
    }
    body_to_runtime_script(object)
}

fn parse_blend_weights(script: &str, leg_count: usize) -> Vec<f64> {
    if leg_count == 0 {
        return Vec::new();
    }
    if script.contains("0.5") || script.contains("half") || script.contains("mix(") {
        if leg_count == 2 {
            return vec![0.5, 0.5];
        }
        return vec![1.0 / leg_count as f64; leg_count];
    }
    vec![1.0 / leg_count as f64; leg_count]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::compile_object_program as parse_object_program;

    #[test]
    fn compiles_signal_tier_from_otl_program() {
        let source = r#"
signal alpha_gate(input closure raw, output closure gated) {
    gated = data;
}
"#;
        let program = parse_object_program(source).expect("parse");
        let mut registry = ObjectCodegenRegistry::default();
        registry.upstream_series.push((0..10).map(|index| index as f64).collect());
        let tier = compile_object_program(&program, &registry).expect("compile");
        assert!(matches!(tier, CompiledProgramTier::Signal(_)));
    }

    #[test]
    fn compiles_allocator_requires_signal_columns() {
        let source = r#"
allocator hrp_blend(input closure[] legs, output closure blended) {
    blended = mix(legs[0], legs[1], 0.5);
}
"#;
        let program = parse_object_program(source).expect("parse");
        let registry = ObjectCodegenRegistry::default();
        let err = compile_object_program(&program, &registry).expect_err("missing columns");
        assert!(matches!(err, CompileError::UnknownIdentifier(_)));
    }

    #[test]
    fn portfolio_tier_emits_weight_encodings_during_deferred_sweep() {
        use crate::evaluate_compiled_tier;
        use crate::execution_matrix::{ExecutionContext, GraphSeriesMatrix};

        let source = r#"
portfolio master_fund(input closure[] legs, output closure nav) {
    nav = legs[0];
}
"#;
        let program = parse_object_program(source).expect("parse");
        let mut registry = ObjectCodegenRegistry::default();
        registry.portfolio_prim_path = Some(Arc::from("/MarketLab/Portfolios/node_00000007"));
        registry
            .source_prim_paths
            .push(Arc::from("/MarketLab/Universe/node_00000001"));
        registry
            .source_prim_paths
            .push(Arc::from("/MarketLab/Universe/node_00000002"));
        registry.register_signal_column("leg_0", 0);
        registry.register_signal_column("leg_1", 1);

        let mut tier = compile_object_program(&program, &registry).expect("compile portfolio");
        let CompiledProgramTier::Portfolio(_) = &tier else {
            panic!("expected portfolio tier");
        };

        let bar_count = 4usize;
        let mut ctx = ExecutionContext::new(
            bar_count,
            10_000.0,
            "Allocation::EqualWeight",
            HashMap::new(),
            HashMap::new(),
        );
        let mut matrix = GraphSeriesMatrix::with_capacity(bar_count, 2, 2);
        for bar in 0..bar_count {
            matrix.write_allocator_weight(0, bar, 0.5);
            matrix.write_allocator_weight(1, bar, 0.5);
        }

        evaluate_compiled_tier(&mut ctx, &mut tier, &mut matrix).expect("sweep");

        let CompiledProgramTier::Portfolio(engine) = &tier else {
            panic!("expected portfolio tier");
        };
        assert_eq!(engine.weight_encodings().len(), bar_count);
        assert!(engine.weight_encodings()[0].contains("/MarketLab/Universe/node_00000001"));
        assert!(engine.weight_encodings()[0].contains("0.5000"));
    }
}
