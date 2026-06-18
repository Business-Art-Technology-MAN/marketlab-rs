# Directive: Integrate Graph Engine Invalidation with Workspace Context
Target File: `crates/pulsar_marketlab_ui/src/workspace/context.rs`

1. Update `WorkspaceContext::modify_attribute` and `connect_primitives`:
   - When an edit occurs, increment the internal `engine_cache_generation` identifier.
2. In the main application layout workspace, spawn an asynchronous worker thread using `cx.background_executor().spawn(...)`.
   - The thread checks if the cache generation is dirty. If yes, it completely recompiles the `MarketLabGraphEngine`, pulls fresh asset vectors, executes the compiled closures across the historical timeline, and dumps the final computed streams back into the workspace state for rendering.