# MarketLab Technical Specification: OTL Uber Signal Primitives

## 1. Purpose & Context
This document specifies the architectural refactor of the Technical Analysis (TA) node interface in MarketLab (`crates/pulsar_marketlab_core/src/orchestration/compiler.rs` and `schema.usda`). 

To prevent an explosion of unique node types and reduce code churn across the OpenUSD layer serialization and GPUI rendering trees, we unify technical analysis operations into four clear **Uber Signal** node archetypes. These map high-level visual configurations to optimized background `VectorTA` operations via dynamic `inputs:script_src` generation.

---

## 2. Structural Split-Plane Invariants
1. **Immutable Port Signatures:** Switching a node's internal algorithm or hyperparameters within the Parameter Inspector must never dynamically add or remove physical ports on the canvas topology, preventing graph validation crashes in `graph_compiler/registry.rs`.
2. **Asynchronous Execution:** Script compilation and vectorized timeline array sweeps must be offloaded via `cx.background_executor().spawn()`. The main GPUI thread interacts solely with the pre-computed `ComputedAttributeStream` cache using deferred execution (`cx.defer()`) on mouse-up events.

---

## 3. The Four Uber Signal Archetypes

### Archetype 1: Trend & Location (`OtlTaTrendNode`)
* **Mathematical Role:** First-order filtering to extract central tendency, trend, or location.
* **UI/Visual Context:** Rendered directly overlaid on the main asset price pane.
* **Input Ports:** `source_stream` (f64 vector, defaults to asset close).
* **Allowed Algorithms:** `sma`, `ema`, `wma`, `hma`, `tema`.
* **Hyperparameters:** `period: int` (default: 14).
* **Output Ports:** `result` (Single vector stream).
* **Dynamic OTL Template:** `ta::sma(input, {period})`

### Archetype 2: Risk & Dispersion (`OtlTaVolatilityNode`)
* **Mathematical Role:** Second-order filtering to capture asset risk, variance, standard deviation, and rate of price expansion.
* **UI/Visual Context:** Plotted as standard deviation bands encapsulating price, or as an auxiliary indicator stream.
* **Input Ports:** `source_stream` (f64 vector).
* **Allowed Algorithms:** `stddev`, `variance`, `atr` (Average True Range), `historical_volatility`.
* **Hyperparameters:** `period: int` (default: 20), `annualization_factor: float` (default: 252.0).
* **Output Ports:** `result` (Single vector stream).
* **Dynamic OTL Template:** `ta::volatility(input, {period})` or `ta::stddev(input, {period})`

### Archetype 3: Oscillators (`OtlTaOscillatorNode`)
* **Mathematical Role:** Normalizing price or momentum within bounded limits ($[0, 100]$ or $[-1, 1]$) to identify extreme structural thresholds.
* **UI/Visual Context:** Rendered in an isolated sub-pane directly below the main asset canvas.
* **Input Ports:** `source_stream` (f64 vector).
* **Allowed Algorithms:** `rsi`, `cci`, `stochastic`, `roc`, `macd`.
* **Hyperparameters:** `period: int` (default: 14), `signal_period: int` (default: 9).
* **Output Ports:** `oscillator`, `signal_line` (Secondary named output acting as an OTL AOV).
* **Dynamic OTL Template:** `ta::rsi(input, {period})`

### Archetype 4: Channels & Bands (`OtlTaChannelNode`)
* **Mathematical Role:** Multi-point structural boundaries representing dual-sided price containment envelopes.
* **UI/Visual Context:** Overlaid directly on the asset pane as upper, middle, and lower lines.
* **Input Ports:** `source_stream` (f64 vector).
* **Allowed Algorithms:** `bollinger_bands`, `keltner_channels`, `donchian_channels`.
* **Hyperparameters:** `period: int` (default: 20), `multiplier: float` (default: 2.0).
* **Output Ports (Exposed via Named AOVs):** * `upper_band`
  * `basis_line`
  * `lower_band`
* **Dynamic OTL Template:** Uses generic internal array tuple parsing or separate mapped evaluations.

---

## 4. OpenUSD Schema Integration (`schema.usda`)
The OpenUSD layer represents the nodes abstractly using a uniform schema layout. No raw individual indicators should be declared at the USD layer level.

```usd
class "OtlTaUberSignal" (
    doc = "Unified technical analysis node wrapping background VectorTA primitives."
) {
    # Core Archetype Switch
    uniform token info:archetype = "trend" (
        allowedTokens = ["trend", "volatility", "oscillator", "channel"]
    )
    
    # Concrete Sub-Algorithm Target
    uniform token info:algorithm = "sma"
    
    # Uniform Hyperparameter Storage Slots
    int inputs:period = 14
    float inputs:multiplier = 2.0
    float inputs:annualization = 252.0
    
    # The generated/editable execution source string parsed by compiler.rs
    string inputs:script_src = "ta::sma(input, 14)"
    
    # Output ports
    custom vector outputs:result
}