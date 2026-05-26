# MarketLab SRD - UI & Graphics Upgrade: Native Asset Charting Engine
**Target File Location:** `crates/pulsar_marketlab/src/main.rs` (Canvas Rendering & State layers)

---

### 1. Objective
Implement a hardware-accelerated, zero-dependency 2D vector charting engine inside the layout view model. Any node flagged as an Asset or Portfolio type must be able to project its historical price series as a responsive, anti-aliased vector line graph directly within its UI container boundaries.

### 2. Functional Requirements

#### A. Time-Series Cache Cache Schema
* **Historical Ring Buffer:** Update the data ingestion schema so that when an Asset node reads from `SharedCsvAssetPaths`, it caches up to the last 500 parsed close prices inside an internal tracking vector on the view state:
  ```rust
  pub struct ChartHistoryBuffer {
      pub timestamps: Vec<String>,
      pub values: Vec<f32>,
  }