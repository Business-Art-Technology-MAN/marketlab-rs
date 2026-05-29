# MarketLab SRD - Phase B Pillar 1 (Part 2): Spatial Time Resolution & Integration
**Target Files:** `crates/pulsar_marketlab/src/trading_stage/mod.rs`, `src/workspace_state.rs`

---

### 1. Objective
Complete the Phase B structural implementation by defining the forward-fill query API for `TimeSampledAttribute` and establishing the coordinate inversion bridge inside `workspace_state.rs` to decouple view components from raw row-index counting.

### 2. Functional Requirements

#### A. Causal Forward-Fill Query Resolution
* **Binary Search Implementation:** Implement `evaluate_at_time(&self, t: f64) -> Option<f32>` inside `trading_stage/mod.rs` using `BTreeMap::range(..=OrderedFloat(t))`.
* **Historical Boundary Scan:** * Return the exact match if an index sits precisely on the coordinate.
  * If no exact match exists, return the nearest historical value immediately preceding $t$ (`next_back()`).
  * Return `None` if $t$ is smaller than the earliest record in the timeline.

#### B. Path Namespace Harmonization
* **Tier Elimination:** Drop the legacy `Base`, `Signals`, and `Overrides` enum matrix wrappers from `TradingStage`.
* **Uniform Hierarchical Paths:** Map all internal parameters to flat, slash-delimited namespaces:
  * Raw market assets match: `"/assets/{ticker}/{attribute}"`
  * Compiled indicators match: `"/analytics/{indicator_id}/{attribute}"`

#### C. Workspace Coordinate Inversion Bridge
* **State Metric Migration:** Update `TradingSystemWorkspace` inside `workspace_state.rs` to track time via `playhead_time: f64`.
* **Dynamic View Assembly:** Refactor `synchronize_inspector_view` or its local pipeline equivalent. Instead of extracting raw cell values from `Array2<f64>` using integer index offsets, update it to pull values by passing `self.playhead_time` directly into `MarketStage::resolve_attribute_at`.