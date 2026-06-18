# System Requirements Document (SRD)

## OTL Phase 2 Engine Codegen & Vector Sweep Integration

**Document Version:** 4.0.0

**Target Environment:** `marketlab-rs` / `crates/pulsar_marketlab_core`

**Context:** Compiler backend and execution wiring for Cursor coding agent

---

## 1. Executive Summary & Core Intent

With the legacy iterative simulation framework completely deprecated and removed, this sprint implements the backend compilation and execution pathways for the Open Trading Language (OTL) Phase 2 keywords: `signal`, `allocator`, and `portfolio`.

The core objective is to transition these three specialized block configurations from static Abstract Syntax Tree (AST) representations into functional operational instructions. These instructions will evaluate inside the ahead-of-time background timeline loop, allowing alpha convictions to dynamically drive capital allocation rules across contiguous time-series vectors (`ComputedAttributeStream`).

---

## 2. Structural Pipeline Overview

The execution pipeline must process the structural definitions down to the layer where primitive numerical vectors are calculated:

```
[ OTL Source Code ] 
       │
       ▼ (Parser / AST)
[ AbstractProgram ] ──> Tier-Specific Program Structs (SignalProgram, AllocatorProgram, PortfolioProgram)
       │
       ▼ (Codegen Linkage)
[ compile_object_program() ] ──> Emits executable token arrays or structural evaluation closures
       │
       ▼ (Runtime Vector Sweep)
[ evaluate_node_vector_series() ] ──> Loops over time-series arrays to populate column vectors

```

---

## 3. Targeted Implementation Files & Specifications

### 3.1 Compiler Backend Linkage

**File:** `crates/pulsar_marketlab_core/src/compiler/codegen.rs`

Implement or extend `compile_object_program` to process the structural variants into operational evaluation engines.

```rust
pub enum CompiledProgramTier {
    Signal(SignalExecutionEngine),
    Allocator(AllocatorExecutionEngine),
    Portfolio(PortfolioExecutionEngine),
}

pub fn compile_object_program(
    ast_program: &AbstractProgram,
    registry: &NodeRegistry
) -> Result<CompiledProgramTier, CompileError> {
    // 1. Inspect the ast_program tier type via parsed keywords
    // 2. Map symbolic identifiers to index-based column lookups
    // 3. Emit the optimized execution engine wrapper
    todo!("Wire AST nodes directly to high-performance operational vector instructions")
}

```

### 3.2 Runtime Engine Vector Integration

**File:** `crates/pulsar_marketlab_core/src/engine/vector_sweep.rs`

Modify `evaluate_node_vector_series` to ingest the compiled program tiers. The engine must loop across the time-series space to evaluate math assignments without allocating sub-arrays mid-loop.

```rust
pub fn evaluate_node_vector_series(
    ctx: &mut ExecutionContext,
    compiled_tier: &CompiledProgramTier,
    output_matrix: &mut GraphSeriesMatrix
) {
    let bar_count = ctx.timeline_length();
    
    match compiled_tier {
        CompiledProgramTier::Signal(engine) => {
            // Populate boolean or continuous alpha conviction streams
            for bar_idx in 0..bar_count {
                let conviction = engine.execute_at_bar(bar_idx, ctx);
                output_matrix.write_signal(bar_idx, conviction);
            }
        }
        CompiledProgramTier::Allocator(engine) => {
            // Read active signals from matrix, compute multi-asset allocation weights
            for bar_idx in 0..bar_count {
                engine.allocate_capital_at_bar(bar_idx, ctx, output_matrix);
            }
        }
        CompiledProgramTier::Portfolio(engine) => {
            // Process target weight configurations into simulated asset holding growth metrics
            for bar_idx in 0..bar_count {
                engine.track_portfolio_metrics_at_bar(bar_idx, ctx, output_matrix);
            }
        }
    }
}

```

---

## 4. Operational Codegen Execution Rules

1. **Zero-Allocation Time Loops:** All state changes and lookups occurring within the bar loops (`0..bar_count`) must use pre-allocated buffers or direct index slicing. Do not initialize or drop heap allocations (`String`, `Vec`) during individual bar processing iterations.
2. **Strict Calculation Dependencies:**
* `allocator` nodes must explicitly read from parent or connected `signal` vector buffers within the context.
* `portfolio` nodes must ingest macro target weights from `allocator` streams to drive calculation vectors such as Net Asset Value (NAV), drawdowns, and cash balances.


3. **Graceful Structural Error Handling:** If compilation or calculation dependencies fail (e.g., an allocator references a signal indicator that is missing from the active matrix workspace configuration), the engine must bubble up a clear `RuntimeEngineError` to prevent visual workspace locks.

---

## 5. Verification & Testing Requirements

* **Unit Test Requirement:** Implement automated integration test frames inside `crates/pulsar_marketlab_core/tests/codegen_spec.rs` to verify that an OTL script defining a custom alpha conviction properly modifies a simulated cash position across a minimum test window of 100 historical data points.
* **Compilation Target:** Execute `cargo check -p pulsar_marketlab_core` to verify that all code modifications compile cleanly without warnings or reference conflicts.
