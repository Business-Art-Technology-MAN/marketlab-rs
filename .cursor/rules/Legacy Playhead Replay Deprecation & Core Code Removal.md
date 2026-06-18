# System Requirements Document (SRD)
## Legacy Playhead Replay Deprecation & Core Code Removal

**Document Version:** 3.0.0  
**Target Environment:** `marketlab-rs` / `crates/pulsar_marketlab_core` & `crates/pulsar_marketlab`  
**Context:** Structural execution cleanup for Cursor coding agent  

---

## 1. Executive Summary & Core Intent

This document specifies the technical requirements to completely deprecate, prune, and remove the legacy asynchronous step-replay engine path (`evaluate_portfolio_at_playhead`) and its accompanying structural bridge utilities (`TaExecutionBridge`). 

With the ahead-of-time **Time-Series Vectorization** framework and the lazy-slicing `sync_view_window()` cache layer fully operational, maintaining the legacy iterative simulation fallback path introduces code bloat, technical debt, and architectural confusion. This sprint enforces a "burn the ships" strategy to ensure the application relies exclusively on high-performance columnar matrix tracking.

---

## 2. Targeted Components for Deletion

The coding agent must trace and completely remove the following definitions, dependencies, and imports across the workspace:
[ Legacy Async Step Paths ]   -->   [ TaExecutionBridge ]   -->   [ Out-of-Band Fallbacks ](DELETE)                          (DELETE)                       (DELETE)
1. **`evaluate_portfolio_at_playhead`:** Remove the asynchronous, step-based execution loop entirely.
2. **`TaExecutionBridge` & `TaExecutionFrame`:** Prune the structural types previously used to pass individual out-of-band bar frames to the UI context.
3. **Conditional Fallback Blocks:** Strip out early-return checks or match arms within layout composition pipelines that fall back to iterative stepping when a vectorized cache is missing.

---

## 3. Reference Files & Code Cleanups

### 3.1 Playhead Evaluation Routing
Modify `crates/pulsar_marketlab/src/orchestration/playhead.rs` to strip out asynchronous step generation:

```rust
// REMOVE THIS ENTIRE FUNCTION PATH AND ITS UTILITIES:
// pub async fn evaluate_portfolio_at_playhead(...) -> Result<TaExecutionFrame, BridgeError> { ... }

// ENFORCE STRICT EXCLUSIVITY ON VECTORIZED LOOKUPS:
pub fn spawn_playhead_evaluation_async(cx: &mut AppContext, node_id: usize) {
    // BEFORE: Contained an if-let fallback checking if the vectorized stream was cold, 
    // spinning up evaluate_portfolio_at_playhead on a worker pool.
    
    // AFTER: Assert that the vectorized cache matrix must be present.
    // If warm, trigger direct lookup slice immediately.
    self.sync_view_window(cx);
}
3.2 Canvas Card Layout CleanupsModify crates/pulsar_marketlab_ui/src/canvas/node_card.rs:Rust// Strip out all legacy match evaluation branches. 
// Canvas cards must read from the vectorized series matrix or telemetry bridge exclusively.

// REMOVE:
// let fallback_data = cx.global::<LegacyPlayheadCache>().get(&self.node_id);

// ENFORCE:
let matrix = cx.global::<GraphSeriesMatrix>();
let view = cx.global::<WorkspaceViewWindow>();
// Direct index slicing only...
3.3 Sidebar Inspector UI StripsModify crates/pulsar_marketlab_ui/src/panels/sidebar_inspector.rs:Rust// Remove any "Legacy Playhead Replay Active" indicator sub-elements, 
// banners, or structural fallback states from the rendering tree.
// The "Live GraphEngine" telemetry strip becomes the permanent, unified view layout.
4. Execution Flow Refactor RulesPanic on Missing Stream Cache: If sync_view_window() is invoked and the background GraphSeriesMatrix has not yet evaluated the USD stage, it should cleanly report a blank state ($0.0$ values) or display an explicit initialization loading state on the canvas card metrics, rather than switching to an iterative fallback calculation block.Compile-Time Enforcement: Remove any #[allow(dead_code)] attributes masking legacy evaluation structures. Let the compiler point directly to unused step-by-step methods, then prune them out cleanly.Maintain Test Suite Validity: All 31 bin tests and 63 core engine tests must pass after stripping out the execution bridge modules.5. Verification CheckpointsConfirm that cargo build completes cleanly with zero reference errors or unresolved import paths linking to TaExecutionBridge.Manually verify UI performance: boot the app using cargo run -p pulsar_marketlab. Drag the time-window slider across an active portfolio canvas graph—ensure metrics resolve entirely via index offset caching with zero background asynchronous worker pool allocations.
