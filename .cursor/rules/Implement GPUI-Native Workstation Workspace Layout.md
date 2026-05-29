# Blueprint Directive: Implement GPUI-Native Workstation Workspace Layout
Target: Create crates/pulsar_marketlab_ui/src/workspace/

1. Structure a nested resizable Splitter Tree inside `mod.rs`:
   - Split vertically to separate the Upper Workstation panels from the Lower Render Viewport panel.
   - Split horizontally across the upper panel to house the Left (Stage Composer), Center (Node Canvas), and Right (Inspector) panes.
   - Ensure panels can be dynamically dragged and resized at 60fps+ utilizing GPUI's high-speed rendering cycles.

2. Implement the specific pane view files:
   - `stage_composer.rs`: Renders the USD Layer Tree hierarchy with checkbox bindings to FieldKey::Active flags.
   - `node_canvas.rs`: A custom painting element mapping Tier-1, 2, and 3 nodes with color-coded socket pins (Gray/Green/Cyan) matching PortWireKind boundaries and red alerts for graph compiler errors.
   - `param_inspector.rs`: Hosts an inline text field for editing OTL script text strings and a checkbox list to toggle AOV channel outbound pins.
   - `render_viewport.rs`: Houses a dense, grid-based financial ledger table and a continuous timeline playhead slider that updates the evaluation coordinate 't' reactively.

3. Guarantee Thread Safety:
   - Ensure all views maintain proper GPUI Model/View encapsulation.
   - Dispatch background timeline sweeps and script compilations using `cx.background_executor().spawn(...)` to prevent UI thread lockups, ensuring the entire workspace compiles seamlessly with our existing Send + Sync backend code.