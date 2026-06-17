# Finance editor setup

This folder documents how to run the MarketLab finance blueprint editor on your machine.

## What you get

- A node-graph editor for finance workflows (assets → TA/OTL → portfolio)
- **Compile** runs a backtest sweep and shows results in the right panel
- **Wealth Chart** tab (bottom dock) plots portfolio wealth over time
- Sample price data for SPY, QQQ, IWM, and GLD ships with the repo — leave **CSV path** empty on asset nodes

## Quick start

1. Clone this repo and install Rust if needed.

2. Fetch the blueprint UI dependency (one-time):

   ```powershell
   .\scripts\setup_pulsar_external.ps1
   ```

3. Apply finance UI patches (automatic if you used setup script):

   ```powershell
   .\scripts\apply_plugin_blueprints_patches.ps1
   ```

   Or re-run setup (clone + patch + shared target config):

   ```powershell
   .\scripts\setup_pulsar_external.ps1
   ```

4. Run the editor:

   ```powershell
   .\scripts\run_finance_editor.ps1
   ```

   Or directly:

   ```powershell
   cargo run --manifest-path crates/marketlab_finance_editor/Cargo.toml
   ```

5. In the editor: add a **Financial Asset** (symbol `SPY`), wire it to a **Portfolio Integrator**, click **Compile**. Check **Sweep Results** on the right and the **Wealth Chart** tab at the bottom.

## Tips

- **Symbol** — ticker name, e.g. `SPY`. Used to find bundled CSV at `crates/pulsar_marketlab/data/{SYMBOL}.csv`.
- **CSV path** — leave blank for bundled samples. Only set this if you have your own CSV file on disk (full path, not just the ticker).
- **Disk space** — builds share one `target/` folder. To reclaim space after experimenting: `.\scripts\clean_build_artifacts.ps1`

## Regenerate sample CSVs

Bundled OHLC files are ~252 trading days of synthetic data for local testing:

```powershell
python scripts/generate_bundled_sample_csvs.py
```

## Finance UI patches (external folder)

Tracked patch: `scripts/patches/plugin_blueprints_marketlab_finance.patch`

Apply after `setup_pulsar_external.ps1`:

```powershell
.\scripts\apply_plugin_blueprints_patches.ps1
```

To refresh the patch after editing `external/Plugin_Blueprints`:

```powershell
cd external/Plugin_Blueprints
git add -A
git diff --cached HEAD > ../../scripts/patches/plugin_blueprints_marketlab_finance.patch
```

| Area | Files |
|------|--------|
| Compile mode | `src/core/types.rs` — `CompileMode::MarketLabFinance` |
| Compile + sweep | `src/features/compilation/compiler.rs` |
| Panel state | `src/editor/panel.rs` |
| Workspace layout | `src/editor/workspace.rs` |
| Panels + property flush | `src/editor/workspace_panels.rs` |
| Properties UI | `src/ui_components/properties.rs` |
| Wealth chart | `src/ui_components/finance_wealth_chart.rs` |
| Save / Open | `src/io/save_load.rs`, `src/editor/toolbar.rs` |
| Pin compatibility | `src/core/types.rs` |
| Definitions | `src/core/definitions.rs` |
| Example | `examples/standalone_finance.rs` |

`external/Plugin_Blueprints/Cargo.toml` needs:

```toml
marketlab_blueprint_adapter = { path = "../../crates/marketlab_blueprint_adapter" }
```

## Crates in this repo

- `crates/marketlab_blueprint_adapter` — converts the graph to engine format and runs sweeps
- `crates/marketlab_finance_editor` — standalone editor binary (recommended entry point)
