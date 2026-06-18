# UI Execution, Inspector, and DCC Styling Audit

**Scope:** Node canvas, Param Inspector, USD Stage Composer, downstream graph execution.

## Execution / invalidation contract

| User action | Expected downstream behavior | Entry point |
|-------------|------------------------------|-------------|
| Wire connect/disconnect | Recompose USDA + timeline sweep | `sync_pipeline_graph` → `publish_canvas_to_usd_stage` |
| OTL script (inspector / canvas / editor) | Port resync + recompose + sweep | `commit_otl_script`, `otl_node_params`, OTL editor |
| TA hyperparameter | Deferred publish + sweep | `commit_ta_uber_parameter_change` |
| Stage **Active** checkbox | Overlay + invalidate cache + sweep | `WorkspaceContext::modify_attribute("inputs:active")` |
| Asset CSV path | OHLC reload + sweep | `reload_asset_chart_from_path` / `request_graph_engine_timeline_refresh` |
| Playhead scrub | View slice only (no recompile) | `sync_view_window` |

**Selection sync (no recompile):** `select_stage_path` ↔ `WorkspaceContext.set_selected_path` via `ui_selection_generation`. Host observer calls `sync_canvas_selection_from_context`, `sync_inspector_from_selection`, `sync_view_window`.

## Fixes applied (2026-06)

1. **Param Inspector OTL** — `commit_otl_script` now calls `sync_otl_shader_ports_from_script`, clears stale canvas param inputs, and runs `sync_pipeline_graph` + `sync_view_window`.
2. **Inspector reset on selection** — `sync_inspector_from_selection` clears `otl_shader_param_inputs` and refreshes the view window.
3. **Pipeline overview** — Shown even when a prim is selected (was hidden whenever `selected_path` was set).
4. **Stage overlay durability** — `publish_canvas_to_usd_stage` snapshots `RuntimeOverlaySnapshot` (active flags, attributes, relationships, edit target) and restores after USDA recompose so **Active** toggles survive canvas edits.
5. **DCC inspector inputs** — `dcc_multiline_input` / `dcc_singleline_input` in `node_inline_controls.rs`; Param Inspector OTL field uses recessed chrome (not default `Input::new`).
6. **AOV label color** — Uses `theme::SOCKET_AOV` instead of hardcoded cyan.
7. **Stage active checkbox** — `stop_propagation` on click so row selection does not swallow the toggle.

## DCC styling status

| Area | Status |
|------|--------|
| Node canvas shell, grid, capsules | Themed via `theme.rs` + `workspace/node_canvas.rs` |
| Inline node controls | `node_inline_controls.rs` (reference) |
| Param Inspector / Stage Composer | Themed rows; inspector inputs now recessed |
| Menu bar, portfolio ledger, OHLC charts | Still use parallel zinc/Bloomberg palette — migrate to `theme.rs` |
| Raw `Button::new` without `.custom` | `menu_bar.rs`, `portfolio_integrator_ledger.rs` |

**Rule:** New UI must use `theme::*` and `node_inline_controls` / `pane_shell` helpers — not raw GPUI defaults or ad-hoc `rgb(0x…)`.

## Remaining gaps (follow-up)

- **Wire colors** in `pulsar_marketlab/src/ui/node_canvas.rs` — duplicate `socket_color` literals; centralize in `theme` or `socket_color()`.
- **Global matrix spreadsheet** in `sidebar_inspector.rs` — `render_spreadsheet_inspector` is dead; live pane is `param_inspector`.
- **OTL inspector debounce** — Per-keystroke `sync_pipeline_graph` is correct but heavy; optional debounce for large graphs.
- **compose `inputs:active`** — USDA compose still writes `true`; runtime overlay wins for execution via `prim_active()`.

## Verification checklist

- [ ] Select node on canvas → Param Inspector title + OTL field match node.
- [ ] Edit OTL in inspector → ports/wires update; graph recompiles; streams refresh at playhead.
- [ ] Toggle **Active** on Stage row → prim excluded from sweep when false; survives adding a wire.
- [ ] Select row in Stage → canvas selection + inspector extensions update.
- [ ] AOV toggles in inspector → extra output ports + recompose.
