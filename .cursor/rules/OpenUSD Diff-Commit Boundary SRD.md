# OpenUSD Diff-Commit Boundary SRD (v9.0)

**Status:** Implemented (2026-06-04) — Phase A

## Principle

OpenUSD is the **interchange and persistence** format (import, export, save, cold hydrate). It is **not** the live interactive IR on every slider tick.

## Implemented (Phase A)

| Change type | USD | Engine |
|-------------|-----|--------|
| Wire / node topology | Debounced incremental or full compose | `invalidate_engine_topology_cache` → recompile |
| TA param / OTL body | **No USD write** (`schedule_ta_param_resweep`) | Re-sweep; tier cache cleared; canvas snapshot compile |
| Attribute-only incremental overlay | Overlay write; no topology invalidation | Re-sweep only |
| Save / Save As / Import | Full USDA compose | Hydrate on load |
| Session autosave | JSON every debounce; USDA every 30s checkpoint | — |

## Canvas-native compile path

- `build_stage_graph_snapshot_from_canvas` — in-memory USDA compose + single stage walk
- `MarketLabGraphEngine::compile_from_canvas` — engine entry without session disk `Stage::open`
- `stage_graph_snapshot_cache` keyed by `pipeline_graph.revision()`

## Deferred ledger

- `sync_workspace_ledger` removed from synchronous `finish_canvas_stage_publish`
- Debounced background rebuild keyed on `usd_commit_generation`

## Phase B (optional)

- Persistent in-memory `openusd::Stage` in `ManagedUsdStage` (eliminate per-query disk open)
- Topology tree from cached `StageGraphSnapshot` instead of live `Stage::open`

## Verification

```bash
cargo test --test perf_test -- --nocapture
```

Expect `perf_engine_canvas_direct` ≪ `perf_engine_usd_roundtrip` in debug builds.
