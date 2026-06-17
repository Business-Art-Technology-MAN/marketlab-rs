//! Parallel signal-tier sweeps for nodes that share a topological level.

use std::collections::HashMap;

use petgraph::stable_graph::NodeIndex;
use petgraph::stable_graph::StableGraph;
use petgraph::Direction;
use rayon::prelude::*;

use crate::compiler::CompiledProgramTier;
use crate::engine::evaluate_compiled_tier;
use crate::execution_matrix::{ExecutionContext, GraphSeriesMatrix, RuntimeEngineError};
use crate::AssetQuote;
use crate::ExecutionNode;
use crate::OtlObjectKind;

/// Shared read-only inputs for parallel signal jobs (cloned per thread).
#[derive(Clone)]
pub struct ParallelSweepContext {
    pub timeline_len: usize,
    pub initial_capital: f64,
    pub allocation_method: String,
    pub asset_quotes: HashMap<String, AssetQuote>,
}

/// One independent signal-tier node ready for a parallel sweep.
#[derive(Clone)]
pub struct ParallelSignalJob {
    pub node_index: NodeIndex,
    pub prim_path: String,
    pub column: usize,
    pub upstream: Vec<f64>,
    /// OTL expression text for stream attribute naming after the sweep.
    pub expression: String,
}

/// Result of an isolated signal sweep (merge into the shared matrix on the main thread).
pub struct ParallelSignalOutcome {
    pub node_index: NodeIndex,
    pub prim_path: String,
    pub column: usize,
    pub convictions: Vec<f64>,
    pub error: Option<RuntimeEngineError>,
}

/// Group execution indices by longest-path depth from graph sources.
pub fn compute_execution_levels(
    graph: &StableGraph<ExecutionNode, ()>,
    execution_order: &[NodeIndex],
) -> Vec<Vec<NodeIndex>> {
    let mut level_by_node: HashMap<NodeIndex, usize> = HashMap::new();

    for &index in execution_order {
        let preds: Vec<NodeIndex> = graph
            .neighbors_directed(index, Direction::Incoming)
            .collect();
        let level = if preds.is_empty() {
            0
        } else {
            preds
                .iter()
                .filter_map(|pred| level_by_node.get(pred))
                .max()
                .copied()
                .unwrap_or(0)
                + 1
        };
        level_by_node.insert(index, level);
    }

    let max_level = level_by_node.values().copied().max().unwrap_or(0);
    let mut levels = vec![Vec::new(); max_level + 1];
    for &index in execution_order {
        if let Some(level) = level_by_node.get(&index) {
            levels[*level].push(index);
        }
    }
    levels
}

/// Returns true when this node can participate in a same-level parallel signal batch.
pub fn is_parallel_tier_signal(node: &ExecutionNode) -> bool {
    matches!(
        node,
        ExecutionNode::SignalTransform {
            tier_kind: Some(OtlObjectKind::Signal),
            tier_program: Some(_),
            ..
        }
    )
}

/// Run one signal tier sweep into a contiguous output slice (no shared matrix).
pub fn sweep_signal_into_column(
    output: &mut [f64],
    timeline_len: usize,
    column: usize,
    tier: &mut CompiledProgramTier,
    upstream: &[f64],
    ctx: &ParallelSweepContext,
) -> Result<(), RuntimeEngineError> {
    let bar_count = timeline_len.min(output.len());
    if bar_count == 0 {
        return Ok(());
    }

    let mut exec_ctx = ExecutionContext::new(
        timeline_len,
        ctx.initial_capital,
        ctx.allocation_method.clone(),
        ctx.asset_quotes.clone(),
        HashMap::new(),
    );
    exec_ctx.signal_upstream = upstream.to_vec();
    exec_ctx.signal_output_column = column;

    let mut matrix = GraphSeriesMatrix::with_capacity(timeline_len, column + 1, 1);
    evaluate_compiled_tier(&mut exec_ctx, tier, &mut matrix)?;
    output[..bar_count].copy_from_slice(&matrix.signal_column_slice(column)[..bar_count]);
    Ok(())
}

/// Sweep disjoint signal columns in parallel; `tiers` must align 1:1 with `jobs` (forked engines).
pub fn run_parallel_signal_batch(
    jobs: &[ParallelSignalJob],
    tiers: &mut [CompiledProgramTier],
    ctx: &ParallelSweepContext,
) -> Vec<ParallelSignalOutcome> {
    debug_assert_eq!(jobs.len(), tiers.len());

    if jobs.is_empty() {
        return Vec::new();
    }
    if jobs.len() == 1 {
        let job = &jobs[0];
        let mut convictions = vec![0.0; ctx.timeline_len];
        let error = sweep_signal_into_column(
            &mut convictions,
            ctx.timeline_len,
            job.column,
            &mut tiers[0],
            &job.upstream,
            ctx,
        )
        .err();
        return vec![ParallelSignalOutcome {
            node_index: job.node_index,
            prim_path: job.prim_path.clone(),
            column: job.column,
            convictions,
            error,
        }];
    }

    jobs.par_iter()
        .zip(tiers.par_iter_mut())
        .map(|(job, tier)| {
            let mut convictions = vec![0.0; ctx.timeline_len];
            let error = sweep_signal_into_column(
                &mut convictions,
                ctx.timeline_len,
                job.column,
                tier,
                &job.upstream,
                ctx,
            )
            .err();
            ParallelSignalOutcome {
                node_index: job.node_index,
                prim_path: job.prim_path.clone(),
                column: job.column,
                convictions,
                error,
            }
        })
        .collect()
}

