# MarketLab SRD - Agent B Upgrade: Viewport Pan & Zoom Engine
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (Canvas View Render & Mouse loops)

---

### 1. Objective
Transform the layout canvas into an infinite, scale-aware 2D viewport workspace. Users must be able to pan the viewport via middle-mouse dragging and zoom smoothly in and out centered around their cursor via the mouse scroll wheel.

### 2. Functional Requirements
* **Viewport Tracking State:** Add `pan_offset: Point<Pixels>` (default to 0,0), `zoom_scale: f32` (default to 1.0), and `is_panning: bool` to the root workspace struct.
* **Canvas Scroll Wheel Capture:** Bind `on_scroll_wheel` to the parent canvas frame. Adjust `zoom_scale` smoothly using the vertical wheel delta. Clamp scale strictly between `0.15` (maximum zoom out) and `3.0` (maximum zoom in).
* **Middle-Mouse Viewport Pan:** * Bind `on_mouse_down` for `MouseButton::Middle` to toggle `is_panning = true`.
  * In the canvas `on_mouse_move` handler, if `is_panning` is active, add the cursor's movement delta straight into `pan_offset` and fire `cx.notify()`.
  * Clear `is_panning` to `false` when `MouseButton::Middle` triggers an `on_mouse_up` lifecycle callback.
* **Transformed Matrix Rendering Layout:** Update the logic inside `render_node_canvas`. Before rendering a node card or calculating port vector line connection paths, project world positions into screen coordinates:
  * Display Width = `220.0 * zoom_scale`
  * Display Left = `(node.x * zoom_scale) + pan_offset.x`
  * Display Top = `(node.y * zoom_scale) + pan_offset.y`
* **Transformed Drag Correction:** Update your existing node dragging calculations. When mutating `node.x` and `node.y` while dragging, divide the raw mouse cursor delta by the active `zoom_scale` factor to ensure dragging speeds scale naturally relative to the viewport depth.