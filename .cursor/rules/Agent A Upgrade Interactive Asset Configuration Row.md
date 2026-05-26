# MarketLab SRD - Agent A Upgrade: Interactive Asset Configuration Row
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (Inspector View Model block)

---

### 1. Objective
Provide a clean text input interface inside the spreadsheet inspector sidebar whenever an Asset Node is selected, allowing the user to modify the target CSV path string dynamically.

### 2. Functional Requirements
* **Asset Node Detection Check:** Inside the `render_spreadsheet_inspector` layout method, evaluate the current `self.selected_node_id`. If the selected node possesses a `node_type` matching `NodeType::Asset`, execute the asset configuration interface row layout.
* **Text Input Frame Container:** Render a clear text entry block directly beneath the sidebar context header styled to match the workspace theme background (`0x141417`). Label the section `"📁 Data Stream Target Path:"`.
* **Path State Sync Mutation:** Wire keyboard typing changes or string entries completed via the Return key to overwrite the current node's internal `AssetSourceType::Csv { path }` property directly in the global workspace view state.
* **Canvas Focus Redraw:** Trigger `cx.notify()` immediately upon string submittal to force the UI workspace view hierarchy to re-evaluate and clear layout artifacts.