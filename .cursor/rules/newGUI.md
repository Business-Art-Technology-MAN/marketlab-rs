# Role & Context
You are an expert systems developer specializing in Rust, high-performance desktop graphics applications, and the GPUI framework (0.2.2). You are modifying `MarketLab`, a quantitative visual pipeline workbench using an OpenUSD structural backbone and an immediate-mode GPUI layout engine.

# Objective
Re-skin and synchronize the structural layout views to conform to a professional Digital Content Creation (DCC) application (e.g., Blender/Houdini) rather than a high-contrast Bloomberg financial terminal. Eliminate layout redundancies, ensure deep synchronization between the hierarchy tree and the node canvas, and apply a muted dark chrome palette.

## Core Directives

### 1. Unified Selection State Mapping
* **File Target:** `crates/pulsar_marketlab_ui/src/workspace/context.rs`
* **Mechanism:** All user interactions (clicking a row in the Hierarchy Tree-Table or clicking a node on the Node Canvas) must call a single unified mutation function: `WorkspaceContext::set_selected_path(path: Option<String>, cx: &mut ViewContext<Self>)`.
* **Reactivity:** This function must step a unique `ui_selection_generation: u64` sequence flag and trigger `cx.notify()`.
* **Observation:** Install explicit observers in both `NodeCanvasPane` and `StageComposerPane` tracking this generation flag. When it changes, matching elements must instantly apply the focused background highlights and auto-scroll to view limits.

### 2. Node Canvas Shell Transformation (`node_canvas.rs`)
* **Palette Realignment:** Rewrite the painting loops to use the DCC Palette: Canvas Backplate (`#1b1b1f`), Node Hull (`#2d2d32`), Selected Node (`#3b3b42`), Active Header (`#3d3d44`), and Primary Text (`#e5e5ea`).
* **Geometry Styling:** Update node containers to use `rounded_md()` (6px-8px corners) and subtle dark borders (`#121214`) to create geometric anti-aliasing depth on screen.
* **Blender Capsule (Pill) Mode:** Read a `node.collapsed` boolean flag from the local visual state view. 
  * If `true`, bypass the body panel rendering entirely. Render a sleek horizontal capsule container using `rounded_full()`, height constraint `h_7()`, and width `w_180`. 
  * Hide all unconnected input/output sockets. Tightly bundle remaining active sockets on the absolute left and right perimeter boundary points of the pill layout shell.
* **Inline Controls Integration:** Use `gpui-component` primitives (`NumberInput`, `Dropdown`) directly inside the expanded node body layout. Bind data changes directly to `WorkspaceContext::modify_attribute`. Never trigger raw USD layer operations mid-drag; only commit to the backend structural plane on mouse event release loops to avoid processing latency.

### 3. Structural Tree-Table Refactoring (`stage_composer.rs`)
* **Concept Shift:** Abandon the raw, unstructured text printout of USD stage structures. Re-architect this component into a clean, metadata-focused multi-column tree grid layout.
* **Formatting Rules:** Columns must explicitly show: `Primitive Node Path`, `Type Class`, `Weight/Allocation`, `Strategy Version`, and `Active Status`. 
* **Visuals:** Use horizontal dividers (`#26262b`), alternating row backplates (`#1e1e22` vs `#1b1b1f`), and indent rows cleanly using parent-child chevron collapse handles.

### 4. Code Generation & Safety Restraints
* **No UI Thread Blocks:** Never run OTL compilation, historical metric lookups, or path dependency sorts directly within a layout or render tick pass. Offload operations using `cx.background_executor().spawn()`.
* **No Circular Notify Cascades:** Ensure that background threads dropping data streams back into the main loop check if the structural version matches the active UI revision state before firing layout notifications.
* **Follow Framework Architecture:** Extend panes strictly through the established host trait abstraction patterns defined in `pulsar_marketlab_ui/src/workspace/`. Do not fork structural positioning primitives inside `split_layout.rs`.