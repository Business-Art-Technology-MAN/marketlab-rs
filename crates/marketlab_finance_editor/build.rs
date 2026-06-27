//! Patches upstream `pulsar_rendering` to use the modern `#[pulsar_type(...)]` form.
//!
//! Pulsar-Native still emits proc-macro deprecation warnings for legacy `primitive` /
//! `structure` arguments until the pinned rev is bumped.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    for path in pulsar_rendering_component_files() {
        patch_pulsar_type_attrs(&path);
    }
}

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cargo"))
        })
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

fn pulsar_rendering_component_files() -> Vec<PathBuf> {
    let checkouts = cargo_home().join("git/checkouts");
    let Ok(entries) = fs::read_dir(&checkouts) else {
        return Vec::new();
    };

    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let root = entry.path();
        if !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("pulsar-native-"))
        {
            continue;
        }
        let Ok(rev_dirs) = fs::read_dir(&root) else {
            continue;
        };
        for rev in rev_dirs.flatten() {
            let components = rev
                .path()
                .join("crates/pulsar_rendering/src/components");
            if components.is_dir() {
                paths.push(components.join("script_component.rs"));
                paths.push(components.join("static_mesh_component.rs"));
            }
        }
    }
    paths
}

fn patch_pulsar_type_attrs(path: &Path) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let legacy = "#[pulsar_reflection::pulsar_type(\n    primitive,\n    structure = String,";
    if !content.contains(legacy) {
        return;
    }
    let patched = content.replace(
        legacy,
        "#[pulsar_reflection::pulsar_type(",
    );
    if patched != content {
        let _ = fs::write(path, patched);
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
