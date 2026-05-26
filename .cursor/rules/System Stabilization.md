# MarketLab SRD - System Stabilization & Interaction Fixes
**Target File Location:** `crates/pulsar_marketlab/src/main.rs`

---

### 1. Objective
Clean up the visual workspace by removing the vestigial Bivector node, fixing the canvas context menu spawning routines, and stopping the Sharpe Ratio calculation drift.

### 2. Functional Requirements

#### A. Strip Out Vestigial Bivector Logic
* Remove the "Bivector" / "Signal" node variant from the default startup pipeline creation logic (`default_pipeline_nodes`).
* Rewire the default start state to map directly: **Node 1 (Asset: SPY) ──► Node 2 (TA: RSI) ──► Node 4 (Sim Portfolio)**.

#### B. Fix Canvas Spawning and Wire Interaction
* Ensure the right-click popover menu explicitly features three clean operational buttons: `Spawn Asset Node`, `Spawn TA Node`, and `Spawn Portfolio Node`.
* Update the node connection validation loop (`commit_wire_to_input`). Ensure a user can manually drag a wire from any TA node's output socket and snap it cleanly into the Portfolio node's input socket.

#### C. Fix Sharpe Ratio Drift (Stop/Reset CSV Playback)
* Locate the background thread `spawn_csv_asset_feeder`. 
* When the CSV file reader encounters End-of-File (EOF), **do not blindly loop without resetting state**. 
* Add an explicit cache clearance message (`PipelineSystemMessage::ResetSimulation`) or completely drain the `portfolio_diagnostics` tracking buffers before restarting the file read, ensuring your Sharpe Ratio and Total Return calculations reset back to absolute baseline variables instead of compounding infinitely.