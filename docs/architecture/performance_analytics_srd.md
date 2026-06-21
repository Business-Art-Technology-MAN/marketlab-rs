# MarketLab SRD: Performance Analytics Node

**Status:** Approved (June 2026)  
**Track:** B — Finance editor (`marketlab_finance_editor` + `marketlab_blueprint_adapter`)  
**Reference libraries (research only):** [pyfolio](https://github.com/quantopian/pyfolio), [PerformanceAnalytics](https://github.com/braverock/performanceanalytics)  
**HTML/table UX reference:** [Plugin_DBTable](https://github.com/Far-Beyond-Pulsar/Plugin_DBTable)

---

## 1. Purpose

Add a **Performance Analytics** terminal sink node that consumes strategy wealth/return series from the finance graph, collects **upstream graph context** (assets → analytics → portfolios), and produces a **Rust-native tear sheet** of portfolio performance metrics and charts after compile/sweep.

Implementation is **Rust-native**. pyfolio and PerformanceAnalytics define the *shape* of a robust analytics product (metrics, panels, report layout); they are **not** runtime dependencies.

---

## 2. Goals

| Goal | Detail |
|------|--------|
| Flexible inputs | Accept wealth, returns, or signal series from assets, OTL/TA, or portfolio integrators |
| Graph context | Resolve lineage from wired inputs back through the graph (symbols, node labels, path) |
| Tear sheet v1 | Summary table, cumulative returns, underwater drawdown, rolling Sharpe & volatility |
| Benchmark (nice-to-have v1) | Strategy vs Buy & Hold; default benchmark rules when `benchmark` pin unwired |
| UI | Hydra compact summary + dock **Performance** tab (full report) |
| Export | HTML tear sheet + CSV stats table (v1) |
| Fan-out | Asset/portfolio outputs may wire to multiple downstream nodes |

---

## 3. Non-goals (later phases)

- Monthly returns heatmap, return distribution / QQ plots  
- Dedicated graph-lineage panel (context folded into summary metadata for v1)  
- Advanced PA metrics (VaR, CVaR, Omega, style analysis)  
- PDF export  
- Engine-integrated execution tier (v1 is **post-sweep only**)

---

## 4. Node taxonomy

| Field | Value |
|-------|--------|
| Display name | **Performance Analytics** |
| Graphy `node_type` | `marketlab.analytics.performance_analytics` |
| USD `typeName` | `PerformanceAnalytics` |
| Palette category | **Reporting** (new) |
| Default prim path | `/MarketLab/Reporting/{name}` |
| Node role | **Terminal sink** — inputs only, no output pins |
| Header tint | Amber/gold (distinct from universe green, analytics blue, portfolio violet) |

### 4.1 Input pins

| Pin | Id | Behavior |
|-----|-----|----------|
| Primary series | `series_0`, `series_1`, … | Dynamic: **connected + 1 spare** (same pattern as Portfolio Integrator) |
| Benchmark (optional) | `benchmark` | Single optional wired benchmark series |

USD relationship for wired series: `inputs:series`  
USD relationship for benchmark: `inputs:benchmark`

### 4.2 Node properties

| Property | Default | Purpose |
|----------|---------|---------|
| `name` | `"Performance Report"` | Header, stage tree, export filename stem |
| `risk_free_rate` | `0.0` | Annualized risk-free rate for Sharpe/Sortino |
| `rolling_window` | `63` | Rolling Sharpe/volatility window (trading days when daily) |
| `benchmark_mode` | `auto` | `auto` \| `wired` \| `symbol` |
| `benchmark_symbol` | `SPY` | Used when `benchmark_mode = symbol` and no wired benchmark |

---

## 5. Input & benchmark rules

### 5.1 Primary series

The node accepts **any compatible `MarketLabSignalSeries`** upstream:

- Financial Asset `close`
- OTL/TA `result` (and named outputs where wired)
- Portfolio Integrator `wealth`

Multiple series pins allow reporting on blended or comparative inputs; v1 tear sheet focuses on the **primary wired series** (`series_0` after compaction).

### 5.2 Graph-collected context

On post-sweep build, walk **upstream** from the Performance Analytics node:

- Collect Financial Asset symbols, analytics labels, portfolio names along paths
- Attach metadata to the report (summary table footer / HTML header)
- Used for export titling and Hydra summary strip

### 5.3 Buy & Hold benchmark (nice-to-have v1)

When benchmark comparison is enabled and no `benchmark` pin is wired:

1. **First upstream Financial Asset** on the graph path → buy & hold that asset’s close series  
2. Else **equal-weight** blend of all Financial Assets feeding the report node

Calendar alignment uses the same timeline as the strategy series.

### 5.4 Calendar-aware returns

Infer bar frequency from **asset CSV dates** (not raw bar index):

- Annualize Sharpe, CAGR, volatility using detected periods-per-year  
- Fallback: 252 trading days/year when dates unavailable

---

## 6. Compile & execution model

**Post-sweep only (approved).**

1. User compiles finance graph → `StageGraphSnapshot` → `FinanceSweepResult`
2. Adapter walks graph for each `PerformanceAnalytics` node
3. Resolve wired series from sweep caches (`portfolios`, `analytics_signals`, asset previews)
4. Compute `FinancePerformanceReport` per node
5. Cache on `BlueprintEditorPanel` (`last_finance_performance_by_node`)
6. Hydra + Performance dock + export read from cache

No new `ExecutionNode` tier in `pulsar_marketlab_core` for v1.

---

## 7. Tear sheet panels

### 7.1 V1 required

| # | Panel | Metrics / visuals |
|---|--------|-------------------|
| 1 | **Summary stats table** | Total return, CAGR, ann. volatility, Sharpe, Sortino, max drawdown, Calmar, win rate, best/worst period |
| 2 | **Cumulative returns** | Strategy wealth or cumulative return index |
| 3 | **Underwater / drawdown** | Drawdown from running peak |
| 4 | **Rolling Sharpe & volatility** | Window = `rolling_window` property |

### 7.2 V1 nice-to-have

| # | Panel | Notes |
|---|--------|-------|
| 7 | **Benchmark comparison** | Cumulative strategy vs Buy & Hold; alpha/beta, capture ratios when benchmark resolved |

### 7.3 Later

- Monthly returns heatmap  
- Return distribution histogram  
- Dedicated lineage diagram panel  

---

## 8. UI surfacing (approved: option D)

| Surface | Content |
|---------|---------|
| **Hydra viewport** | Compact summary: stats table (top rows) + mini cumulative chart when Performance Analytics is viewport selection |
| **Dock tab “Performance”** | Full scrollable tear sheet (panels 1–4, + benchmark when available) |
| **Export** | **HTML** full tear sheet (self-contained, chart SVGs embedded); **CSV** summary stats table |

HTML layout follows tear-sheet section order; tabular sections styled with reference to Plugin_DBTable density/readability patterns.

---

## 9. Wiring constraints

- **Output fan-out:** Data outputs (asset `close`, portfolio `wealth`, analytics `result`) may connect to **multiple** downstream nodes.
- **Input fan-in:** Each input pin accepts **one** wire (standard EventGraph data-input rule).
- Performance Analytics is a **sink**; it does not block portfolio or asset outputs from feeding other nodes.

---

## 10. OpenUSD schema

Add to `crates/pulsar_marketlab_core/resources/usd/schema.usda`:

```usd
class "PerformanceAnalytics" (
    inherits = </Typed>
    doc = "Terminal performance tear-sheet sink; post-sweep reporting only."
) {
    string inputs:name = "Performance Report"
    double inputs:risk_free_rate = 0.0
    int inputs:rolling_window = 63
    token inputs:benchmark_mode = "auto" (
        allowedTokens = ["auto", "wired", "symbol"]
    )
    token inputs:benchmark_symbol = "SPY"
    rel inputs:series
    rel inputs:benchmark
}
```

---

## 11. Implementation map

| Layer | Crate / path | Responsibility |
|-------|----------------|----------------|
| Metrics | `marketlab_blueprint_adapter/src/performance_analytics.rs` | Returns, drawdown, rolling stats, benchmark compare |
| Report build | `marketlab_blueprint_adapter/src/performance_report.rs` | Graph walk, series resolve, `FinancePerformanceReport` |
| Node catalog | `metadata.rs`, `types.rs`, `blueprint.rs` | Registration, labels, pins |
| Persistence | `usd_persistence.rs`, `snapshot.rs`, `stage_tree.rs` | Round-trip, prim paths |
| Pin sync | `Plugin_Blueprints/.../finance_performance_pins.rs` | Dynamic `series_N` pins |
| Hydra | `finance_hydra_viewport.rs` | Compact preview |
| Dock | `FinancePerformancePanel` + `finance_performance_report.rs` | Full tear sheet |
| Export | `finance_performance_export.rs` | HTML + CSV writers |
| Compile hook | `compiler.rs`, `panel.rs` | Rebuild cache after sweep |

---

## 12. Acceptance criteria (v1)

- [ ] Performance Analytics appears in **Reporting** palette with one spare `series_0` pin + optional `benchmark`
- [ ] Wiring asset and/or portfolio to report + other nodes simultaneously works
- [ ] Compile populates report cache; Hydra shows compact summary for selected report node
- [ ] Performance dock tab shows panels 1–4
- [ ] Export HTML opens in browser with embedded charts; Export CSV downloads stats
- [ ] Calendar-aware annualization when CSV dates present
- [ ] Default Buy & Hold: first upstream asset, else equal-weight assets
- [ ] USD export/import preserves node properties and series wiring

---

## 13. Decision log

| # | Question | Decision |
|---|----------|----------|
| 1 | Inputs | Flexible series (B) + optional benchmark/risk-free (C); collect graph lineage context |
| 2 | Node role | Terminal sink; fan-out from upstream outputs allowed |
| 3 | Implementation | Rust-native; pyfolio/PA are research references only |
| 4 | V1 panels | 1–4 required; benchmark compare nice-to-have |
| 5 | Time axis | Calendar-aware from CSV; Buy & Hold = 1st upstream asset else equal-weight; risk-free default 0% |
| 6 | Taxonomy | Approved PerformanceAnalytics naming scheme |
| 7 | UI | Hydra compact + dock full + HTML/CSV export |
| 8 | Compile | Post-sweep only |
