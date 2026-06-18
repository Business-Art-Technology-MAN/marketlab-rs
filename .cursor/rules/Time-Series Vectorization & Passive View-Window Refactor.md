# System Requirements Document (SRD)
## Time-Series Vectorization & Passive View-Window Refactor

**Document Version:** 2.0.0  
**Target Environment:** `marketlab-rs` / `crates/pulsar_marketlab_core` & `crates/pulsar_marketlab`  
**Context:** Structural execution architecture overhaul for Cursor coding agent  

---

## 1. Executive Summary & Core Intent

This document specifies a foundational architectural shift within the MarketLab execution pipeline. The legacy, stateful, frame-by-frame "playhead sweep" loop is deprecated. Iterative step-by-step model generation creates extreme layout fragmentation, prevents CPU cache optimization, and introduces latency during UI visualization updates.

The core calculation framework is transitioned to an ahead-of-time **Time-Series Vectorization** engine. The `GraphEngine` and `TerminalIntegrator` must execute across the entire historical range all at once up to the terminal data point, storing continuous arrays of performance metrics. The concept of a "playhead" is removed from the data-engineering layer entirely and refactored into a passive **UI View-Window Controller** that simply slices into pre-computed memory buffers.

---

## 2. Updated Architectural Model

The simulation and playback responsibilities are completely decoupled into a data-parallel calculation phase and a lazy index-lookup view layer:

```
┌─────────────────────────────────────────────────────────────────┐
│  Phase A — Vectorized Execution (background, on invalidation)   │
│  MarketLabGraphEngine::execute_timeline(asset_vectors, len)     │
│  → ComputedAttributeStream[] + PortfolioIntegrationResult map     │
│  Cached on workspace: graph_engine_streams, portfolio_results   │
└────────────────────────────┬────────────────────────────────────┘
                             │ one-shot write
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase B — Passive View Window (UI thread, on scrub / slider)     │
│  playhead_current = view index into pre-computed buffers          │
│  sync_view_window() → slice diagnostics + inspector at index      │
│  No TaExecutionBridge replay, no frame-by-frame engine sweep      │
└─────────────────────────────────────────────────────────────────┘
```

| Layer | Responsibility | Trigger |
|-------|----------------|---------|
| `GraphEngine` / `TerminalIntegrator` | Full-range vectorized execution | Graph wiring change, CSV reload, OTL compile |
| `playhead_current` (view index) | UI scrubber / chart line position | Mouse drag, slider, CSV playback tick |
| `sync_view_window` | Slice cached streams + portfolio matrices | Any view-index change |
| Legacy `evaluate_portfolio_at_playhead` | Fallback when no vectorized cache | Graph without engine sweep results |

---

## 3. Implementation Requirements

### 3.1 Core Engine (`pulsar_marketlab_core`)

- **`MarketLabGraphEngine::execute_timeline`** must remain the single authoritative vectorized execution entry point.
- Output **`TimelineExecutionResult`** carries:
  - `streams: Vec<ComputedAttributeStream>` — per-prim, per-attribute bar-indexed samples
  - `portfolio_results: HashMap<String, PortfolioIntegrationResult>` — wealth series + tracking matrix
- Playhead / view index must **not** appear in the core engine API.

### 3.2 Workspace Cache (`pulsar_marketlab`)

- On timeline result apply, persist:
  - `graph_engine_streams` — local mirror of computed attribute streams
  - `graph_engine_portfolio_results` — integration output keyed by portfolio prim path
  - `portfolio_timeline_cache`, `portfolio_diagnostics_cache`, `portfolio_ledger_cache`
- **`graph_engine_vectorized_active()`** — true when `graph_engine_streams` is non-empty.
- **`graph_engine_analytics_active()`** — true when portfolio integration results exist.

### 3.3 Passive View Window (`sync_view_window`)

Replace playhead-triggered engine replay with synchronous index lookups:

1. `sync_playhead_time_from_index()` — derive stage time label from OHLC bar date (UI only).
2. `synchronize_inspector_view()` — when vectorized cache present, build ledger rows via `build_inspector_rows_from_streams` (no `refresh_ta_samples_at_playhead`).
3. When portfolio results present: `refresh_portfolio_diagnostics_cache()` slices wealth / tracking matrix through `playhead_current`.
4. `publish_metrics_telemetry_bridge(cx)` — hydrate `MetricsTelemetryBridge` from sliced diagnostics.
5. **Do not** call `spawn_playhead_evaluation_async` when `graph_engine_vectorized_active()`.

### 3.4 UI Integration

- OHLC chart playhead line and render-viewport slider remain the view-window controller.
- Scrub handlers call `sync_view_window` instead of async evaluation.
- Render viewport label: **"View window index"** (not "Evaluation coordinate").
- Background invalidation worker in `graph_engine.rs` unchanged in behavior — still calls `execute_timeline` on cache dirty.

### 3.5 Deprecation

- **`evaluate_portfolio_at_playhead`** — legacy fallback only; marked superseded by graph-engine path.
- **`spawn_playhead_evaluation_async`** — early-returns into `sync_view_window` when vectorized cache is warm.
- Frame-by-frame CSV playback feeder must not drive TA recomputation when vectorized results exist.

---

## 4. Acceptance Criteria

1. Scrubbing the playhead with a wired asset → signal → portfolio graph produces **no background executor work** (only synchronous cache slices).
2. Sharpe / MDD / return metrics at bar *N* match the subset `wealth_series[0..=N]` from the vectorized sweep — no drift across repeated scrubs.
3. Inspector ledger rows at bar *N* match `ComputedAttributeStream` samples at index *N*.
4. Graph or CSV invalidation triggers exactly one background `execute_timeline`; subsequent scrubs reuse the cache until the next invalidation.
5. Workspace without graph-engine results continues to function via the legacy playhead replay path.

---

## 5. Key File Map

| File | Role |
|------|------|
| `crates/pulsar_marketlab_core/src/orchestration/engine.rs` | Vectorized `execute_timeline` |
| `crates/pulsar_marketlab_ui/src/workspace/graph_engine.rs` | Background compile + invalidation worker |
| `crates/pulsar_marketlab/src/workspace_state.rs` | `sync_view_window`, stream cache, inspector slice helpers |
| `crates/pulsar_marketlab/src/ui/mod.rs` | `apply_graph_engine_timeline_result` → `sync_view_window` |
| `crates/pulsar_marketlab/src/ui/timeline_controls.rs` | Chart scrub → `sync_view_window` |
| `crates/pulsar_marketlab/src/ui/telemetry_bridge.rs` | Metrics slice at view index |
| `crates/pulsar_marketlab/src/portfolio_analytics.rs` | `build_portfolio_diagnostics_from_integration` windowed through playhead |
