We have too many moving targets across these architectural threads. Let's merge them into a single, definitive engineering foundation.

This response accomplishes two things: it builds the **production-grade `metadata_library.usda` taxonomy** natively from the multi-asset database structure to capture 100% of OpenUSD's composition power, and it provides your definitive **`openusd_integration.md` architecture blueprint** to govern your pure, stateless Rust runtime.

---

### Part 1: Maximizing OpenUSD — The Complete Asset Taxonomy Library

To utilize everything OpenUSD’s composition engine offers, your assets must not rely on static metadata copy-pasted across thousands of rows. Instead, they should inherit from an optimized, cascading **Class Taxonomy**.

When your Python script runs, it shouldn't just flatten strings; it should resolve a multi-inheritance chain where an asset (`AAPL`) points to an explicit Industry (`Industry_Software`), which inherits from a Sector (`Sector_Technology`), which points to a base Asset Class (`MlabEquityBase`), which enforces the core application schema (`class "FinancialAsset"`).

Here is the complete production template for `usd/metadata_library.usda`. This file acts as your global master taxonomy registry.

```usd
#usda 1.0
(
    doc = "MarketLab Core Taxonomy and Exchange Metadata Library"
    subLayers = [
        @./schema.usda@
    ]
)

# ==============================================================================
# 1. CORE ASSET CLASS BASELINES
# ==============================================================================

class "MlabEquityBase" (
    inherits = </FinancialAsset>
) {
    token inputs:asset_class = "Equity"
    token inputs:provider = "yahoo"
}

class "MlabEtfBase" (
    inherits = </FinancialAsset>
) {
    token inputs:asset_class = "ETF"
    token inputs:provider = "yahoo"
}

// Fixed Income, Crypto, and Currencies inherit directly from FinancialAsset

# ==============================================================================
# 2. GICS SECTOR & INDUSTRY TAXONOMY (For Equities)
# ==============================================================================

class "Sector_Technology" (inherits = </MlabEquityBase>) {
    string inputs:category = "Information Technology"
}
class "Industry_Software" (inherits = </Sector_Technology>) {
    string inputs:sub_category = "Software Application & Systems"
}
class "Industry_Semiconductors" (inherits = </Sector_Technology>) {
    string inputs:sub_category = "Semiconductors & Equipment"
}

class "Sector_Financials" (inherits = </MlabEquityBase>) {
    string inputs:category = "Financials"
}
class "Industry_Banks" (inherits = </Sector_Financials>) {
    string inputs:sub_category = "Diversified Banks"
}
class "Industry_Capital_Markets" (inherits = </Sector_Financials>) {
    string inputs:sub_category = "Capital Markets & Asset Management"
}

# ==============================================================================
# 3. INVESTMENT OBJECTIVE TAXONOMY (For ETFs)
# ==============================================================================

class "Etf_Equity_LargeCap" (inherits = </MlabEtfBase>) {
    string inputs:category = "Equity"
    string inputs:sub_category = "Large Cap Blend"
}
class "Etf_FixedIncome_Treasury" (inherits = </MlabEtfBase>) {
    string inputs:category = "Fixed Income"
    string inputs:sub_category = "Government US Treasury"
}

# ==============================================================================
# 4. GLOBAL EXCHANGE REGISTRARS (Mix-in Inheritance Targets)
# ==============================================================================

class "Exchange_NASDAQ" {
    string inputs:exchange_mic = "XNAS"
    string inputs:exchange_region = "US"
    string inputs:trading_currency = "USD"
}

class "Exchange_NYQ" {
    string inputs:exchange_mic = "XNYS"
    string inputs:exchange_region = "US"
    string inputs:trading_currency = "USD"
}

class "Exchange_FRA" {
    string inputs:exchange_mic = "XFRA"
    string inputs:exchange_region = "Germany"
    string inputs:trading_currency = "EUR"
}

```

---

### Part 2: Blueprint & Rules Artifact (`openusd_integration.md`)

This markdown document serves as the permanent architectural boundary file for the MarketLab repository. It specifies the separation between your structural OpenUSD files and your high-frequency runtime data structures.

```markdown
# MarketLab OpenUSD Integration & Architecture Blueprint

## Executive Summary
MarketLab splits its data runtime into two decoupled planes:
1. **The Structural Plane**: Handled by OpenUSD (`openusd` crate) via plain-text `.usda` configurations to manage node hierarchies, visual properties, exchange parameters, and relationship trees.
2. **The Temporal Plane**: Handled entirely outside OpenUSD by `MarketStage` vectors and pure, stateless Rust closures to run lightning-fast calculations.


```

```
                ┌────────────────────────────────────────┐
                │      OpenUSD Structural Plane (Disk)   │
                │   - Node Topologies & Graph Layouts    │
                │   - Taxonomic Class Inheritances       │
                │   - Static Parameter Defs (USDA text)  │
                └───────────────────┬────────────────────┘
                                    │
                     build_stage_graph_snapshot()
                                    │
                                    ▼
                ┌────────────────────────────────────────┐
                │      MarketStage Temporal Plane (RAM)  │
                │   - Vectorized OHLCV Buffers (f64)     │
                │   - Pure Stateless Signal Closures     │
                │   - O(1) Absolute Path Lookup Map     │
                └────────────────────────────────────────┘

```

