# Plugin_Blueprints finance patches

`external/Plugin_Blueprints` is gitignored. After cloning upstream, apply the MarketLab finance spike edits below (or copy from a machine that already has them).

## Setup

```powershell
.\scripts\setup_pulsar_external.ps1
```

Then apply finance-mode changes to `external/Plugin_Blueprints` (see file list). Build:

```powershell
cargo run --manifest-path crates/marketlab_finance_editor/Cargo.toml
# or (same graph, external manifest)
cargo run --manifest-path external/Plugin_Blueprints/Cargo.toml --example standalone_finance
```

## Finance-mode file touch list

| Area | Files |
|------|--------|
| Compile mode | `src/core/types.rs` — `CompileMode::MarketLabFinance` |
| Compile + sweep | `src/features/compilation/compiler.rs` — `compile_to_finance_snapshot()`, sweep after compile |
| Panel state | `src/editor/panel.rs` — `last_finance_sweep`, `last_finance_portfolio_by_node` |
| Workspace layout | `src/editor/workspace.rs` — wealth chart bottom dock in finance mode |
| Panels | `src/editor/workspace_panels.rs` — finance property inputs, `FinanceWealthChartPanel` |
| Properties UI | `src/ui_components/properties.rs` — finance editors, sweep banner |
| Wealth chart | `src/ui_components/finance_wealth_chart.rs` |
| Pin compatibility | `src/core/types.rs` — `finance_data_types_compatible` in `is_compatible_with` |
| Definitions | `src/core/definitions.rs` — merge finance metadata, skip PBGC type registration for finance nodes |
| Toolbar | `src/editor/toolbar.rs` — finance compile mode label |
| Example | `examples/standalone_finance.rs` |
| Cursor fix | `src/rendering/input.rs`, `workspace_panels.rs` — `pending_cursor` pattern (see `plugin_blueprints_wgpui_cursor.patch`) |

## Workspace crates

- `crates/marketlab_blueprint_adapter` — Graphy → `StageGraphSnapshot` → engine sweep (in git)
- `crates/marketlab_finance_editor` — first-party WGPUI host binary (`cargo run -p marketlab_finance_editor`)

## Dependency path

`external/Plugin_Blueprints/Cargo.toml` must include:

```toml
marketlab_blueprint_adapter = { path = "../../crates/marketlab_blueprint_adapter" }
```

Pinned `graphy` rev must match the adapter crate.
