# OTL Parallel Execution Level Contract

**Target:** `crates/pulsar_marketlab_core` — `MarketLabGraphEngine`, `engine/parallel_sweep.rs`, `execution_matrix.rs`

## Purpose

Signal-tier OTL nodes that share the same topological depth have **no dependency on each other**. The engine may sweep them with Rayon before allocator/portfolio tiers run sequentially.

## Level computation

- `compute_execution_levels(graph, execution_order)` assigns each node `level = max(pred levels) + 1` (sources at level 0).
- `MarketLabGraphEngine::execution_levels()` mirrors this at compile time and is stable for the life of the compiled graph.

## Execution order within one timeline run

For each `level` in `execution_levels`:

1. **Sequential pass** — every node that is *not* part of a parallel signal batch runs via `execute_timeline_node` (e.g. `DataInput`, legacy closures, single tier signal, allocator, portfolio).
2. **Parallel pass** — if the level contains **≥ 2** nodes matching `is_parallel_tier_signal` (signal OTL object + `tier_program` + entry in `tier_sweep_cache`), those nodes are **skipped** in step 1 and executed together via `run_parallel_signal_batch`.

Upstream nodes on **lower levels** must complete before a parallel batch runs so `node_outputs` holds the correct price/signal series per edge.

## Tier compile cache (no per-sweep `compile_object_tier`)

- `compile_otl_scripts()` fills `tier_sweep_cache: HashMap<NodeIndex, CompiledProgramTier>` using predicted signal column indices from graph topology.
- `execute_timeline(&mut self, …)` reuses cached tiers; parallel jobs call `CompiledProgramTier::fork_for_sweep()` so each Rayon task owns mutable sweep state without re-parsing OTL.
- Call **`compile_otl_scripts()`** (or `compile_from_stage`, which invokes it) before the first `execute_timeline`.

## Matrix layout

- `GraphSeriesMatrix` stores signals and allocator legs in column-major `ColumnMajorBlock` buffers.
- Hot paths must use `signal_column_slice` / `allocator_leg_slice` — avoid `signal_series()` clones.
- Export IPC via `column_primitive_array()`, `signals_flatten_primitive_array()`, or `ColumnMajorBlock::shared_buffer()`.

## Parallel job contract

```rust
ParallelSignalJob {
    node_index,
    prim_path,
    column,      // reserved in TimelineTierWorkspace before batch
    upstream,    // padded upstream scalar series for this node only
    expression,  // stream attribute naming
}
// Pair 1:1 with forked CompiledProgramTier scratch engines in run_parallel_signal_batch.
```

Outcomes merge on the main thread through `merge_parallel_signal_outcomes`; failed jobs clear their reserved column.

## Tests & benchmarks

- `parallel_waves_run_two_tier_signals_at_same_level` — two DataInputs + two tier signals, distinct upstream.
- `cargo bench -p pulsar_marketlab_core --bench tier_sweep` — cached vs per-sweep compile, Rayon batch.

## Do not

- Run parallel signal sweeps before upstream `DataInput` nodes on the same timeline have written `node_outputs`.
- Share one `CompiledProgramTier` mutably across Rayon workers (always `fork_for_sweep` per job).
- Assume per-bar conviction sums differ when alpha scaling saturates both legs (compare per-bar samples or use pass-through OTL in tests).