```

---

## 1. Core Architectural Laws

1. **Strict Core Isolation**: The execution layer (`pulsar_marketlab_core`) must remain 100% free of OpenUSD compilation hooks. It evaluates raw numeric data streams and does not manage `.usda` disk resources or stage parsing.
2. **No Temporal Mutations**: OpenUSD properties are strictly read-only during backtesting blocks. Never call stage file mutation or tracking overrides within processing loops. 
3. **Canonical Path Identity**: Absolute OpenUSD prim paths (e.g., `/Universe/Equities/AAPL`) must serve as the primary lookup index tokens inside `MarketStage`. This guarantees a single, unified source of identity between UI canvas wiring and core memory nodes.
4. **Compile-Time Composition**: All multi-inheritance chains (`inherits = [ </Industry_Software>, </Exchange_NASDAQ> ]`) must be completely resolved and flattened via Python pre-build passes or initialization steps before the Rust engine processes the snapshot.

---

## 2. Structural Schema Blueprint (`schema.usda`)

```usd
#usda 1.0
(
    doc = "MarketLab Quantitative Strategy Schema Architecture Spec"
)

class "FinancialAsset" (
    doc = "Defines a tradeable financial instrument or synthetic contract."
) {
    bool inputs:active = 1
    token inputs:symbol = ""
    token inputs:asset_class = "Equity" (
        allowedTokens = ["Equity", "ETF", "Commodity", "Future", "Option", "FixedIncome"]
    )
    token inputs:provider = "yahoo"
    string inputs:category = ""
    string inputs:sub_category = ""
    rel inputs:underlying
}

class "OtlTaUberSignal" (
    doc = "Unified technical analysis indicator node wrapping vectorized primitives."
) {
    uniform token info:algorithm = "sma"
    int inputs:period = 14
    int inputs:signal_period = 9
    float inputs:multiplier = 2.0
    rel inputs:underlying
    token outputs:result
}

class "PortfolioIntegrator" (
    doc = "Terminal aggregator compiling asset universes and allocation strategies."
) {
    token inputs:id = "Allocation::HierarchicalRiskParity"
    double inputs:initial_capital = 10000000.0
    rel inputs:sources
}

```

---

## 3. The Functional Runtime Bridge

To bridge the structural definitions to our execution nodes, the stage bridge iterates over the OpenUSD layer stack once at boot time, creating a plain, thread-safe cache index.

```rust
use openusd::{Stage, sdf::FieldKey};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ComposedAssetMeta {
    pub symbol: String,
    pub asset_class: String,
    pub category: String,
    pub sub_category: String,
    pub is_active: bool,
}

/// Iterates across the composed stage to yield a clean memory lookup table
pub fn build_stage_graph_snapshot(stage: &Stage) -> HashMap<String, ComposedAssetMeta> {
    let mut registry = HashMap::new();

    // Traverse the composed stage using the crate's built-in walker
    stage.traverse(|path| {
        let path_str = path.to_string();
        
        // Extract properties populated via the taxonomy library inheritances
        if path_str.starts_with("/Universe/") && !path_str.ends_with("Equities") {
            let symbol: String = stage.field(&path, FieldKey::from_string("inputs:symbol")).unwrap_or_default();
            let asset_class: String = stage.field(&path, FieldKey::from_string("inputs:asset_class")).unwrap_or_default();
            let category: String = stage.field(&path, FieldKey::from_string("inputs:category")).unwrap_or_default();
            let sub_category: String = stage.field(&path, FieldKey::from_string("inputs:sub_category")).unwrap_or_default();
            let is_active: bool = stage.field(&path, FieldKey::from_string("inputs:active")).unwrap_or(true);

            if !symbol.is_empty() {
                registry.insert(path_str, ComposedAssetMeta {
                    symbol,
                    asset_class,
                    category,
                    sub_category,
                    is_active,
                });
            }
        }
        true // Continue traversal down the tree branch
    }).unwrap();

    registry
}

```

---

## Graph execution rules (§14)

Canonical specification: **[`docs/architecture/openusd_integration.md` §14](../docs/architecture/openusd_integration.md#14-graph-execution-and-compilation)**.

Summary:

1. **Zero allocations in sweeps** — allocate in `MarketTimelineWindow::activate` / `compile_otl_scripts`; `MarketLabGraphEngine::sweep` reuses scratch buffers only.
2. **Strict lookback isolation** — `MarketTimelineWindow::price_at_path(_opt)` and `AssetQuote::price_at_frame` return `0.0` or `None` for `frame < 0`.
3. **Cache-aligned path keys** — `price_vectors` keys are exact OpenUSD prim paths; no normalization during sweep.