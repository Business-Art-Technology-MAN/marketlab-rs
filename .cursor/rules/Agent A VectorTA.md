# MarketLab SRD - Agent A/B Upgrade: VectorTA Analytics Engine
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (State Models & Channel Processors)

---

### 1. Objective
Expose high-performance `vector-ta` indicator streams directly within the node logic, processing incoming data packets through active streaming indicators.

### 2. Functional Requirements
* **Dependency Addition:** Add `vector-ta = "0.2.7"` to your workspace dependencies.
* **Analytical Node Definitions:** Update the visual node template mapping to explicitly support analytical calculation profiles (e.g., Node 2 configured as a `VectorTA::ALMA` indicator or `VectorTA::ADX` calculator).
* **Stateful Stream Initialization:** Inside the background worker thread, when an analytics node initializes, instantiate its matching stateful indicator stream object (e.g., `vector_ta::indicators::alma::AlmaStream`).
* **Pipeline Processing Execution:** When a raw data packet arrives from the engine channel for an analytics node:
  1. Extract the current raw float value from the inbound message payload.
  2. Execute the stateful stream update step: `.update(value)`.
  3. Format the computed indicator result (e.g., the moving average or signal directional marker) straight into the `multivector_value` field of a `PipelineSystemMessage::TickUpdate` message, routing it instantly to the UI window.