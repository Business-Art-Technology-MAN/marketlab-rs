# MarketLab SRD - Phase B Pillar 1: OpenUSD-Inspired Time-Series Stage
**Target File Location:** `crates/pulsar_marketlab/src/trading_stage/mod.rs` (Create or refactor this file)

---

### 1. Objective
Implement a multi-layered, in-memory Market Stage composed of hierarchical paths (Prims) and time-sampled attributes using continuous `f64` timestamps, replacing row-index dependencies with forward-filled spatial lookups.

---

### 2. Functional Requirements

#### A. The Prim & Time-Sampled Attribute Memory Model
* **The Stage Container:** Define a `MarketStage` struct that stores a map of string-based hierarchical paths to primitive definitions:
  ```rust
  pub struct MarketStage {
      pub prims: HashMap<String, MarketPrim>,
  }