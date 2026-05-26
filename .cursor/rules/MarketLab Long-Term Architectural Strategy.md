# MarketLab Long-Term Architectural Strategy
## Integrating Metricless USD Time, Hydra Dispatching, and OSL Shading Shaders

---

### Phase A: The Core Functional Backbone (Current Workstream)
* **Goal:** Lock down the end-to-end Rust computation pipeline.
* **UI Focus:** Complete the Portfolio node sidebar and the Lower Status Console to verify signal closures, order executions, and classical return metrics (Sharpe, Drawdowns) using real Yahoo CSV data.
* **Systems Focus:** Unify the graph state (`SharedPipelineGraph`) so canvas node wires dynamically drive the `vector-ta` backend execution threads.

### Phase B: The OpenUSD Schema & Metricless Time Transition
* **Goal:** Replace the flat `MatrixDataRow` vector buffers with an in-memory USD Stage representation.
* **Data Layer:** Define custom USD schemas (`UsdTradingStage`, `UsdAssetModel`) where time-series ticks are stored as time-sampled attributes.
* **Agnostic Alignment:** Wire a mock FIX protocol network stream alongside the CSV file reader, demonstrating that USD can automatically align dense microsecond events with sparse bar charts via continuous time interpolation.

### Phase C: The OSL-Inspired Signal DSL Engine
* **Goal:** Implement the custom domain-specific language compiler/interpreter for writing custom signal shaders.
* **Language Design:** Define a high-level, human-readable math syntax inspired by OSL closures and shading networks, tailored for writing high-performance Technical Analysis modules.
* **Math Compilation:** The DSL must compile scripts directly down to safe, parallelized execution blocks that plug into your Layer 3 `signal_kernel` (Clifford hyper-rotors and covariance metrics).

### Phase D: Hydra Execution Stream Integration
* **Goal:** Wire Hydra scene delegates to drive the real-time workstation execution loops.
* **Render Pipeline:** Route the output of the USD data stages and analytical shaders into a Hydra render delegate, enabling real-time, hardware-accelerated 3D performance graphs, order-book depth maps, and portfolio risk landscapes.