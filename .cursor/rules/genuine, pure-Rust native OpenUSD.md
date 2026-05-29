# Role & Context
You are an expert Rust systems architect working on MarketLab. We are moving away from our primitive in-memory mock composition layers to use the genuine, pure-Rust native OpenUSD implementation from the `mxpv/openusd` crate (version 0.3.0).

Our core architectural pattern is a Split-Plane Hybrid Engine:
1. Structural Plane (Managed via openusd::Stage): Handles layer stacking, user variant picks, and structural path resolution.
2. Vector Temporal Plane (Managed via MarketStage): Handles high-frequency time-series array sweeps, bypassing OpenUSD's current lack of value-resolution time-sampling.

# Target Files
- Update: `Cargo.toml` (Add `openusd = "0.3.0"`)
- Refactor/Replace: `crates/pulsar_marketlab/src/stage_bridge/production_provider.rs`
- Refactor/Replace: `crates/pulsar_marketlab/src/stage_bridge/usd_spike.rs` (Transform into a wrapper/utility module for mxpv/openusd)

# Technical Requirements

1. Update ProductionStageProvider to hold an actual openusd::Stage:
   - Replace the old custom structs with:
     ```rust
     pub struct ProductionStageProvider {
         usd_stage: std::sync::Arc<openusd::Stage>,
         temporal_stage: std::sync::Arc<MarketStage>,
         active_path: String,
     }
     ```

2. Implement Strong-to-Weak LIVRPS Layer Composition Rules inside sample_timeline:
   - When `sample_timeline(path, start, end)` is called with a path like `"/assets/SPY/close"`:
     a) Parse or strip the asset path down to the USD Prim path format (e.g., `"/assets/SPY"`).
     b) Use the native openusd stage field API to query whether that specific Prim is active in the composed layer hierarchy:
        `let active: Option<bool> = self.usd_stage.field(&prim_path, openusd::sdf::FieldKey::Active).unwrap_or(Some(true));`
     c) If active is false (meaning an upstream `.usda` user overlay deactivated the asset), return an empty `vec![]`.
     d) If active is true, fall through to the Vector Temporal Plane and pull the high-frequency array slices out of `MarketStage::samples_in_time_range`.

3. Implement Environment Variable Lookups via openusd::Stage:
   - Inside `get_global_attribute(name, t)`, if the name targets metadata or a session configuration (e.g., matching `"global::"` prefixes), query the field properties straight from the composed OpenUSD stage structure using generic typed field lookups: `self.usd_stage.field::<f32>(&self.active_path, openusd::sdf::FieldKey::from(name))`.
   - Fall back to the internal `temporal_stage` execution ledger if it's a dynamic operational variable like account cash balances or margins.

4. Keep End-to-End Test Harness Compiling and Passing:
   - Update `tests/end_to_end_core_spec.rs` to construct an actual native `openusd::Stage` (e.g., using a small, valid inline or mock `.usda` asset definition text string loaded via `openusd::Stage::open` or the `Stage::builder()`).
   - Wire this real OpenUSD stage right into the constructor for `ProductionStageProvider`.
   - Ensure all 57 tests continue passing flawlessly, and verify that the provider, closures, and the nested `openusd::Stage` maintain strict thread-safety boundaries (`Send + Sync`) to allow parallel multi-core execution loops.