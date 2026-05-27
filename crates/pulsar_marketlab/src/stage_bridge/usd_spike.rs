//! Wrapper utilities for the native [`openusd`] structural plane.
//!
//! High-frequency temporal sweeps stay on [`MarketStage`]; this module handles
//! LIVRPS layer composition, prim activation, and session metadata via
//! [`openusd::Stage`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
}

impl UsdStageBridge {
    /// Open and validate a composed stage from a root `.usda` / `.usd` path.
    pub fn open(root_layer_path: impl AsRef<Path>) -> io::Result<Self> {
        let path = root_layer_path.as_ref().to_string_lossy().into_owned();
        Stage::open(&path).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(Self {
            root_layer_path: Arc::new(path),
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
