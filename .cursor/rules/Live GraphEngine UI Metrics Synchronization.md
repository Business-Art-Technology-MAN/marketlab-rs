# System Requirements Document (SRD)
## Live GraphEngine UI Metrics Synchronization

**Document Version:** 1.0.0  
**Target Environment:** `marketlab-rs` / `crates/pulsar_marketlab_ui` & `crates/pulsar_marketlab`  
**Context:** Production-grade cursor workspace generation path  

---

## 1. Executive Summary & Core Intent

This document specifies the technical requirements to displace the legacy, out-of-band CSV simulation file path and establish a direct, low-latency, reactive data synchronization link between `GraphEngine` execution outputs and the GPUI layout interface. 

Both workspace canvas nodes and sidebar inspector panels must show live performance telemetry. To prevent redundant computation threads or layout locks during rapid time-series playback sweeps, a centralized, atomic memory lookup cache resource (`MetricsTelemetryBridge`) will be established globally inside the GPUI window application context.

---

## 2. Technical Component Architecture

The synchronization pipeline establishes an event-driven flow from the engine timeline iteration loop to the visual presentation layers:

1. **`MetricsTelemetryBridge` (Global Resource):** An in-memory cache indexing localized matrix performance structures by `NodeId` alongside unified global portfolio parameters.
2. **`CanvasNodeCard` Layout Update:** Subscribes reactively to the global cache on drawing ticks to display dynamic return, drawdown, and position exposure bounds on individual visual nodes.
3. **`SidebarInspector` Card Update:** Replaces file-system parsing logic with direct access to terminal portfolio boundaries, rendering aggregate account metrics.

---

## 3. Structural Code Definitions for Cursor Execution

### 3.1 The Shared Telemetry Bridge Layer
Create or append this structure inside `crates/pulsar_marketlab/src/ui/telemetry_bridge.rs` (or equivalent interface boundary):

```rust
use std::collections::HashMap;
use gpui::*;
use pulsar_marketlab_core::orchestration::graph_engine::SymbolicOtlClosure;

#[derive(Clone, Debug, Default)]
pub struct EvaluatedMetrics {
    pub total_return: f64,
    pub rolling_drawdown: f64,
    pub net_exposure: f64,
    pub current_conviction: f64,
    pub trailing_trades_count: usize,
}

#[derive(Default)]
pub struct MetricsTelemetryBridge {
    pub node_metrics: HashMap<usize, EvaluatedMetrics>,
    pub global_metrics: EvaluatedMetrics,
}

impl Global for MetricsTelemetryBridge {}

impl MetricsTelemetryBridge {
    /// Ingests a slice of computed runtime closure states to hydrate the active view cache
    pub fn push_timeline_frame(
        &mut self, 
        node_id: usize, 
        closures: &[SymbolicOtlClosure], 
        performance_map: &HashMap<String, f64>
    ) {
        let metrics = self.node_metrics.entry(node_id).or_default();
        
        if let Some(latest) = closures.last() {
            metrics.current_conviction = latest.raw_weight;
            metrics.net_exposure = latest.altered_weight;
        }
        
        if let Some(&r_total) = performance_map.get("total_return") {
            metrics.total_return = r_total;
        }
        if let Some(&dd) = performance_map.get("drawdown") {
            metrics.rolling_drawdown = dd;
        }
        if let Some(&trades) = performance_map.get("trades_count") {
            metrics.trailing_trades_count = trades as usize;
        }
    }
}