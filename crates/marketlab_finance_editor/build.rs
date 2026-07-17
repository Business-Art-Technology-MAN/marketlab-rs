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
    for path in wgpui_renderer_files() {
        patch_wgpui_zero_size_framebuffer(&path);
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

fn wgpui_renderer_files() -> Vec<PathBuf> {
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
            .is_some_and(|name| name.starts_with("wgpui-"))
        {
            continue;
        }
        let Ok(rev_dirs) = fs::read_dir(&root) else {
            continue;
        };
        for rev in rev_dirs.flatten() {
            let renderer = rev.path().join("src/platform/cross/renderer.rs");
            if renderer.is_file() {
                paths.push(renderer);
            }
        }
    }
    paths
}

/// Avoid wgpu `Texture::create_view` validation failures when the OS reports 0×0 drawable size.
fn patch_wgpui_zero_size_framebuffer(path: &Path) {
    const UPDATE_MARKER: &str = "MARKETLAB_CLAMP_DRAWABLE_UPDATE";
    const NEW_MARKER: &str = "MARKETLAB_CLAMP_DRAWABLE_NEW";
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };

    let normalized = content.replace("\r\n", "\n");
    let mut patched = normalized.clone();
    let mut changed = false;

    if !patched.contains(UPDATE_MARKER) {
        let next = patched.replace(
            "    pub fn update_drawable_size(&mut self, size: geometry::Size<DevicePixels>) {\n        self.surface_configuration.width = size.width.0 as u32;\n        self.surface_configuration.height = size.height.0 as u32;",
            "    pub fn update_drawable_size(&mut self, size: geometry::Size<DevicePixels>) {\n        // MARKETLAB_CLAMP_DRAWABLE_UPDATE\n        let width = size.width.0.max(1) as u32;\n        let height = size.height.0.max(1) as u32;\n        self.surface_configuration.width = width;\n        self.surface_configuration.height = height;",
        );
        if next != patched {
            patched = next;
            changed = true;
        }
    }

    if !patched.contains(NEW_MARKER) {
        let next = patched.replace(
            "        path_sample_count: u32,\n    ) -> anyhow::Result<Self>\n    where\n        WindowHandle: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,\n    {\n        let surface = unsafe {",
            "        path_sample_count: u32,\n    ) -> anyhow::Result<Self>\n    where\n        WindowHandle: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,\n    {\n        // MARKETLAB_CLAMP_DRAWABLE_NEW\n        let width = width.max(1);\n        let height = height.max(1);\n        let surface = unsafe {",
        );
        if next != patched {
            patched = next;
            changed = true;
        }
    }

    if !changed {
        return;
    }

    let patched = if content.contains("\r\n") {
        patched.replace('\n', "\r\n")
    } else {
        patched
    };
    let _ = fs::write(path, patched);
    println!("cargo:rerun-if-changed={}", path.display());
}
