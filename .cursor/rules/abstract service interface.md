# Role & Context
You are an expert Rust systems architect building MarketLab, a high-performance quantitative look-development platform inspired by OpenUSD and Open Shading Language (OSL). 

We are implementing an abstract service interface equivalent to OSL's `RenderServices`. This layer completely decouples our domain-specific execution language (Open Trading Language / OTL) from concrete data storage structures, specific math engines, and live execution brokers.

# Target Files
- Create: `crates/pulsar_marketlab/src/signal_dsl/services.rs`
- Update: `crates/pulsar_marketlab/src/signal_dsl/mod.rs` (expose the services module)

# Technical Requirements

1. Define the Universal Closure Payload Type:
   - Ensure you import your internal geometric `Vector` primitive.
   - Define a polymorphic, thread-safe higher-order function type alias named `OtlClosure`:
     `pub type OtlClosure = Arc<dyn Fn(&dyn MarketProviderServices, f64) -> Option<Vector> + Send + Sync>;`

2. Implement the `MarketProviderServices` Trait:
   - Define `pub trait MarketProviderServices: Send + Sync` with three pure abstract message-passing hooks:
     
     a) `fn sample_timeline(&self, path: &str, start_time: f64, end_time: f64) -> Vec<Vector>;`
        - Purpose: Pulls an unconstrained, multi-point slice of data across historical time boundaries (e.g., pathing format: "/assets/SPY/close", "/alternative/news_decay").
     
     b) `fn get_global_attribute(&self, name: &str, t: f64) -> Option<Vector>;`
        - Purpose: Queries framing properties native to the execution frame at playhead time t (e.g., "global::lookback_duration", "portfolio::cash").
     
     c) `fn execute_integrator(&self, integrator_name: &str, inputs: &[OtlClosure], t: f64) -> Option<Vector>;`
        - Purpose: Routes raw, un-evaluated closure capabilities down to arbitrary, registered external analytical engines (like vectorTA or traditional time-series reduction frames).

3. Enforce Strict Decoupling Rules:
   - `services.rs` must have ZERO dependencies on specific time-series buffers, databases, or ledger execution caches. It must only depend on your core `Vector` mathematical primitive and standard library types.

4. Testing Frame Harness:
   - Inside a `mod tests` block, implement a lightweight `MockMarketProvider` struct that satisfies the `MarketProviderServices` trait.
   - Hardcode an internal lookup structure so that querying a specific test path via `sample_timeline` returns a fixed, deterministic sequence of mock price vectors. 
   - Verify that invoking the interface methods compiled against the mock engine works correctly.