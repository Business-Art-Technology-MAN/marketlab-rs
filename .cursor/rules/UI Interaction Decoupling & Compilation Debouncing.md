# System Requirements Document (SRD)
## UI Interaction Decoupling & Compilation Debouncing

**Document Version:** 5.0.0  
**Target Environment:** `crates/pulsar_marketlab_ui` & `crates/pulsar_marketlab`  
**Context:** Main-Thread Unblocking & Responsiveness Sprint  

---

## 1. Core Objective
Eliminate GUI unresponsiveness and main-thread thread locks during node manipulation, canvas navigation, and parameter adjustments. The interface must maintain a consistent 120 FPS visual paint cycle by completely isolating continuous mouse/slider drag events from synchronous OTL compilations and OpenUSD stage writes.

---

## 2. Modification Specifications

### 2.1 Implement Debounced Execution on Parameter Changes
Modify parameter slider actions and node property fields inside the workspace view layer. 
* **Continuous Actions:** Dragging a slider or inputting text must update the local view state property *immediately* for responsive rendering, but must **not** trigger `execute_timeline()` or stage serialization.
* **Debounce Window:** Implement a `500ms` asynchronous timer window. Only execute the full OTL compilation engine sweep and update the `GraphSeriesMatrix` when the user has stopped modifying properties for a full 500ms.

### 2.2 Asynchronous Background Compilation
Move `compile_object_tier()` and the core timeline execution loop entirely off the main GPUI event thread.
* Spawn compilation tasks using an asynchronous background thread worker or runtime task pool (`cx.background_executor().spawn(...)`).
* While the compilation runs in the background, display a passive, low-overhead visual loading spinner or loading state on the affected canvas cards, leaving the main canvas layout fluid and interactive.

### 2.3 Optimize Canvas Paint Loops
Verify that **zero math evaluations, matrix slicing, or type lookups** occur inside the `render()` implementation of `CanvasNodeCard` and `SidebarInspector`. All components must lazily read pre-cached scalar values from the `MetricsTelemetryBridge`. If the bridge is dirty or uninitialized, fallback instantly to a default zeroed state without blocking the layout tick.

---

## 3. Verification Metrics
* Dragging structural layout nodes across the canvas must run smoothly without jumping, clipping, or stuttering.
* Changing an OTL parameter must show immediate slider movement, with the 15-second compilation running cleanly in the background without freezing the UI shell window.
