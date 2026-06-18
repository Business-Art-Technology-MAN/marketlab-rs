# Directive: Build High-Density Stage Ledger Explorer Panel
Target Module: `crates/pulsar_marketlab_ui/src/workspace/stage_ledger.rs`

1. Instantiate a grid view parsing a shared `Model<WorkspaceContext>` handle. Attach a structural listener using `cx.observe(workspace, |this, _, cx| cx.notify())` to automate app-wide repaints.
2. Formulate 4 explicit text parsing tracks within the tree renderer loop:
   - TRACK 1 (Activation): Query 'inputs:active'. If false, enforce an absolute 0.4 opacity styling across the row elements.
   - TRACK 2 (Specifier Types): Inspect the core USD primitive state. If the prim is an override layer, append an amber badge labeled "⚠️ OVERRIDE ACTIVE".
   - TRACK 3 (Value Tracking): Highlight properties that deviate from the fallback 'schema.usda' defaults using bold typography styles.
   - TRACK 4 (Lineage): Traverse relationship arrays ('inputs:target', 'inputs:constituents') and generate inline directional labels mapping to the targets.