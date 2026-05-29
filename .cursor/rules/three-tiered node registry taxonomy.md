# Role & Context
You are an expert Rust systems architect working on MarketLab. We have successfully split our data pipeline into an OpenUSD Structural Plane and a Vector Temporal Plane. 

Now, we need to update `graph_compiler.rs` to support our expanded, three-tiered node registry taxonomy. We must ensure that the graph compiler can statically analyze and validate different wire configurations (e.g., preventing users from wiring an OpenUSD Asset path straight into a numeric math port, or capturing OTL Arbitrary Output Variables [AOVs]).

# Target Files
- Update/Refactor: `crates/pulsar_marketlab/src/graph_compiler.rs` (or your active node registry modules)
- Update: Node testing frames inside `crates/pulsar_marketlab/src/graph_compiler/tests.rs`

# Technical Requirements

1. Define the Three-Tiered Node Schema Enumeration:
   Represent our explicit architectural categories inside the compilation loop:
   ```rust
   #[derive(Debug, Clone, PartialEq)]
   pub enum NodeType {
       /// Tier 1: Non-executable OpenUSD Structural nodes (maps directly to a Prim or Layer path)
       AssetAdaptor { prim_path: String },
       /// Tier 2: Executable OTL script closure or a standard library node (mix, clamp, step)
       OtlShader { script: String },
       /// Tier 3: Terminal Exporter/Integrator blocks that execute lookbacks and write results (Spreadsheets, VectorTA)
       TerminalIntegrator { engine_target: String },
   }