//! Bar-indexed vector sweep for compiled OTL program tiers.

use crate::compiler::{
    compile_object_program, AllocatorExecutionEngine, CompiledProgramTier, ObjectCodegenRegistry,
    PortfolioExecutionEngine, SignalExecutionEngine,
};
use crate::frontend::OtlProgram;

use crate::execution_matrix::{ExecutionContext, GraphSeriesMatrix, RuntimeEngineError};

/// Compile an OTL program and sweep all bars into `output_matrix`.
pub fn evaluate_node_vector_series(
    ctx: &mut ExecutionContext,
    program: &OtlProgram,
    registry: &ObjectCodegenRegistry,
    output_matrix: &mut GraphSeriesMatrix,
) -> Result<(), RuntimeEngineError> {
    let mut compiled_tier =
        compile_object_program(program, registry).map_err(RuntimeEngineError::Compile)?;
    evaluate_compiled_tier(ctx, &mut compiled_tier, output_matrix)
}

/// Sweep a pre-compiled tier across the timeline.
pub fn evaluate_compiled_tier(
    ctx: &mut ExecutionContext,
    compiled_tier: &mut CompiledProgramTier,
    output_matrix: &mut GraphSeriesMatrix,
) -> Result<(), RuntimeEngineError> {
    let bar_count = ctx.timeline_length();
    if bar_count == 0 {
        return Ok(());
    }

    match compiled_tier {
        CompiledProgramTier::Signal(engine) => {
            sweep_signal(ctx, engine, output_matrix, bar_count)?;
        }
        CompiledProgramTier::Allocator(engine) => {
            sweep_allocator(ctx, engine, output_matrix, bar_count)?;
        }
        CompiledProgramTier::Portfolio(engine) => {
            sweep_portfolio(ctx, engine, output_matrix, bar_count)?;
        }
    }
    Ok(())
}

fn sweep_signal(
    ctx: &ExecutionContext,
    engine: &mut SignalExecutionEngine,
    output_matrix: &mut GraphSeriesMatrix,
    bar_count: usize,
) -> Result<(), RuntimeEngineError> {
    let upstream = ctx.signal_upstream.as_slice();
    engine.prepare(upstream, bar_count);
    let column = ctx.signal_output_column;
    let out = output_matrix.signal_column_slice_mut(column);
    let len = bar_count.min(out.len());
    for bar_idx in 0..len {
        out[bar_idx] = engine.execute_at_bar(bar_idx, ctx);
    }
    Ok(())
}

fn sweep_allocator(
    ctx: &ExecutionContext,
    engine: &mut AllocatorExecutionEngine,
    output_matrix: &mut GraphSeriesMatrix,
    bar_count: usize,
) -> Result<(), RuntimeEngineError> {
    resize_leg_buffers(engine, bar_count);
    for leg in 0..engine.leg_weights.len() {
        output_matrix.clear_allocator_leg(leg);
    }
    for bar_idx in 0..bar_count {
        if let Err(err) = engine.allocate_capital_at_bar(bar_idx, ctx, output_matrix) {
            for leg in 0..engine.leg_weights.len() {
                output_matrix.clear_allocator_leg(leg);
            }
            return Err(err);
        }
    }
    Ok(())
}

fn sweep_portfolio(
    ctx: &ExecutionContext,
    engine: &mut PortfolioExecutionEngine,
    output_matrix: &mut GraphSeriesMatrix,
    bar_count: usize,
) -> Result<(), RuntimeEngineError> {
    resize_portfolio_buffers(engine, bar_count);
    output_matrix.clear_portfolio_metrics();
    for bar_idx in 0..bar_count {
        if let Err(err) = engine.track_portfolio_metrics_at_bar(bar_idx, ctx, output_matrix) {
            output_matrix.clear_portfolio_metrics();
            return Err(err);
        }
    }
    Ok(())
}

fn resize_leg_buffers(engine: &mut AllocatorExecutionEngine, bar_count: usize) {
    for buffer in &mut engine.leg_weights {
        if buffer.len() != bar_count {
            buffer.resize(bar_count, 0.0);
        }
    }
}

fn resize_portfolio_buffers(engine: &mut PortfolioExecutionEngine, bar_count: usize) {
    if engine.nav.len() != bar_count {
        engine.nav.resize(bar_count, engine.initial_capital);
    }
    if engine.cash.len() != bar_count {
        engine.cash.resize(bar_count, engine.initial_capital);
    }
    if engine.drawdown.len() != bar_count {
        engine.drawdown.resize(bar_count, 0.0);
    }
    if engine.weight_encodings.len() != bar_count {
        engine.weight_encodings.resize(bar_count, String::new());
    }
}
