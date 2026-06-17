# Vendor Plugin_Blueprints + Graphy for Track B GUI spike.
# Run from repo root: .\scripts\setup_pulsar_external.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$external = Join-Path $root "external"

if (-not (Test-Path $external)) {
    New-Item -ItemType Directory -Path $external | Out-Null
}

function Ensure-Clone($name, $url) {
    $dest = Join-Path $external $name
    if (Test-Path $dest) {
        Write-Host "$name already present at $dest"
        return
    }
    Write-Host "Cloning $name..."
    git clone --depth 1 $url $dest
}

Ensure-Clone "Plugin_Blueprints" "https://github.com/Far-Beyond-Pulsar/Plugin_Blueprints.git"
Ensure-Clone "Graphy" "https://github.com/Far-Beyond-Pulsar/Graphy.git"

$applyPatches = Join-Path $root "scripts\apply_plugin_blueprints_patches.ps1"
if (Test-Path $applyPatches) {
    & $applyPatches
} else {
    Write-Warning "Patch script not found: $applyPatches — apply finance patches manually."
}

$pbpCargo = Join-Path $external "Plugin_Blueprints\.cargo"
$configTemplate = Join-Path $root "scripts\patches\plugin_blueprints_cargo_config.toml"
if (Test-Path $configTemplate) {
    New-Item -ItemType Directory -Force -Path $pbpCargo | Out-Null
    Copy-Item $configTemplate (Join-Path $pbpCargo "config.toml") -Force
    Write-Host "Installed shared target-dir config for Plugin_Blueprints"
}

Write-Host ""
Write-Host "Finance editor:"
Write-Host "  .\scripts\run_finance_editor.ps1"
Write-Host "  cargo run --manifest-path crates/marketlab_finance_editor/Cargo.toml"
Write-Host ""
Write-Host "Legacy spike example:"
Write-Host "  cd external/Plugin_Blueprints && cargo run --example standalone_finance"
Write-Host "Pin revs from Plugin_Blueprints/Cargo.toml before production vendoring."
