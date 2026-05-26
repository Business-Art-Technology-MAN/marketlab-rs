# MarketLab SRD - Agent B Upgrade: Interactive Manual Wiring
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (Canvas Event & Vector layers)

---

### 1. Objective
Enable users to click a node's output port and drag an elastic, real-time cubic Bézier wire across the canvas that tracks the mouse cursor until it is dropped onto an input port, establishing a new functional connection.

### 2. Functional Requirements
* **State Management:** Add `active_wire_source: Option<(usize, usize)>` (storing from_node_id and from_port_idx) and `active_mouse_pos: Point<Pixels>` to the main workspace state.
* **Source Event Capture:** Bind an `on_mouse_down` listener to all port hitboxes. If an output port is left-clicked, populate `active_wire_source` with that node's information.
* **Canvas Pointer Tracking:** Bind an `on_mouse_move` listener to the parent Canvas background frame element. If `active_wire_source` is `Some`, update `active_mouse_pos` with the current cursor location and call `cx.notify()`.
* **Dynamic Path Generation:** Modify the vector rendering loop. If a wire is in flight, calculate the absolute screen-space origin of the starting port and draw an active blue cubic Bézier curve from that coordinate point directly to the fluctuating `active_mouse_pos`.
* **Connection Commitment:** Bind an `on_mouse_up` listener to all input port frames. If the mouse button is released over an input port while a wire is in flight, validate that it isn't connecting a node to itself, construct a new `NodeConnection`, push it into the graph array, and reset `active_wire_source` to `None`.