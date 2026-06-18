# MarketLab Technical Specification: OTL Script Editor Workspace Tab

## 1. Purpose & User Workflow
This document specifies the implementation of a first-class, dedicated Open Trading Language (OTL) Script Editor Tab (`WorkspaceTab::OtlEditor`) inside the `pulsar_marketlab_ui` workstation shell. 

Mirroring Blender's split-workspace paradigm—where a text editor panel seamlessly syncs with selected Open Shading Language (OSL) shader nodes—this pane provides an integrated environment for script authoring, compilation, and dynamic node port synchronization. 

### Core Workflow Interface Map
* **Left Canvas Region:** User navigates the node canvas panel and selects a target `OtlShader` node. The workspace tracks the selected prim path.
* **Right Editor Region:** The OTL Script Editor Tab (`WorkspaceTab::OtlEditor`) reads and surfaces the source code code block.
* **Interactive Evaluation:** Modifying the code text and clicking the global **[ Compile Script ]** action updates the active OpenUSD layer stack.

---

## 2. Hard Architectural Invariants
1. **Split-Plane Integrity:** The code buffer inside the editor panel must never bypass the OpenUSD layer stack. When a script is compiled, the text string must write directly down to the selected prim's `inputs:script_src` attribute via `WorkspaceContext::modify_attribute` on the background thread using a deferred write loop.
2. **Asynchronous Non-Blocking Compilation:** Script tokenizer, parser, and abstract syntax tree (AST) construction passes must never execute on the main GPUI render tick. All compilation, code validation, and timeline graph sweeps must be offloaded via `cx.background_executor().spawn()`.
3. **Reactive Selection Lifecycle:** The script editor tab must be strictly context-aware. It actively binds to `WorkspaceContext.selected_path` via `install_ui_selection_observer`. It must gracefully handle empty selections, node switching, and changes to structural nodes that do not possess script attributes.

---

## 3. Subsystem Implementation Requirements

### Task 1: The OTL Editor Pane Layout (`otl_editor_pane.rs`)
Create a new workspace pane component handling code presentation and input interaction.
* **Target File:** `crates/pulsar_marketlab_ui/src/workspace/otl_editor_pane.rs`
* **Interface Mechanics:**
  - Add `WorkspaceTab::OtlEditor` to the primary layout tab switcher array.
  - Implement a monospace code canvas complete with dynamic line numbering.
  - Expose a clean command header featuring a **[ Compile Script ]** action button, paired with a global keyboard shortcut hook (`Cmd+B` / `Ctrl+B`).
  - Implement a persistent compilation status console area at the footer of the editor panel. If `compiler::compile_script()` fails, print syntax line markers, warnings, or lexer evaluation errors. On a successful build pass, render a clean `[ OK: Compiled Series Closure ]` benchmark report.

### Task 2: Context Selection Binding & USD Synchronization Hook
Connect the active script text buffer to the open project stage architecture.
* **Target Files:** `crates/pulsar_marketlab/src/workspace_state.rs` and `crates/pulsar_marketlab_ui/src/workspace/context.rs`
* **Sync Lifecycle:**
  - When a user selects a node on the canvas, check if its type matches `NodeType::OtlShader` or points to an underlying `OtlOperator` prim on the stage.
  - If true, pull the current string state out of `inputs:script_src` to cleanly populate the editor pane's text buffer. If false or unselected, present a clear blank placeholder screen reading: *"Select an OtlShader Node on the Canvas to view or edit its underlying Open Trading Language source."*
  - When the compilation pipeline runs, wrap the buffer contents and commit it using `cx.defer(publish_canvas_to_usd_stage)`. This steps through the layer stack, increments `engine_cache_generation`, and triggers the unified background compilation loops.

### Task 3: Dynamic Node Port Re-Synthesis
Unlike the immutable port layouts used by fixed `TaUberSignal` archetypes, custom `OtlShader` nodes must dynamically restructure their visible graph sockets based on parameter mutations inside the code string.
* **Target Files:** `crates/pulsar_marketlab/src/graph_compiler/registry.rs` and `crates/pulsar_marketlab_core/src/orchestration/compiler.rs`
* **Interface Mechanics:**
  - When `compiler.rs` normalizes and parses the written script, expose a shallow scanning block (a script signature parser) that returns a vector of input parameter identifiers and explicit named outputs.
  - For example, parsing `fn main(source, lookback, threshold)` tells the system to instantly construct exactly three incoming input ports on the visual card (`source`, `lookback`, `threshold`).
  - Update `graph_compiler/registry.rs`. If a user modifies their code parameters and explicitly deletes an active input or output variable, the system must immediately capture that structural alteration, invalidate and prune any stale, broken edge wiring paths, and log the topological correction inside `wiring_errors` without crashing the active render grid.

---

## 4. Verification Checklist
- [ ] Selecting an `OtlShader` node immediately populates the text workspace panel with its active `inputs:script_src` text line.
- [ ] Pressing `Cmd+B` / `Ctrl+B` initiates a background non-blocking execution pass and updates the compile console footer.
- [ ] Changing function parameters in the script dynamically creates/removes corresponding sockets on the canvas node frame interface.
- [ ] Broken connections caused by code refactoring are safely cleaned up and flagged in the validation panel without engine panic.
- [ ] Running the total test sweep via `cargo test` confirms 100% green stability across all 115 core workspace engine validations.