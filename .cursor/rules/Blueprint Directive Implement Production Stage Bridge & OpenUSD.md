# Blueprint Directive: Implement Production Stage Bridge & OpenUSD Spike
Target: Create crates/pulsar_marketlab/src/stage_bridge/

1. Implement `ProductionStageProvider` matching the `MarketProviderServices` trait:
   - Connect `sample_timeline` to look up real asset timeline caches from our `MarketStage`.
   - Parse paths like "/assets/SPY/close" and retrieve historical contiguous data slices efficiently.
   - Provide a real routing link for `execute_integrator` to route macro math functions out of the evaluator.

2. Create an isolated spike file `usd_spike.rs`:
   - Import native OpenUSD Rust bindings (`usd` or equivalent crate).
   - Test defining a time-sampled asset schema primitive and profile the retrieval speeds of thousands of attributes across a continuous time axis.
   - Test basic Sdf/Pcp layer composition to see how USD handles overlapping historical data layers.

3. Write an end-to-end integration test (`tests/end_to_end_core_spec.rs`):
   - Load real data into a live Stage, compile an OTL script string into a closure, execute it via the `ProductionStageProvider` at playhead time `t`, and verify that it delivers a mathematically valid terminal vector payload. Ensure thread-safety (Send + Sync) is completely maintained.