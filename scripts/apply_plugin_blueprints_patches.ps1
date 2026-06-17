# Apply MarketLab finance patches to external/Plugin_Blueprints.
# Run from repo root after setup_pulsar_external.ps1:
#   .\scripts\apply_plugin_blueprints_patches.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$pluginRoot = Join-Path $root "external\Plugin_Blueprints"
$patch = Join-Path $root "scripts\patches\plugin_blueprints_marketlab_finance.patch"

if (-not (Test-Path $pluginRoot)) {
    Write-Error "Plugin_Blueprints not found. Run .\scripts\setup_pulsar_external.ps1 first."
}

if (-not (Test-Path $patch)) {
    Write-Error "Patch file missing: $patch"
}

Push-Location $pluginRoot
try {
  # Discard prior local edits so the patch applies cleanly on re-run.
  git checkout -- .
  git clean -fd --exclude=.cargo

  git apply --check $patch
  if ($LASTEXITCODE -ne 0) {
    Write-Error "Patch does not apply cleanly. Reset Plugin_Blueprints or refresh the patch."
  }
  git apply $patch
  Write-Host "Applied finance editor patch to Plugin_Blueprints."
} finally {
  Pop-Location
}

$pbpCargo = Join-Path $pluginRoot ".cargo"
$configTemplate = Join-Path $root "scripts\patches\plugin_blueprints_cargo_config.toml"
if (Test-Path $configTemplate) {
    New-Item -ItemType Directory -Force -Path $pbpCargo | Out-Null
    Copy-Item $configTemplate (Join-Path $pbpCargo "config.toml") -Force
    Write-Host "Installed shared target-dir config for Plugin_Blueprints"
}

Write-Host "Finance editor: cargo run -p marketlab_finance_editor"
