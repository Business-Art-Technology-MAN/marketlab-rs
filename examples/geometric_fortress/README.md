# Geometric Fortress (Pure Geometric Beta)

OTL scripts ported from [The Geometric Investing: Solving the "Everything Crash" with New Math](https://substack.com/home/post/p-184893648) (Agus Sudjianto, Jan 2026).

These files implement the article's **three geometric sensors** and the **monthly rebalance decision tree** using MarketLab `ga::` operators (vector-ga / clifford-backed).

## Universe (Yahoo-style tickers)

| Role | Tickers |
|------|---------|
| Regime (sector wedge volume) | XLK, XLF, XLV, XLE, XLI, XLY, XLP, XLB, XLU |
| Offense | SPY |
| Defense candidates | TLT, GLD |
| Cash proxy | SHV |

Load daily **adjusted close** price columns for each ticker into the stage graph (FinanceDatabase / CSV ingest).

## Strategy logic (article)

On each **month-end bar** (`ga::month_end()` ≈ 21 trading days):

1. **PANIC** — wedge volume &lt; 15th percentile (252-day rolling) → **100% SHV**
2. **DEFENSE** — wedge volume &lt; 40th percentile → pick **TLT or GLD** with highest **bivector beta** vs SPY, but only if **orientation(200) &gt; 0**; else **SHV**
3. **OFFENSE** — healthy volume → **SPY** if SPY orientation &gt; 0, else **SHV**

Between month-ends, holdings are unchanged (Python `current_holdings` carry-forward). In MarketLab, forward-fill allocator weights or gate updates with `ga::month_end()`.

## Graph wiring

```
[9 × Sector assets] ──inputs:constituents──► [regime_wedge_volume]
                                                      │
                                                      ▼
                                            [regime_thresholds] (panic + defense)
                                                      │
[SPY] ──inputs:underlying──► [orientation_spy] ─────┼──► [fortress_allocator]
[TLT] ──inputs:sources──────► [orientation_tlt]      │         │
      ──inputs:sources──────► [bivector_tlt]         │         ├──► PortfolioIntegrator
[GLD] ──inputs:sources──────► [orientation_gld]      │              (SPY, TLT, GLD, SHV legs)
      ──inputs:sources──────► [bivector_gld]         │
[SPY] ──inputs:sources──────► (market for beta) ─────┘
```

### Per-node `inputs:script_src`

Copy the body from the matching `.otl` file into each `OtlOperator` / `OtlTaUberSignal` node, or reference the file in your editor.

| File | Node role | Wires |
|------|-----------|-------|
| `sensors/regime_wedge_volume.otl` | Regime sensor | `inputs:constituents` ← 9 sector price streams |
| `sensors/regime_thresholds.otl` | Adaptive quantiles | primary ← wedge volume output |
| `sensors/orientation_200.otl` | Trend sensor | primary ← asset close; reuse per asset |
| `sensors/bivector_beta_60.otl` | Haven orthogonality | primary ← asset returns; `market` ← SPY |
| `strategy/fortress_weight_spy.otl` | SPY leg weight | sensor streams |
| `strategy/fortress_weight_tlt.otl` | TLT leg weight | sensor streams |
| `strategy/fortress_weight_gld.otl` | GLD leg weight | sensor streams |
| `strategy/fortress_weight_cash.otl` | SHV leg weight | sensor streams |
| `strategy/fortress_signal.otl` | Simplified offense-only signal | sensor streams |
| `strategy/fortress_portfolio.otl` | Portfolio tier shell | `inputs:sources` ← 4 asset legs |

## Parameters (match Python)

| Parameter | Python | OTL |
|-----------|--------|-----|
| Wedge / beta window | 60 | `ga::wedge_volume(60)`, `ga::bivector_beta(..., 60)` |
| Orientation window | 200 | `ga::orientation(..., 200)` |
| Quantile lookback | 252 | `ga::rolling_quantile(..., 252, q)` |
| Panic quantile | 0.15 | `q = 0.15` |
| Defense quantile | 0.40 | `q = 0.40` |
| Rebalance | calendar month-end | `ga::month_end()` |

## Quick compile check

```bash
cargo test -p pulsar_marketlab_core --test geometric_fortress_spec
cargo test -p pulsar_marketlab geometric_fortress_otl_scripts_compile -- --nocapture
```

## Limitations

- **Monthly hold**: vector OTL is bar-by-bar; forward-fill the four leg weight streams between `ga::month_end()` pulses in the allocator tier.
- **NNLS sector gravity** (`calculate_dynamic_weights` in Python) is available as `ga::nnls_weights(60)` but is **not** used in the article's Pure Fortress decision tree; see `sensors/nnls_sector_gravity.otl` if you extend the strategy.
- Full backtest parity requires historical data from 2010+ and the graph topology above; golden CSV comparison is not yet bundled.
