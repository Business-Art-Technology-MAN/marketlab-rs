# System Requirements Document (SRD)
## Dopesheet Viewport Clipping & USD Namespace Path Generation

**Document Version:** 6.0.0  
**Target Environment:** `crates/pulsar_marketlab_ui` & `crates/pulsar_marketlab`  
**Context:** UI Layout Optimization & Hierarchy Readability Deficit  

---

## 1. Core Objectives
1. Eliminate UI lag by implementing strict horizontal layout clipping on the dopesheet time-series grid view.
2. Replace generic, anonymous USD path identifiers (`node_0000*`) with structured, human-readable namespaces derived from node metadata and asset identities.

---

## 2. Modification Specifications

### 2.1 Enforce Lazy Boundary Clipping on the Timeline Grid View
**File:** `crates/pulsar_marketlab_ui/src/dopesheet/` (or equivalent layout module)
* **The Problem:** The UI layer is evaluating layout boundaries or allocating visual elements for all 2,872 frames simultaneously, locking the main event thread during canvas mouse drags.
* **The Fix:** Implement strict virtual scrolling/clipping. Calculate the visible frame window based on the current timeline scale and horizontal scroll offset. **Only** allocate, layout, and render grid items that fall within the actively visible screen boundaries. Discard or skip off-screen elements immediately.

### 2.2 Semantic USD Namespace Path Compilation
**File:** `crates/pulsar_marketlab/src/scene/canvas_compose.rs` (or your USD abstraction layer)
* **The Problem:** Nodes are serialized using abstract incremental strings (`node_000002`), making the Context Tower inspector unreadable.
* **The Fix:** Modify the USD primitive naming engine. When compiling the scene graph, sanitize the node's visual title and type identifier to construct meaningful string tokens:
  * *Example:* A generic `node_000002` calculating a cross over `GLD.csv` must resolve cleanly to:
    `/MarketLab/Universe/GLD_Cross_Signal`
  * If duplicate names exist within the same topological layer, append a simple short index: `GLD_Cross_Signal_01`.
* Update the ledger rendering component inside `SidebarInspector` to pull this human-readable path string directly for display.

---

## 3. Verification Metrics
* **UI Snappiness:** Moving a node card or panning across the workspace canvas must maintain a locked 120 FPS paint cycle with the 2,872-frame timeline open.
* **USD Clarity:** The Context Tower's Integrator Ledger must display explicit paths (e.g., `GLD_Cross_Signal`, `SPY_Crossover`) rather than raw anonymous increments.
