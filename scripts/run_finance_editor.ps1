# Launch the MarketLab finance blueprint editor (WGPUI).
# Run from repo root: .\scripts\run_finance_editor.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$plugin = Join-Path $root "external\Plugin_Blueprints"
if (-not (Test-Path $plugin)) {
    Write-Host "Fetching external dependencies..."
    & (Join-Path $root "scripts\setup_pulsar_external.ps1")
}

cargo run --manifest-path crates/marketlab_finance_editor/Cargo.toml @args
