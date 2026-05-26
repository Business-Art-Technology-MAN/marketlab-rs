Agent A (UI Specialist): Selection Context Filtering & Inspector Linkage
Target File: crates/pulsar_marketlab/src/main.rs (State UI Evaluation blocks)

1. Objective
Build an interactive relationship between the active canvas node selection state and the data inspector sidebar. Selecting a node must dynamically filter and transform the contents displayed in the spreadsheet inspector.

2. Functional Requirements
Data Row Struct Expansion: Update your spreadsheet data representation struct (DataRow or your active record wrapper) to include a node binding field:

Rust
pub associated_node_id: Option<usize>
Context-Aware Data Filtering: Inside the render_spreadsheet_inspector method, inspect the current workspace state (self.selected_node_id).

Condition A (No Node Selected): Render the full, un-truncated, real-time incoming register row buffer up to your maximum capacity cap.

Condition B (Node Selected): Filter the row buffer on the fly. Only render rows where associated_node_id == self.selected_node_id.

Dynamic Sidebar Header UX: Modify the inspector title element to provide context layout feedback:

When no node is active, render "📊 Global Register Inspector".

When a node is active, look up the node's string name from the active list and render "📊 Inspector Context // [Node Name]".

Conflict Prevention Rule: Do not touch mouse dragging vectors or async network channel definitions. Restrict mutations purely to the view evaluation loops inside the inspector rendering structures.