//! Wrapper utilities for the native [`openusd`] structural plane.
//!
//! High-frequency temporal sweeps stay on [`MarketStage`]; this module handles
//! LIVRPS layer composition, prim activation, and session metadata via
//! [`openusd::Stage`].

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use openusd::sdf::schema::FieldKey;
use openusd::Stage;

use crate::signal_dsl::Vector;
use crate::trading_stage::MarketStage;

/// Send + Sync handle to a composed OpenUSD root layer.
///
/// `openusd::Stage` uses interior mutability and is `!Send` / `!Sync` in 0.3.0.
/// Structural queries reopen the root layer per call (infrequent vs temporal sweeps).
#[derive(Clone, Debug)]
pub struct UsdStageBridge {
    root_layer_path: Arc<String>,
    active_overrides: Arc<Mutex<HashMap<String, bool>>>,
}

impl UsdStageBridge {
    /// Open and validate a composed stage from a root `.usda` / `.usd` path.
    pub fn open(root_layer_path: impl AsRef<Path>) -> io::Result<Self> {
        let path = root_layer_path.as_ref().to_string_lossy().into_owned();
        Stage::open(&path).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(Self {
            root_layer_path: Arc::new(path),
            active_overrides: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Write inline USDA text to a temp file, then open it as a composed stage.
    pub fn open_from_usda_text(content: &str) -> io::Result<Self> {
        let path = write_inline_usda(content)?;
        Self::open(path)
    }

    pub fn root_layer_path(&self) -> &str {
        &self.root_layer_path
    }

    /// Run a callback against a freshly opened native [`Stage`].
    pub fn with_stage<R>(
        &self,
        f: impl FnOnce(&Stage) -> Result<R, io::Error>,
    ) -> Result<R, io::Error> {
        let stage = Stage::open(&self.root_layer_path)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        f(&stage)
    }

    /// LIVRPS-composed `active` metadata for a prim path (defaults to `true`).
    pub fn prim_active(&self, prim_path: &str) -> bool {
        if let Some(active) = self
            .active_overrides
            .lock()
            .ok()
            .and_then(|overrides| overrides.get(prim_path).copied())
        {
            return active;
        }
        self.read_prim_active_from_stage(prim_path)
    }

    /// Mutate composed `FieldKey::Active` for a prim path in the bridge overlay.
    pub fn set_prim_active(&self, prim_path: &str, active: bool) {
        if let Ok(mut overrides) = self.active_overrides.lock() {
            overrides.insert(prim_path.to_string(), active);
        }
    }

    fn read_prim_active_from_stage(&self, prim_path: &str) -> bool {
        self.with_stage(|stage| {
            Ok(stage
                .field::<bool>(prim_path, FieldKey::Active)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                .unwrap_or(true))
        })
        .unwrap_or(true)
    }

    /// Typed composed field lookup on a prim or property path.
    pub fn field_f32(&self, path: &str, field: impl AsRef<str>) -> Option<f32> {
        self.with_stage(|stage| {
            Ok(stage
                .field::<f32>(path, field.as_ref())
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                .filter(|value| value.is_finite()))
        })
        .ok()
        .flatten()
    }

    /// Replace the root layer path and clear runtime overlays.
    pub fn reload_from_path(&mut self, root_layer_path: impl AsRef<Path>) -> io::Result<()> {
        let path = root_layer_path.as_ref().to_string_lossy().into_owned();
        Stage::open(&path).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        self.root_layer_path = Arc::new(path);
        if let Ok(mut overrides) = self.active_overrides.lock() {
            overrides.clear();
        }
        Ok(())
    }

    /// Write the composed stage hierarchy to a USDA text stream on disk.
    pub fn dump_to_path(&self, target: impl AsRef<Path>) -> io::Result<()> {
        let overrides = self
            .active_overrides
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let content = if overrides.is_empty() {
            fs::read_to_string(self.root_layer_path.as_str())?
        } else {
            self.export_usda_text()?
        };
        fs::write(target.as_ref(), content)
    }

    fn export_usda_text(&self) -> io::Result<String> {
        self.with_stage(|stage| {
            let mut out = String::from("#usda 1.0\n(\n");
            if let Some(default_prim) = stage.default_prim() {
                out.push_str(&format!("    defaultPrim = \"{default_prim}\"\n"));
            }
            out.push_str(")\n\n");
            export_prim_tree(self, stage, &openusd::sdf::Path::abs_root(), 0, &mut out)?;
            Ok(out)
        })
    }

    /// Flatten the composed USD hierarchy for the stage composer pane.
    pub fn stage_prim_rows(&self) -> io::Result<Vec<StagePrimRowSnapshot>> {
        self.with_stage(|stage| {
            let mut rows = Vec::new();
            collect_prim_rows(self, stage, &openusd::sdf::Path::abs_root(), 0, &mut rows)?;
            Ok(rows)
        })
    }
}

/// One prim row read from the composed OpenUSD stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagePrimRowSnapshot {
    pub path: String,
    pub label: String,
    pub depth: usize,
    pub active: bool,
}

/// Alias matching the split-plane spec: shared structural plane handle.
pub type SharedOpenUsdStage = Arc<UsdStageBridge>;

pub fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

fn write_inline_usda(content: &str) -> io::Result<String> {
    let dir = std::env::temp_dir().join("marketlab_openusd");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("inline_{}.usda", std::process::id()));
    fs::write(&path, content)?;
    Ok(path.to_string_lossy().into_owned())
}

fn export_prim_tree(
    bridge: &UsdStageBridge,
    stage: &Stage,
    path: &openusd::sdf::Path,
    depth: usize,
    out: &mut String,
) -> io::Result<()> {
    let children = stage
        .prim_children(path.clone())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    for child_name in children {
        let child_path = path
            .append_path(child_name.as_str())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let path_str = child_path.to_string();
        let active = bridge.prim_active(&path_str);
        let indent = "    ".repeat(depth + 1);
        let inner = "    ".repeat(depth + 2);

        out.push_str(&format!("{indent}def Xform \"{child_name}\" (\n"));
        out.push_str(&format!("{inner}active = {}\n", if active { "true" } else { "false" }));
        out.push_str(&format!("{indent})\n"));
        out.push_str(&format!("{indent}{{\n"));

        if let Ok(properties) = stage.prim_properties(child_path.clone()) {
            for property in properties {
                if property == "active" {
                    continue;
                }
                let property_path = child_path
                    .append_property(&property)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                let property_path_str = property_path.to_string();
                if let Ok(Some(value)) = stage.field::<f32>(property_path_str.as_str(), "default") {
                    out.push_str(&format!(
                        "{inner}custom float {property} = {value}\n"
                    ));
                }
            }
        }

        export_prim_tree(bridge, stage, &child_path, depth + 1, out)?;
        out.push_str(&format!("{indent}}}\n"));
    }
    Ok(())
}

fn collect_prim_rows(
    bridge: &UsdStageBridge,
    stage: &Stage,
    path: &openusd::sdf::Path,
    depth: usize,
    rows: &mut Vec<StagePrimRowSnapshot>,
) -> io::Result<()> {
    let children = stage
        .prim_children(path.clone())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    for child_name in children {
        let child_path = path
            .append_path(child_name.as_str())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let path_str = child_path.to_string();
        let label = child_path
            .name()
            .map(|name| {
                if depth == 0 && path == &openusd::sdf::Path::abs_root() {
                    format!("{name} (Xform)")
                } else {
                    name.to_string()
                }
            })
            .unwrap_or_else(|| path_str.clone());
        rows.push(StagePrimRowSnapshot {
            path: path_str.clone(),
            label,
            depth,
            active: bridge.prim_active(&path_str),
        });
        collect_prim_rows(bridge, stage, &child_path, depth + 1, rows)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsdSpikeProfile {
    pub sample_count: usize,
    pub query_count: usize,
    pub resolve_elapsed: Duration,
    pub range_elapsed: Duration,
}

/// Profile temporal sweeps on [`MarketStage`] plus USD prim-active checks.
pub fn profile_time_sample_retrieval(
    temporal_stage: &MarketStage,
    usd_stage: &UsdStageBridge,
    sample_count: usize,
    query_count: usize,
) -> UsdSpikeProfile {
    let prim_path = "/assets/SPY";

    let resolve_start = Instant::now();
    for query in 0..query_count {
        let _ = usd_stage.prim_active(prim_path);
        let t = (query % sample_count.max(1)) as f64;
        let _ = temporal_stage.resolve_attribute_at(prim_path, "close", t);
    }
    let resolve_elapsed = resolve_start.elapsed();

    let range_start = Instant::now();
    for query in 0..query_count {
        let end = (query % sample_count.max(1)) as f64;
        let start = (end - 64.0).max(0.0);
        if usd_stage.prim_active(prim_path) {
            let _ = temporal_stage.samples_in_time_range(prim_path, "close", start, end);
        }
    }
    let range_elapsed = range_start.elapsed();

    UsdSpikeProfile {
        sample_count,
        query_count,
        resolve_elapsed,
        range_elapsed,
    }
}

pub fn composed_close_at(
    temporal_stage: &MarketStage,
    prim_path: &str,
    attribute: &str,
    t: f64,
    usd_stage: &UsdStageBridge,
) -> Option<Vector> {
    if !usd_stage.prim_active(prim_path) {
        return None;
    }
    temporal_stage
        .resolve_attribute_at(prim_path, attribute, t)
        .map(f64::from)
        .filter(|value| value.is_finite())
        .map(Vector::scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading_stage::asset_prim_path;

    fn seeded_temporal_stage(sample_count: usize) -> MarketStage {
        let mut stage = MarketStage::new();
        let prim = asset_prim_path("SPY").unwrap();
        for index in 0..sample_count {
            stage
                .set_sample(&prim, "close", index as f64, 100.0 + index as f32 * 0.01)
                .unwrap();
        }
        stage
    }

    #[test]
    fn native_stage_reads_active_metadata_from_fixture() {
        let usd = UsdStageBridge::open(fixture_path("spy_assets.usda")).expect("open fixture");
        assert!(usd.prim_active("/assets/SPY"));
    }

    #[test]
    fn inactive_overlay_blocks_temporal_resolve() {
        let usd =
            UsdStageBridge::open(fixture_path("spy_assets_inactive_overlay.usda")).expect("open");
        assert!(!usd.prim_active("/assets/SPY"));

        let temporal = seeded_temporal_stage(8);
        let value = composed_close_at(&temporal, "/assets/SPY", "close", 4.0, &usd);
        assert!(value.is_none());
    }

    #[test]
    fn profile_retrieval_completes_for_large_sample_sets() {
        let usd = UsdStageBridge::open(fixture_path("spy_assets.usda")).expect("open fixture");
        let temporal = seeded_temporal_stage(4_096);
        let profile = profile_time_sample_retrieval(&temporal, &usd, 4_096, 8_192);
        assert_eq!(profile.sample_count, 4_096);
        assert_eq!(profile.query_count, 8_192);
        assert!(profile.resolve_elapsed < Duration::from_secs(30));
        assert!(profile.range_elapsed < Duration::from_secs(30));
    }

    #[test]
    fn bridge_is_send_and_sync() {
        let usd = UsdStageBridge::open(fixture_path("spy_assets.usda")).expect("open fixture");
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        assert_send_sync(&usd);
        assert_send_sync(&Arc::new(usd));
    }
}
