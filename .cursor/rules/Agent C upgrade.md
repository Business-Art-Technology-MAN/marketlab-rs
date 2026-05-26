# MarketLab SRD - Agent A Upgrade: Canvas Spawning & Node Genesis
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (UI Overlay & Menu states)

---

### 1. Objective
Allow users to create new modular nodes directly on the canvas at their cursor's exact coordinates via a localized context interaction.

### 2. Functional Requirements
* **Context State Tracking:** Add `context_menu_pos: Option<Point<Pixels>>` to the workspace.
* **Background Interception:** Bind an `on_mouse_down` listener to the empty background layout of the canvas container, filtering exclusively for `MouseButton::Right`. On click, populate `context_menu_pos` with the absolute local coordinate click location.
* **Absolute Menu Overlay:** If `context_menu_pos` is `Some(pos)`, render a small, high-density popover panel at those absolute pixel coordinates (`.absolute().left(pos.x).top(pos.y)`), styled with your dark theme assets (`0x1c1c21`).
* **Node Struct Injection:** Add action items to the menu (e.g., "Spawn Custom Node"). Clicking an item must trigger a state mutation that:
  1. Generates a new unique `id` integer.
  2. Creates a `VisualNode` struct with pre-configured default ports.
  3. Sets the new node's `x` and `y` fields to match the captured click coordinates.
  4. Appends the node to the active `self.nodes` vector.
  5. Resets `context_menu_pos` to `None` and triggers a workspace redraw via `cx.notify()`.