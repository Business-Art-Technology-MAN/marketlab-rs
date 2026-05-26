# MarketLab SRD - Agent C Upgrade: Yahoo Finance CSV Asset Node Ingestion
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (State Config & Engine Worker blocks)

---

### 1. Objective
Introduce a polymorphic "Asset" node architecture to the workspace, specifically implementing a streaming CSV engine that reads, parses, and loops real historical Yahoo Finance export files down the cross-thread channel.

### 2. Functional Requirements
* **Asset Data Model Expansion:** Update the visual node metadata structs to support an `AssetSourceType` enum containing a `Csv { path: String }` property variant.
* **Yahoo Finance Parsing Implementation:** Include the `csv = "1.3"` dependency in the workspace crate manifests. Inside the background thread handler, build a file reader function that targets the configured path value.
* **Column Mapping Rules:** Map incoming Yahoo Finance file lines using the following target structural configuration:
  * `Date` column $\rightarrow$ Ingest straight into `MatrixDataRow::tick` string.
  * `Close` (or `Adj Close`) column $\rightarrow$ Convert to a clean string format and store in `MatrixDataRow::multivector_value`.
  * Set `MatrixDataRow::asset` string property to the active filename or ticker context (e.g., `"SPY"`).
* **Cross-Thread Stream Dispatch:** For every line parsed out of the file, package the structural metrics inside a `PipelineSystemMessage::TickUpdate` data envelope. Explicitly tag `associated_node_id` with the current Asset Node's identifier value so it routes directly to the isolated Inspector filtering loop.
* **Playback Rate Regulator:** Introduce a non-blocking execution block that steps through rows sequentially at your active engine tick speed (e.g., 400ms intervals or raw frame-time ticks), wrapping around to the beginning of the file when it hits EOF to ensure continuous streaming look-development.