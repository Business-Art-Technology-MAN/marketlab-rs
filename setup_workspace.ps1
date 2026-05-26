# setup_workspace.ps1
$ErrorActionPreference = "Stop"

Write-Host "==> 🚀 Initializing MarketLab Project Directory Structural Matrix..." -ForegroundColor Cyan

# 1. Establish directory tree framework
New-Item -ItemType Directory -Force -Path "core\src\trading_stage" | Out-Null
New-Item -ItemType Directory -Force -Path "core\src\execution_engine" | Out-Null
New-Item -ItemType Directory -Force -Path "core\src\signal_kernel" | Out-Null
New-Item -ItemType Directory -Force -Path "external" | Out-Null
New-Item -ItemType Directory -Force -Path ".cursor\rules" | Out-Null

# 2. Ingest external open-source engines under dedicated tracking directories
Write-Host "==> 📥 Cloning forked Pulsar-Native GPUI host environment..." -ForegroundColor Yellow
git clone https://github.com/Business-Art-Technology-MAN/Pulsar-Native.git external/pulsar-native

Write-Host "==> 📥 Cloning forked Plugin_Blueprints canvas pipeline framework..." -ForegroundColor Yellow
git clone https://github.com/Business-Art-Technology-MAN/Plugin_Blueprints.git external/plugin-blueprints

# 3. Initialize Git control shell over the composite root
if (-not (Test-Path ".git")) {
    Write-Host "==> 🛠️  Configuring a clean git tracking matrix over workspace root..." -ForegroundColor Cyan
    git init
    Set-Content -Path ".gitignore" -Value "target/`n**/*.rs.bk`n.DS_Store`n.history/"
}

Write-Host "==> 🎉 Structural Workspace setup complete. Ready for Cargo compilation scaffolding." -ForegroundColor Green