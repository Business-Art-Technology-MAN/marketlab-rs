# MarketLab SRD - Phase B Pillar 1: OpenUSD-Inspired Time-Series Stage
**Target Directory:** `crates/pulsar_marketlab/src/trading_stage/`
**Target Files:** `src/trading_stage/mod.rs` (Refactor), `src/workspace_state.rs` (Integration)

---

### 1. Objective
Refactor the core data layer from a rigid, row-indexed dense matrix into an in-memory, hierarchical scene graph (`MarketStage`) composed of paths ("Prims") and time-sampled attributes using continuous `f64` timestamps. This foundational layer decouples evaluation from explicit tick intervals, enabling asynchronous multi-asset alignment and strict look-ahead bias protection.

---

### 2. Functional Requirements

#### A. The Hierarchical Scene Graph & Prim Memory Model
* **The Stage Root (`MarketStage`):** Maintain a thread-safe, centralized stage container that maps unique hierarchical string paths (e.g., `"/assets/SPY"`, `"/analytics/RSI_0"`) to individual Prims.
* **The Primitive Component (`MarketPrim`):** Define a primitive structure that contains an internal dictionary of named properties/attributes (e.g., `"close"`, `"volume"`, `"signal"`).
* **The Time-Sampled Vector (`TimeSampledAttribute`):** Attributes must store their underlying values inside a sorted map (`std::collections::BTreeMap`) where keys are `ordered_float::OrderedFloat<f64>` and values are `f32`. This ensures native sorting of arbitrary, irregular microsecond timestamps alongside coarse daily intervals.

```rust
// Architectural Layout Layout
pub struct MarketStage {
    pub prims: HashMap<String, MarketPrim>,
}

pub struct MarketPrim {
    pub attributes: HashMap<String, TimeSampledAttribute>,
}

pub struct TimeSampledAttribute {
    pub samples: BTreeMap<ordered_float::OrderedFloat<f64>, f32>,
}