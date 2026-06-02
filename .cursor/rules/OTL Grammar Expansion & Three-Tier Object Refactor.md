# System Requirements Document (SRD)
## OTL Grammar Expansion & Three-Tier Object Refactor

**Document Version:** 1.0.0  
**Target Environment:** `marketlab-rs` / `crates/pulsar_marketlab_core` & `crates/pulsar_marketlab`  
**Context:** Production-grade cursor workspace generation path  

---

## 1. Executive Summary & Core Intent

This document establishes the technical specification to explicitly separate allocation and compilation logic from stateful historical execution. The Open Trading Language (OTL) Domain-Specific Language (DSL) is expanded from a monolithic `shader` topology into a strictly typed, **Three-Tier Object Model** introducing the explicit syntactic keywords: `signal`, `allocator`, and `portfolio`. 

Under the hood, the compiler frontend unifies these declarations into a shared Abstract Syntax Tree (AST) node type (`OtlObject`), but enforces compile-time semantic rules based on intent. Crucially, all system nodes across the execution graph must update their function signatures to explicitly handle these object variants, maintaining full nesting support (meta-signals and sub-portfolios) while preserving scale-invariant symbolic closure pipelines.

---

## 2. Grammar Specification & AST Definitions

### 2.1 Lexer Additions
Register the following reserved keywords in the OTL compiler frontend (`crates/pulsar_marketlab_core/src/frontend/lexer.rs`):
* `signal` (Token type: `TokSignal`)
* `allocator` (Token type: `TokAllocator`)
* `portfolio` (Token type: `TokPortfolio`)

2.2 AST Structural Modifications
Modify the object declaration structure inside crates/pulsar_marketlab_core/src/frontend/ast.rs:

```Rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OtlObjectKind {
    Signal,
    Allocator,
    Portfolio,
    LegacyShader, // Maintained for legacy testing back-compat
}

#[derive(Debug, Clone)]
pub struct OtlObjectDeclaration {
    pub kind: OtlObjectKind,
    pub name: String,
    pub inputs: Vec<PropertyDeclaration>,
    pub outputs: Vec<PropertyDeclaration>,
    pub body: Vec<Statement>,
}
```
#3. Node Function Signature Migration Layout
To support the three-tier object architecture throughout the graph traversal, validation, and optimization passes, all core engine node interfaces must be updated.

3.1 Registry & Compilation Pipeline
Update crates/pulsar_marketlab/src/graph_compiler/registry.rs:

```Rust
// BEFORE:
// pub fn register_shader(name: &str, ast: LegacyAstNode) -> Result<(), CompileError>;

// AFTER:
pub fn register_otl_object(
    name: &str, 
    kind: OtlObjectKind, 
    ast: OtlObjectDeclaration
) -> Result<(), CompileError>;
```
##3.2 Graph Engine Traversal Node
Update crates/pulsar_marketlab_core/src/orchestration/graph_engine.rs:

```Rust
// BEFORE:
// pub fn evaluate_node_closures(&mut self, node_id: NodeId, ctx: &EvaluationContext) -> Vec<SymbolicOtlClosure>;

// AFTER:
pub fn evaluate_node_closures(
    &mut self, 
    node_id: NodeId, 
    object_kind: OtlObjectKind, 
    ctx: &EvaluationContext
) -> Result<Vec<SymbolicOtlClosure>, ExecutionError>;
```
##3.3 Canvas Composition Hydration Hook
Update crates/pulsar_marketlab/src/canvas_compose.rs:

```Rust
// BEFORE:
// pub fn hydrate_canvas_node(prim_path: &str, script_src: &str) -> CanvasNodeInstance;

// AFTER:
pub fn hydrate_canvas_node(
    prim_path: &str, 
    script_src: &str,
    expected_tier: OtlObjectKind
) -> Result<CanvasNodeInstance, HydrationError>;
```
#4. Semantic Validation Rules (Compiler Pass)
Right after parsing the AST, a dedicated validation pass must check the graph's connections to ensure semantic layout logic remains sound:

signal Declarations:

Must output a valid closure position.

Prohibited: Cannot accept arrays of closures (closure position inputs[]) as inputs. This prevents raw alpha channels from managing macro allocation weights.

allocator Declarations:

Must accept either a single closure or an array of closures.

Must output a modified, blended, or aggregated composite closure.

Prohibited: Cannot invoke or access lower-level execution primitives like portfolio_info("global_max_drawdown"). Allocators must remain scale-invariant and completely blind to absolute portfolio cash boundaries.

portfolio Declarations:

Acts as the final gating block leading into the TerminalIntegrator.

Permitted to access global state tracking parameters (portfolio_info).

Outputs the finalized execution map to be bound immediately to the integrate_portfolio loop in engine.rs.

#5. Execution Pipeline & Nesting Integration
To preserve structural layout flexibility on the canvas, the compiler framework must handle nested patterns without downscaling data down the wire.

##5.1 Meta-Signals (Signals on Signals)
When a signal block ingests a closure parameter from an upstream signal block:

The execution loop evaluates the upstream dependencies first.

The parent signal reads the raw SymbolicOtlClosure properties (such as checking closure_info(raw_signal, "asset_id")), applies its internal conditional filters (e.g., trend gates), and outputs the altered closure token.

##5.2 Sub-Portfolios (Portfolios inside Portfolios)
When a child portfolio script is wired as an input to a parent portfolio script:

The engine traverses the tree from the terminal root backwards.

The child portfolio node processes its local scripts (handling independent asset group risk limits, sector tracking metrics, or specialized sub-portfolio asset drawdowns).

The child node compiles these positions into a single multi-layered Sub-Portfolio Composite Closure Tree and hands it to the parent edge.

The parent master portfolio node applies macro portfolio weights across the collected sub-portfolio trees.

The Single Execution Boundary Guarantee: The absolute physical cash sizing and multiplier scaling calculations (nominal_units = (total_equity * weight) / (price * multiplier)) run exactly once inside engine.rs at the absolute terminal node boundary. This completely decouples complex logical composition from operational accounting passes.

##6. Cursor Context Verification Prompts
Ensure all 53 existing tests in pulsar_marketlab_core pass without modifying underlying matrix tracking outputs.

Verify that pulsar_marketlab_ui compiles seamlessly against the modified layout hooks in sidebar_inspector.rs and portfolio_integrator_ledger.rs.