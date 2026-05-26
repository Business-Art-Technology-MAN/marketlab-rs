# System Requirements Documents (SRDs)
**Project:** MarketLab Workstation Engine  
**Target Framework:** GPUI v0.2.2  

---

## SRD 1: Interactive Node Canvas & Spatial Draggable State

### 1. Objective
Enable users to interactively drag node cards across the 2D canvas space using mouse gestures, updating the node's absolute layout coordinates dynamically in the application state.

### 2. Functional Requirements
* **Drag-State Tracking:** Introduce a state manager to monitor when a node is actively being dragged. It must record:
  * `active_drag_node_id: Option<usize>`
  * `drag_offset: Point<Pixels>` (the difference between the initial mouse click position and the top-left corner of the node, preventing the card from snapping awkwardly on click).
* **Pointer Event Binding:**
  * **On Mouse Down (`on_mouse_down`):** Check for `MouseButton::Left` on the node header panel. If triggered, set the node ID as the active drag target and compute the click offset.
  * **On Mouse Move (`on_mouse_move`):** If a node ID is actively targeted, track the cursor's delta position. Update that specific node's `x` and `y` properties relative to the canvas origin.
  * **On Mouse Up (`on_mouse_up`):** Clear the active drag state when the left mouse button is released anywhere on the canvas window.
* **Frame Notification:** Call `cx.notify()` during the movement phase to ensure the canvas layer re-renders fluidly at full frame rate.

---

## SRD 2: Layout Hierarchy & Screen Space Partitioning

### 1. Objective
Establish a clean, non-overlapping, high-density desktop workspace shell partitioned into a flexible canvas, an inspector sidebar, and a status sheet at the bottom.

### 2. Functional Requirements
* **Parent Shell Container:** Map the root element to occupy the absolute maximum window size (`.size_full()`). Flex direction must be set to column (`flex_col`) with a background token of `0x09090b`.
* **Upper Section (Main Split View):**
  * Create a wrapper row utilizing `.flex_1()` to consume all available vertical space above the status panel.
  * **Left Panel (Node Canvas):** Set to `.flex_1()` so it aggressively expands to occupy all remaining width. Must be explicitly set to `.relative()` and `.overflow_hidden()` to anchor absolute-positioned nodes safely.
  * **Right Panel (Spreadsheet Inspector):** Set to a locked horizontal width of `w(px(384.0))` (96 units) with a left border separator (`border_l_1`) of `0x222227`.
* **Lower Section (Console Log Panel):** Set to a fixed height of `h(px(144.0))` (36 units) spanning the absolute bottom of the window frame, bounded by a top border separator (`border_t_1`).

---

## SRD 3: Visual Port Mapping & Selection Architecture

### 1. Objective
Expose distinct input/output connection points along the layout boundaries of individual node cards and track user selection highlighting.

### 2. Functional Requirements
* **Node Card Frame Styling:**
  * Render node cards as a vertical stack with a fixed width of `w(px(220.0))`, using a background color of `0x1c1c21` and a subtle rounded corner (`rounded_md`).
  * Monitor `selected_node_id: Option<usize>` in the global workspace view state. If a card matches the selection ID, shift its border color to a distinct active state (`0x3b82f6`); otherwise, fallback to the dormant border color (`0x2d2d34`).
* **Visual Port Distribution:**
  * Read a list of metadata ports bound to the node definition.
  * Iterate over the ports inside a child container. For each entry designated as an **Input Port**, left-align a text block or terminal bracket asset (`→ [ ]`) within the row.
  * For each entry designated as an **Output Port**, right-align the structural layout element, placing the terminal bracket asset (`[ ] →`) at the far right edge of the card boundary.

---

## SRD 4: Real-time Data Ingestion & State Threading

### 1. Objective
Bridge background execution threads with the main UI layout thread so that incoming register states continuously update the spreadsheet inspector view without blocking frame rendering performance.

### 2. Functional Requirements
* **Asynchronous Task Spawning:** Use the GPUI runtime task executor context (`cx.spawn`) during application startup to detach a long-running background polling worker loop.
* **State Ingestion Channel:**
  * The async worker task must periodically listen to an engine backtest tape or mock execution loop.
  * Upon receiving a data frame packet, use `view.update` to securely pass mutating data arrays into the main thread UI state model.
* **Buffer Cap Constraint:** Inside the mutation closure, append incoming rows to the `inspector_data` vector, and truncate old rows if the size exceeds a maximum capacity to prevent silent memory bloat.
* **UI Redraw Trigger:** Force an instant visual refresh of the register spreadsheet by executing a structural view notification (`cx.notify()`) immediately after every buffer push.
