# Directive: Build the USD Stage Graph Compilation Core
Target File: `crates/pulsar_marketlab_core/src/orchestration/engine.rs`

1. Implement a `MarketLabGraphEngine` structure that utilizes `petgraph::stable_graph::StableGraph`.
2. Define a thread-safe node execution type enum:
```rust
pub enum ExecutionNode {
    DataInput { symbol: String },
    SignalTransform { expression: String, compiled_fn: Option<Box<dyn Fn(&[f64]) -> Vec<f64> + Send + Sync>> },
    PortfolioSink { method: String, initial_capital: f64 },
}