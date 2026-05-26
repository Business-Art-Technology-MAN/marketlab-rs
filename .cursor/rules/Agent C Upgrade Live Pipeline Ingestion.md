# MarketLab SRD - Agent C Upgrade: Live Pipeline Ingestion
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (Engine Feeder Loop)

---

### 1. Objective
Replace the hardcoded static values in the data worker loop with a continuous streaming engine ingestion loop reading actual asset data matrices.

### 2. Functional Requirements
* **Data Stream Connection:** Update `spawn_pipeline_engine_feeder` to process an incoming queue of raw market values or historical replay buffers instead of hardcoded strings.
* **Matrix Packaging:** Parse raw decimal ticks into structured `MatrixDataRow` frames. The fields must accurately map active timestamp ticks, asset tickers (e.g., `"ES_F"`), and real numerical vectors.
* **Message Serialization:** Ensure every inbound tick parses smoothly into a `PipelineSystemMessage::TickUpdate` data packet and routes directly to the cross-thread `mpsc::Sender` handle.
* **Jitter Control Constraints:** Ensure the loop thread sleeps or awaits via a non-blocking 16ms cadence window to maintain predictable message volume and protect the main layout thread from event queues.