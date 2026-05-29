# Blueprint Directive: Implement schema.usda Foundation & GPUI Menu Bar Actions
Target: `crates/pulsar_marketlab_core/` and `crates/pulsar_marketlab_ui/`

1. Write the Schema Asset:
   - Create the text file `crates/pulsar_marketlab_core/resources/usd/schema.usda` exactly as specified in the SRD.
   - Ensure it explicitly defines the typed classes "FinancialAsset", "OtlOperator", and "PortfolioIntegrator" using native `inputs:name` conventions.

2. Build the Global Workspace Menu Bar (`crates/pulsar_marketlab_ui/src/workspace/menu_bar.rs`):
   - Create a custom GPUI View named `MenuBar`. 
   - Render a high-contrast horizontal toolbar along the top edge of the interface viewport layout bounds.
   - Implement the "File" dropdown overlay containing context-driven entries for: New, Open, Save, and Save As.

3. Implement Native File IO Event Handlers:
   - When "Open" is clicked or `Ctrl + O` is entered, spawn an asynchronous worker thread using `cx.background_executor().spawn(...)` to fetch a target path via a file selection dialog.
   - Load the target `.usda` text layer using our pure-Rust OpenUSD layer engine, and cleanly overwrite the shared `Model<UsdStageBridge>` handle on the main execution loop thread.
   - When "Save" or `Ctrl + S` is triggered, dump the live stage data hierarchy directly to the verified on-disk path text stream and log a baseline telemetry success indicator.