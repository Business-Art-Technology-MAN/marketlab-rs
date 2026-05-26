# MarketLab SRD - UI Interaction & Engine Compilation Alignment
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (UI Loop, Event Ingestion, & Graph Traversals)

---

### 1. Objective
Enable manual user wiring to dynamically drive backend calculations, remove legacy geometric algebra code structures, and implement cached evaluations to stop the Sharpe ratio from jittering.

### 2. Functional Requirements

#### A. Fully Functional Node Spawning & Wiring Controls
* **Canvas Menu Synchronization:** Verify that selecting `Spawn Asset` or `Spawn TA` from the right-click menu appends a fully operational `VisualNode` to the workspace array, complete with matching input/output ports.
* **Portfolio Dynamic Input Snapping:** When a user drags a wire from a Technical Analysis node's output port to a Portfolio node, the connection must snap cleanly into place, generating a new `NodeConnection` entry and updating the graph's revision token (`SharedPipelineGraph.revision`).

#### B. Complete Elimination of Legacy Bivector Signatures
* **Codebase Sweep:** Delete all remaining structural definitions, display strings, and hardcoded logic pathways related to the "Bivector" or "Signal" node types from the layout loop.
* **Direct Signal Routing:** Ensure that the trading bridge logic directly maps the calculated floating-point outputs of your `vector-ta` indicators to the transaction trigger boundaries, completely bypassing any intermediate geometric transformation blocks.

#### C. Cached Playhead Evaluation (Fixing Sharpe Jitter)
* **Evaluation Dirty-Flag:** Introduce an internal caching mechanism for portfolio diagnostics. Introduce a tracking variable, `last_evaluated_state: (usize, usize)`, which stores a combined snapshot of `(playhead_current, graph_revision)`.
* **Conditional Execution:** Inside the 16ms UI polling loop, skip calling `evaluate_portfolio_at_playhead` if the current `(playhead_current, graph_revision)` matches the cached state. Only execute a calculation pass when the user scrubs the playhead, a new tick arrives, or the canvas wiring changes.

#### D. Turn Off the Parallel Synthetic Feeder
* **Single Stream Consolidation:** Comment out or disable `spawn_pipeline_engine_feeder` (the 16ms synthetic engine loop). Force the entire workspace to rely on the sequential CSV playback engine to ensure your analytics panels are free of multi-threaded data conflicts.