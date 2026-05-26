# MarketLab SRD - Phase A Cleanup: Monolithic Module Extraction
**Target Directory:** `crates/pulsar_marketlab/src/`

---

### 1. Objective
Refactor the ~4.8k LOC monolithic `main.rs` file by extracting isolated functional domains into dedicated sub-modules, dramatically reducing file complexity and preparing the codebase for Phase B data structures.

---

### 2. Structural Requirements

Extract code blocks out of `main.rs` and distribute them into the following clean module layout:

#### A. `src/ui/mod.rs` (and sub-components)
* **The Target:** Move all heavy GPUI canvas painting and view rendering logic.
* **The Splits:** Create:
  * `src/ui/node_canvas.rs`: Handles blueprint nodes, rendering port connections, mouse dragging, and context menus.
  * `src/ui/sidebar_inspector.rs`: Hosts `render_spreadsheet_inspector`, parameter input boxes, algorithm selection drop-downs, and layout ledgers.
  * `src/ui/timeline_controls.rs`: Handles the transport controls cluster and playhead index scrubbing widgets.

#### B. `src/graph_compiler.rs`
* **The Target:** Isolate your core graph configuration and sorting engines.
* **The Details:** Move `SharedPipelineGraph`, `NodeConnection`, `compile_graph_to_dag`, and the Kahn topological sort validation tracking loops here.

#### C. `src/workspace_state.rs`
* **The Target:** The central state coordination hub.
* **The Details:** Encompass the core `TradingSystemWorkspace` struct definition along with its frame index boundaries (`playhead_current`, `last_calculated_state`, `graph_revision`), update models, and cross-thread messaging hooks.

---

### 3. Verification & Compliance
* **Zero Behavioral Change:** This is a pure structural refactor. No new logic or mathematical variables may be introduced.
* **Build Check:** Run `cargo check -p pulsar_marketlab` and verify that all 24 library tests execute cleanly across the newly separated files.