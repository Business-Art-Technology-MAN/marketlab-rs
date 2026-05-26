# MarketLab SRD - Agent C Upgrade: Asynchronous CSV File Monitor & Hot Swap
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (Background Engine Thread loop)

---

### 1. Objective
Enable the background execution thread to continuously monitor the path variable of active asset nodes, immediately re-binding file handles and resetting the row stream position when a user updates a path via the UI.

### 2. Functional Requirements
* **Active Path Tracking Cache:** Inside the `spawn_csv_asset_feeder` worker loop loop, introduce an internal `current_active_path: String` tracking variable to cache the currently playing file handle location.
* **Hot-Swap Path Inspection:** At the top of the 400ms ticker processing loop, read the target asset node's path variable from the main shared view state context. Compare it directly against your internal `current_active_path` string cache.
* **Stream Re-initialization Sequence:** If the UI string path does not match the internal worker thread cache string value:
  1. Print an alert confirmation message to the bottom status logging panel.
  2. Instantly drop the active file handle and close old reader buffers.
  3. Attempt to open a clean file reader pointer targeting the newly specified path layout.
  4. Reset the file iterator row count position back to index zero to begin processing the fresh source from line one.
  5. Update `current_active_path` to match the new string path descriptor.
* **Graceful Failure Fallbacks:** If the path typed by the user is invalid or points to a missing file on disk, do not panic the application. Pause the reader loop, display a file warning message inside the bottom log panel, and wait until the path variable changes again.