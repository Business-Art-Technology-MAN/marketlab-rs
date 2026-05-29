# MarketLab SRD - Phase B Pillar 1 (Part 3): Continuous-Time Execution Ledger
**Target Directory:** `crates/pulsar_marketlab/src/execution_engine/`
**Target Files:** `src/execution_engine/mod.rs` (Refactor), `src/workspace_state.rs` (Integration)

---

### 1. Objective
Refactor the Layer 2 execution engine and simulation account layers to store account balances, position states, and trade executions directly inside the `MarketStage` scene graph using continuous `f64` timestamps. Completely eliminate the legacy row-indexed tracking matrices and tier buckets.

---

### 2. Functional Requirements

#### A. Refactoring `SimulationAccountMatrix` to Stage Primitives
* **State Devolution:** Strip the internal `Array2<f64>` matrix completely out of the account tracking structures.
* **Stage Ownership Linkage:** Pass a mutable or thread-safe reference of `MarketStage` into the execution engine's transaction processors so mutations write directly to the global scene graph.
* **Initialization Injection:** When a simulation initializes, seed the initial cash balance at $t = 0.0$ by injecting a sample at `"/execution/portfolio/cash"` -> `"balance"`.

#### B. Dynamic Double-Entry Transaction Execution
When an analytical node closure emits a transaction signal at timestamp $t$, the execution matching engine must process mutations onto the stage according to strict double-entry mechanics:
1. **Cash Adjustment:** Read the current cash balance at $t$ using `resolve_attribute_at("/execution/portfolio/cash", "balance", t)`. Calculate the transactional impact (shares $\times$ execution price $+$ slippage/fees). Insert the new snapshot value exactly at timestamp $t$.
2. **Inventory Adjustment:** Read the existing share count at `"/execution/portfolio/positions/{ticker}"` -> `"shares"` at time $t$. Write the updated position quantity to that same attribute path at timestamp $t$.

#### C. Continuous Net Asset Value ($NAV$) Rollup Math
* **Dynamic Valuation Engine:** Rewrite `compute_portfolio_diagnostics` to calculate metrics ephemerally at playhead time $t$:
  $$NAV(t) = \text{CashBalance}(t) + \sum \left( \text{Shares}_{i}(t) \times \text{MarkPrice}_{i}(t) \right)$$
* **Lookup Mechanics:** * Extract $\text{CashBalance}(t)$ via the continuous forward-fill lookup from the cash path.
  * For each active asset primitive, extract the current shares quantity at $t$ and multiply it by the asset's mark-price at $t$ fetched from `"/assets/{ticker}/close"`.
* **Invariance:** Because all inputs leverage the `BTreeMap::range` lookup mechanism, $NAV(t)$ remains completely invariant and jitter-free unless a new execution sample is explicitly written at $t$ or the playhead shifts.

#### D. Complete Deprecation of Phase A Tier Enums
* **Dead Code Pruning:** Remove the legacy `TradingStage` struct definition, the `TierPrimitiveBucket` collections, and references to `Base`, `Signals`, and `Overrides` tiers from the codebase.
* **Namespace Enforcement:** Force all charts and inspector rows to populate their view state frames exclusively through unified slash-delimited path queries.