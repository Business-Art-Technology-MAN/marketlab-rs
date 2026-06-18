# Role & Context
You are an expert systems developer specializing in Rust, high-performance desktop graphics architectures, and the GPUI framework (0.2.2). You are refactoring the workstation layout of `MarketLab`, an immediate-mode trading system workbench leveraging an OpenUSD structural plane and a multi-threaded execution plane.

# Objective
Execute a comprehensive layout restructuring and visual overhaul to transition the application from a high-contrast Bloomberg Terminal aesthetic to a cohesive, professional Digital Content Creation (DCC) pipeline workbench (inspired by Blender and usdtweak). Eliminate white-luminance inputs, ensure bidirectional selection synchronization without re-entrancy, and structure nested metadata cleanly.

## Phase 1: Visual Theme & Token Realignment (`theme.rs` / Style Engine)
* **Canvas Backplate Color:** Override the main canvas workspace background to a muted charcoal: `gpui::color::rgb(0x1b1b1f)`.
* **Grid Formatting:** Render background grid layout lines at 100px major and 20px minor divisions using a low-contrast tone: `gpui::color::rgb(0x26262b)`.
* **Node Chassis Styling:** Paint standard node backgrounds with `gpui::color::rgb(0x2d2d32)` and focused selection halos with `gpui::color::rgb(0x3b82f6)`. Apply an explicit curve profile of `8.0px` (`rounded_md()`) with a `1.0px` dark outline boundary (`#121214`) to ensure anti-aliased geometric sharpness.
* **Component Input Re-skinning:** Locate `NumberInput` and `Dropdown` rendering blocks inside the node body. Invert their contrast: Set their container background color to a sunken dark fill `gpui::color::rgb(0x1a1a1e)` and map inner text/digits to crisp, off-white typography `gpui::color::rgb(0xe5e5ea)`. Completely eradicate solid white (`#FFFFFF`) input fills.

## Phase 2: Node Canvas Layout Upgrades (`node_canvas.rs`)
* **Blender Pill Layout Mode:** Check a `node.collapsed` flag from the local visual state view. If true, branch the rendering pass into a single-row capsule layout: `rounded_full()`, height constraint `h_7()`, fixed width `w_180`. Omit all unconnected sockets; pack remaining active input/output sockets tightly to the far left and right coordinates of the pill perimeter.
* **Inline Variant Selectors:** Inspect if a node primitive contains active USD Variant Sets (`prim.HasVariantSets()`). If true, render a compact, inset dropdown menu inside the node header or parameters block displaying available variant options.
* **Event Deferral:** Route dropdown selections and slider drag updates through `cx.defer()` or `cx.defer_in()`. Never write to the underlying USD layer synchronously during high-frequency mouse movement frames; only commit attribute changes to the `WorkspaceContext` structural plane on mouse event release.

## Phase 3: High-Density Tree-Table Refactoring (`stage_composer.rs`)
* **Layout Structure:** Re-architect the Stage Composer pane from a raw text tree into a split horizontal panel.
* **Sub-Panel A (Layer Stack Selector):** Render a top-anchored panel displaying the active USD Layer Stack (`stage.GetLayerStack()`). Include interactive checkbox toggles next to each layer path row to let users visually re-target `stage.SetEditTarget(layer)`.
* **Sub-Panel B (Metadata Spreadsheet):** Render a structured grid containing explicit layout columns for: `Primitive Node Path` (with nested indent chevrons), `Type Class`, `Weight/Allocation`, `Strategy Version`, and `Active Status`. Style rows with alternating subtle background tones (`#1e1e22` vs `#1b1b1f`).
* **Safe Selection Routing:** Route all row clicks directly through the established, non-re-entrant `host.select_stage_path()` system to safely update the central selection generation flag without triggering mutable borrow conflicts.