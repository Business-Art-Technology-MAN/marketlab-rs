# Timeline Double-Buffer & Engine Isolation SRD (v7.0)

**Status:** Implemented (2026-06-04)

## Goal

Keep GPUI `render()` off the graph-engine hot path by building all chart, ledger, diagnostics, and telemetry caches on a background thread after `execute_timeline`, then swapping a single `Arc<TimelineUiSnapshot>` (alias: `GraphUiSnapshot`) on the UI thread.

## Architecture

```
Background worker (graph_engine.rs)
  compile_from_canvas → execute_timeline → build_graph_ui_snapshot
       ↓
UI thread: apply_graph_ui_snapshot(Arc) → sync_view_window → cx.notify
       ↓
render(): ui_read_snapshot() only (no cache rebuild)
```

## Key types

| Type | Crate | Role |
|------|-------|------|
| `GraphUiSnapshot` / `TimelineUiSnapshot` | `pulsar_marketlab` | Immutable Buffer B |
| `GraphUiSnapshotBuildInput` | `pulsar_marketlab` | Frozen host inputs for off-thread build |
| `GraphEngineInvalidationHost::UiSnapshot` | `pulsar_marketlab_ui` | Associated type wired in `ui/mod.rs` |

## Interaction gating

During canvas drag / TA scrub, sweep results and snapshots are held in `pending_timeline_result` + `pending_ui_snapshot` until `on_pipeline_interaction_ended`.

## Verification

- TA slider drag: no `refresh_portfolio_wealth_chart_cache` on UI thread
- Inspector / node cards read `ui_read_snapshot()` accessors
- `cargo test --test perf_test` documents engine-only vs USD roundtrip split
