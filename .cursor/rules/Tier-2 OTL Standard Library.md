# Role & Context
You are an expert Rust systems architect working on MarketLab. We are extending our Tier-2 OTL Standard Library (`signal_dsl`) to support native, built-in financial primitives for Returns and Volatility calculations, matching Open Shading Language's (OSL) paradigm of global intrinsic scene functions.

These built-ins must be decoupled from static configurations, allowing child parameters to be dynamically resolved at playhead time `t` across our Split-Plane storage engine.

# Target Files
- Update: `crates/pulsar_marketlab/src/signal_dsl/interpreter.rs` (Add routing for financial intrinsics)
- Update: `crates/pulsar_marketlab/src/stage_bridge/production_provider.rs` (Implement concrete mathematical math execution)
- Update: Tests in `crates/pulsar_marketlab/src/signal_dsl/interpreter.rs` or `tests/end_to_end_core_spec.rs`

# Technical Requirements

1. Expand OTL Core Intrinsics Routing to include Financial Namespaces:
   Update `execute_intrinsic_function` or your integrator dispatch engine to natively parse and validate the following standard functions:
   - `rtn::log(period)`: Compiles to a closure that calculates log returns: `ln(price_t / price_{t-period})`.
   - `vol::realized(period)`: Compiles to a closure that calculates the rolling historical standard deviation of log returns over the requested temporal slice.
   - `vol::parkinson(period)`: Compiles to a closure executing Parkinson high-low range variance calculations over the requested window.

2. Implement Mathematical Execution inside the Production Stage Bridge:
   - When a financial intrinsic is triggered at playhead `t`:
     a) Evaluate the `period` argument closure at time `t` to dynamically resolve the calculation window.
     b) Request the raw high, low, and close time-series data arrays through the abstract `MarketProviderServices::sample_timeline` interface.
     c) Perform the mathematical reduction (log return calculation or variance array standard deviation) over the returned `Vec<Vector>` blocks.
     d) Package the result as a polymorphic geometric `Vector` and pass it back down the wire.

3. Build Standard Library Shader Chaining Cases:
   Verify that these new financial built-ins can be safely fed into standard math helper nodes. For example, a script block must be able to cleanly evaluate expressions like:
   - `clamp(rtn::log(3), -0.05, 0.05)` (Limiting extreme return shocks)
   - `mix(close, sma(5), vol::realized(10))` (Dynamically blending values based on changing volatility thickness)

4. Verification Contract:
   - Write unit and integration tests confirming that:
     1. Compiling and running `"rtn::log(3)"` against a mock or production asset stage accurately returns historical log return differentials.
     2. Evaluating `"vol::realized(3)"` returns mathematically accurate standard deviation steps across a sequence of varying prices.
     3. All newly added financial closures maintain strict thread-safety boundaries (`Send + Sync`) to protect parallel multi-core execution loops.