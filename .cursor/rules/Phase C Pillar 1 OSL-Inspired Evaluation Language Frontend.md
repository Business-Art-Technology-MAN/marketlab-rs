# MarketLab SRD - Phase C Pillar 1: OSL-Inspired Evaluation Language Frontend
**Target Directory:** `crates/pulsar_marketlab/src/signal_dsl/` (Create this module)
**Target Files:** `src/signal_dsl/mod.rs` (New), `src/graph_compiler.rs` (Integration)

---

### 1. Objective
Implement a lightweight, dynamically evaluated expression parser and compiler frontend inspired by Open Shading Language (OSL). This engine translates text-based mathematical equations defined within canvas node parameters into executable runtime blocks, replacing static indicator logic with an on-demand look-development shader pipeline.

---

### 2. Functional Requirements

#### A. The DSL Abstract Syntax Tree (AST) & Lexer
* **Token Registry:** Stand up a basic tokenizer that handles standard mathematical symbols (`+`, `-`, `*`, `/`), float constants, variables (`close`, `volume`), and nested functional transformations (`sma`, `rsi`).
* **The AST Node Tree:** Define an internal expression evaluator enum to structure parsed formulas:
  ```rust
  pub enum DslExpression {
      Literal(f32),
      Variable(String),
      BinaryOp(Box<DslExpression>, char, Box<DslExpression>),
      FunctionCall { name: String, args: Vec<DslExpression> },
  }