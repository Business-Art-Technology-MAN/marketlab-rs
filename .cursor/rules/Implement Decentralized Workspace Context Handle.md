# Directive: Implement Decentralized Workspace Context Handle
Target Module: `crates/pulsar_marketlab_ui/src/workspace/context.rs`

1. Design the `WorkspaceContext` data container:
   - Wrap the open `openusd::Stage` structure as a thread-safe, managed handle.
   - Separate UI-only tracking vectors into dedicated parameters: `selected_path: Option<String>`, `node_positions: HashMap<String, Point2D>`, and `active_panels: Vec<PanelType>`.

2. Build out standardized Model-View-Update (MVU) mutation endpoints:
   - Implement an explicit transaction wrapper: `pub fn set_usd_attribute(&mut self, prim_path: &str, attr: &str, val: openusd::Value, cx: &mut ModelContext<Self>)`.
   - The wrapper must write the variable directly down to the passive USD memory layer, invalidate any downstream calculation caches on the engine thread pool, and trigger a global window notification via `cx.notify()`.