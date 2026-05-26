# MarketLab Workspace Initialization & Repository Setup

This document establishes the step-by-step procedure to bootstrap the MarketLab engineering framework. It clones the core GPU-accelerated graphic host frameworks, isolates our proprietary financial layers, and structures the environment as a unified Cargo Workspace.

## Prerequisites

Ensure the following system tools are installed and globally accessible before executing the configuration phase:
* **Rust Toolchain:** Stable version 1.78+ (with `cargo` and `rustc`)
* **Git Source Control Tooling**
* **System Library Hooks:** Native build dependencies required for GPUI compilation (C++ toolchains, cmake, fontconfig, and x11/vulkan libraries depending on your Host OS).

## 1. Automated Initialization Step

Create a shell script named `setup_workspace.sh` in your workspace root, paste the code chunk below, make it executable, and run it.

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "==> 🚀 Initializing MarketLab Project Directory Structural Matrix..."

# 1. Establish directory tree framework
mkdir -p core/src/trading_stage
mkdir -p core/src/execution_engine
mkdir -p core/src/signal_kernel
mkdir -p external
mkdir -p .cursor/rules

# 2. Ingest external open-source engines under dedicated tracking directories
echo "==> 📥 Cloning forked Pulsar-Native GPUI host environment..."
git clone [https://github.com/Business-Art-Technology-MAN/Pulsar-Native.git](https://github.com/Business-Art-Technology-MAN/Pulsar-Native.git) external/pulsar-native

echo "==> 📥 Cloning forked Plugin_Blueprints canvas pipeline framework..."
git clone [https://github.com/Business-Art-Technology-MAN/Plugin_Blueprints.git](https://github.com/Business-Art-Technology-MAN/Plugin_Blueprints.git) external/plugin-blueprints

# 3. Initialize Git control shell over the composite root
if [ ! -d ".git" ]; then
    echo "==> 🛠️  Configuring a clean git tracking matrix over workspace root..."
    git init
    echo -e "target/\n**/*.rs.bk\n.DS_Store" > .gitignore
fi

echo "==> 🎉 Structural Workspace setup complete. Ready for Cargo compilation scaffolding."