# MarketLab SRD - Phase B Pillar 2: Asynchronous FIX Ingestion Bridge
**Target Directory:** `crates/pulsar_marketlab/src/fix_engine/` (Create this module)
**Target Files:** `src/fix_engine/mod.rs` (New), `src/workspace_state.rs` (Integration)

---

### 1. Objective
Transition the global stage time axis from arbitrary daily row ordinals to absolute Unix microsecond timestamps (`f64`). Implement an asynchronous mock FIX (Financial Information eXchange) protocol engine thread that injects dense sub-second execution messages between coarse historical bar marks, testing the multi-frequency robustness of the continuous `MarketStage`.

---

### 2. Functional Requirements

#### A. Absolute Unix Microsecond Timestamp Migration
* **Temporal Axis Refactor:** Replace the daily bar integer-to-float ordinal mapper (`stage_time_from_bar_date`) with a real time-parsing function.
* **Epoch Scale Alignment:** Convert Yahoo daily bar strings to standard Unix epoch mid-day or closing timestamps in floating-point seconds (e.g., `1716724800.0`). 
* **Granularity Protection:** All attribute lookups via `evaluate_at_time(t)` must now operate over this expanded epoch scale, ensuring $O(\log N)$ binary search performance is preserved over large epoch distances.

#### B. The Asynchronous Mock FIX Feeder Thread
* **The Connection Loop:** Inside `src/fix_engine/mod.rs`, implement an asynchronous mock engine block (`spawn_mock_fix_bridge`) that simulates a live matching engine feed line.
* **Dense Sub-Second Injection:** The thread must emit trade alerts at microsecond offsets relative to the active historical simulation playhead:
  $$\text{FIX Timestamp} = \text{Current Bar Epoch} + \text{Microsecond Offset (e.g., } 0.000452\text{)}$$
* **FIX Ingestion Target:** Route these dense executions directly to a dedicated transactional staging path: `"/execution/fix/ticks"`. Attributes must capture `"last_price"` and `"last_qty"`.

#### C. Multi-Frequency Stride Realignment (`MixedFrequencyStrideGrid`)
* **Strided Horizon Mapping:** Refactor the remaining index-bound tracker, `MixedFrequencyStrideGrid`. 
* **Dynamic Time Windows:** Instead of jumping across hardcoded row counts, update the stride calculator to accept lookback time windows expressed as durations (e.g., `86400.0` seconds for a rolling 24-hour analytical window). 
* **Causal Window Slicing:** When calculating indicators across mixed frequencies, the stride engine must query the `MarketStage` by using a time-slice range from `(playhead_time - stride_duration) ..= playhead_time`.

#### D. Live Execution UI Mirroring Harness
* **The Inter-Stage Bridge:** Resolve the separation between the execution loop's `simulation_stage` and the main UI's `market_stage`.
* **State Sync Flushes:** Every time `apply_transaction` commits a double-entry ledger mutation (`/execution/portfolio/...`) to the simulation thread stage, push a corresponding `TickUpdate` containing the absolute path, timestamp, and new balance value across the UI message bus to hydrate the frontend inspector instantly.