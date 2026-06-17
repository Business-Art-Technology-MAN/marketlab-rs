# Reclaim disk space from Rust build caches and compile debug spill files.
# Run from repo root: .\scripts\clean_build_artifacts.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Cleaning shared Cargo target..."
cargo clean

$extraTargets = @(
    "external\Plugin_Blueprints\target",
    "crates\marketlab_finance_editor\target"
)
foreach ($path in $extraTargets) {
    if (Test-Path $path) {
        Write-Host "Removing legacy target: $path"
        Remove-Item $path -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$spill = @("finance_stage_snapshot.txt", "blueprint_graph_debug.json")
foreach ($file in $spill) {
    if (Test-Path $file) {
        Remove-Item $file -Force
        Write-Host "Removed $file"
    }
}

Write-Host "Done. Rebuild with: cargo check -p pulsar_marketlab"
