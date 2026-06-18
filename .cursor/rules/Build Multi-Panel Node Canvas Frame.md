# Directive: Build Multi-Panel Node Canvas Frame
Target Module: `crates/pulsar_marketlab_ui/src/workspace/node_canvas.rs`

1. Construct an interactive rendering space displaying node structures horizontally (Blender Paradigm).
2. Wire up drag-and-drop mouse handlers to map spatial layouts to text instructions:
   - Releasing a node wire connection path over an execution slot must automatically compile an underlying `stage.set_relationship()` directive targeting the string path of the upstream primitive.
3. Build sub-canvas environment tabs:
   - Double-clicking an aggregator block must open a clean canvas slate focused exclusively on the target path's internal constituent array space.