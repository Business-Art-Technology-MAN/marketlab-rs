# OTL Tier Execution Hardening SRD (v8.0)

**Status:** Follow-on — partial wiring in core engine

## Current state

- `evaluate_compiled_tier` runs via `TimelineTierWorkspace.run_signal_node` during vector sweeps
- `MarketLabGraphEngine::compile_from_canvas` is the interactive compile entry (canvas-native IR)
- Cached `MarketLabGraphEngine` reused when `engine_cache_generation` is stable

## Follow-on work

1. Remove silent zero-fill fallbacks in signal tier error paths (`engine.rs` tier sweep)
2. Route legacy portfolio integration paths through tier allocator engine exclusively
3. Add parallel vs sequential tier sweep equivalence test (`pulsar_marketlab_core` benches)
4. Surface compile/runtime tier errors in workstation status bar (no silent `vec![0.0; n]`)

## Verification target

`cargo test -p pulsar_marketlab_core tier_sweep` + graph engine integration tests pass with identical wealth series under Rayon and sequential modes.