pub fn merge_parallel_signal_outcomes(
    matrix: &mut GraphSeriesMatrix,
    outcomes: &[ParallelSignalOutcome],
) {
    for outcome in outcomes {
        if outcome.error.is_some() {
            matrix.clear_signal_column(outcome.column);
            continue;
        }
        matrix.copy_signal_column_from_slice(outcome.column, &outcome.convictions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile_object_tier;
    use crate::frontend::compile_object_program;
    use crate::ObjectCodegenRegistry;

    #[test]
    fn execution_levels_group_independent_signals() {
        use crate::{GraphCompileSpec, GraphCompileWire, MarketLabGraphEngine};

        let spec = GraphCompileSpec {
            nodes: vec![
                (
                    "/assets/A".into(),
                    ExecutionNode::DataInput {
                        symbol: "A".into(),
                    },
                ),
                (
                    "/assets/B".into(),
                    ExecutionNode::DataInput {
                        symbol: "B".into(),
                    },
                ),
                (
                    "/sig/a".into(),
                    ExecutionNode::SignalTransform {
                        expression: String::new(),
                        compiled_fn: None,
                        compiled_multi: None,
                        tier_kind: None,
                        tier_program: None,
                    },
                ),
                (
                    "/sig/b".into(),
                    ExecutionNode::SignalTransform {
                        expression: String::new(),
                        compiled_fn: None,
                        compiled_multi: None,
                        tier_kind: None,
                        tier_program: None,
                    },
                ),
            ],
            wires: vec![
                GraphCompileWire {
                    source_prim_path: "/assets/A".into(),
                    target_prim_path: "/sig/a".into(),
                    relationship: "inputs:underlying".into(),
                },
                GraphCompileWire {
                    source_prim_path: "/assets/B".into(),
                    target_prim_path: "/sig/b".into(),
                    relationship: "inputs:underlying".into(),
                },
            ],
        };
        let engine = MarketLabGraphEngine::compile(spec).expect("compile");
        let levels = compute_execution_levels(engine.graph(), engine.execution_order());
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].len(), 2);
        assert_eq!(levels[1].len(), 2);
    }

    #[test]
    fn parallel_batch_distinguishes_distinct_upstream_series() {
        let source = r#"
signal pass(input closure raw, output closure gated) {
    gated = data;
}
"#;
        let program = compile_object_program(source).expect("parse");
        let registry = ObjectCodegenRegistry::default();
        let template =
            compile_object_tier(&program, &registry).expect("compile signal tier template");
        let ctx = ParallelSweepContext {
            timeline_len: 15,
            initial_capital: 1_000_000.0,
            allocation_method: "Allocation::EqualWeight".into(),
            asset_quotes: HashMap::new(),
        };
        let upstream_a: Vec<f64> = (0..15).map(|bar| bar as f64).collect();
        let upstream_b: Vec<f64> = (0..15).map(|bar| 100.0 + bar as f64).collect();
        let jobs = vec![
            ParallelSignalJob {
                node_index: NodeIndex::new(0),
                prim_path: "/sig/a".into(),
                column: 0,
                upstream: upstream_a,
                expression: String::new(),
            },
            ParallelSignalJob {
                node_index: NodeIndex::new(1),
                prim_path: "/sig/b".into(),
                column: 1,
                upstream: upstream_b,
                expression: String::new(),
            },
        ];
        let mut tiers = vec![template.fork_for_sweep(), template.fork_for_sweep()];
        let outcomes = run_parallel_signal_batch(&jobs, &mut tiers, &ctx);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|outcome| outcome.error.is_none()));
        assert!(
            outcomes[0].convictions.iter().zip(&outcomes[1].convictions).any(
                |(left, right)| (left - right).abs() > 1e-9
            ),
            "parallel jobs must write different columns from distinct upstream"
        );
    }

    #[test]
    fn parallel_batch_produces_same_length_as_timeline() {
        let source = r#"
signal gate(input closure raw, output closure gated) {
    gated = sma(data, 5);
}
"#;
        let program = compile_object_program(source).expect("parse");
        let template =
            compile_object_tier(&program, &ObjectCodegenRegistry::default()).expect("compile");
        let ctx = ParallelSweepContext {
            timeline_len: 10,
            initial_capital: 1_000_000.0,
            allocation_method: "Allocation::EqualWeight".into(),
            asset_quotes: HashMap::new(),
        };
        let upstream: Vec<f64> = (0..10).map(|bar| bar as f64 + 1.0).collect();
        let jobs = vec![ParallelSignalJob {
            node_index: NodeIndex::new(0),
            prim_path: "/sig/a".into(),
            column: 0,
            upstream: upstream.clone(),
            expression: "sma(data,5)".into(),
        }];
        let mut tiers = vec![template.fork_for_sweep()];
        let outcomes = run_parallel_signal_batch(&jobs, &mut tiers, &ctx);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_none());
        assert_eq!(outcomes[0].convictions.len(), 10);
    }
}
