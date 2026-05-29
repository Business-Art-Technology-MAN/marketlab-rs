# Role & Context
You are an expert Rust systems architect. Task 1 is complete; `MarketProviderServices` and `OtlClosure` are available. Now, we need to refactor the OTL syntax evaluation engine away from single-point immediate evaluation to match the OSL closure-passing execution model.

# Target Files
- Refactor: `crates/pulsar_marketlab/src/signal_dsl/ast.rs`
- Refactor: `crates/pulsar_marketlab/src/signal_dsl/parser.rs`
- Refactor: `crates/pulsar_marketlab/src/signal_dsl/interpreter.rs` (or your local evaluator module)

# Technical Requirements

1. Refactor the Core Evaluator Loop to pass Closures:
   - Update the compiler/evaluator entry point. Instead of walking a `DslExpression` AST and returning a flat primitive scalar value immediately, it must recursively assemble an `OtlClosure` expression tree:
     
     - `DslExpression::Literal(val)`: Captures the value inside an `Arc::new(move |_, _| Some(Vector::from_scalar(val)))` block.
     - `DslExpression::Variable(name)`: Captures the variable identifier and defers its lookup to the runtime services handle: `services.get_global_attribute(&name, t)`.
     - `DslExpression::FunctionCall`: Recursively compiles all argument expressions into child closure blocks first, preserving them as independent, lazy observation capabilities.

2. Implement Decoupled Temporal Window Intrinsics:
   - Inside your internal function routing matrix (e.g., `execute_intrinsic_function`), add or update support for multi-point queries (e.g., `ta::sma`):
     a) Evaluate the lookback parameter closure at the current playhead coordinate `t` to dynamically extract a window duration.
     b) Compute the unconstrained historical window boundaries: `start = t - duration`, `end = t`.
     c) Query the data canvas through the abstract services interface: `services.sample_timeline("/assets/active/close", start, end)`.
     d) Perform the rolling mathematical average reduction across the returned `Vec<Vector>` block.

3. General Integrator Delegation Hook:
   - When the parser encounters an explicit integrator instruction, package the raw downstream collection of un-evaluated argument closures and hand them over directly to the host application interface: `services.execute_integrator(target_engine, argument_closures, t)`.

4. Test Harness Preservation:
   - Refactor the branch's 42 passing tests to work with this new closure return paradigm.
   - Instantiate your `MockMarketProvider` from Task 1 inside the test contexts. Pass the mock handle into the compiled `OtlClosure` output tree, trigger the execution at test playhead coordinate `t`, and assert that the resolved vector values match your expected analytical mathematical baselines.
   - Ensure all generated closure structures maintain strict thread safety bounds (`Send + Sync`) to guarantee they can be safely distributed across multi-core CPUs or GPU compute warps without thread contention.