//! Wrapper utilities for the native [`openusd`] structural plane.
//!
//! High-frequency temporal sweeps stay on [`MarketStage`]; this module handles
//! LIVRPS layer composition, prim activation, and session metadata via
//! [`openusd::Stage`].

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, Instant};

use openusd::sdf::schema::FieldKey;
use crate::trading_stage::{
    is_legacy_bucket_path, prim_type_name, should_show_prim_in_stage_tree,
    MARKETLAB_ROOT,
};
use openusd::Stage;
use pulsar_marketlab_core::financial_schema_defaults;
use pulsar_marketlab_ui::workspace::ManagedUsdStage;

use crate::signal_dsl::Vector;
use crate::trading_stage::MarketStage;

/// Unified structural-plane handle wrapping [`ManagedUsdStage`].
///
/// Milestone 1: one Send+Sync authority with full overlay maps. Application code
/// should prefer [`ManagedUsdStage`] via [`WorkspaceContext`]; this newtype remains
/// for hydration, save/export, and [`ProductionStageProvider`].
#[derive(Clone, Debug)]
pub struct UsdStageBridge(ManagedUsdStage);

impl Deref for UsdStageBridge {
    type Target = ManagedUsdStage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl UsdStageBridge {
    pub fn open(root_layer_path: impl AsRef<Path>) -> io::Result<Self> {
        ManagedUsdStage::open(root_layer_path).map(Self)
    }

    pub fn open_from_usda_text(content: &str) -> io::Result<Self> {
        ManagedUsdStage::open_from_usda_text(content).map(Self)
    }

    pub fn inner(&self) -> &ManagedUsdStage {
        &self.0
    }

    pub fn into_inner(self) -> ManagedUsdStage {
        self.0
    }

    /// Cheap clone wrapper for call sites holding [`ManagedUsdStage`].
    pub fn borrow(stage: &ManagedUsdStage) -> Self {
        Self(stage.clone())
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

    /// Read a composed `float2` attribute (e.g. `ui:canvas:pos`).
    pub fn field_vec2f(&self, prim_path: &str, attribute: &str) -> Option<[f32; 2]> {
        let property_path = format!("{prim_path}.{attribute}");
        self.with_stage(|stage| {
            let value = stage
                .field::<[f32; 2]>(property_path.as_str(), "default")
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            let value = if value.is_some() {
                value
            } else {
                stage
                    .field::<[f32; 2]>(property_path.as_str(), FieldKey::Default)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
            };
            Ok(value.filter(|[x, y]| x.is_finite() && y.is_finite()))
        })
        .ok()
        .flatten()
    }

    /// Write the composed stage hierarchy to a USDA text stream on disk.
    pub fn dump_to_path(&self, target: impl AsRef<Path>) -> io::Result<()> {
        let content = if self.has_runtime_overlays() {
            self.export_usda_text()?
        } else {
            fs::read_to_string(self.root_layer_path())?
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

    /// Property grid rows for the Stage Ledger Explorer (four parsing tracks).
    pub fn stage_ledger_entries(&self) -> io::Result<Vec<StageLedgerEntrySnapshot>> {
        let layer_text = fs::read_to_string(self.root_layer_path()).unwrap_or_default();
        let schema_defaults = financial_schema_defaults();
        self.with_stage(|stage| {
            let mut entries = Vec::new();
            collect_ledger_entries(
                self,
                stage,
                &layer_text,
                &schema_defaults,
                &openusd::sdf::Path::abs_root(),
                0,
                &mut entries,
            )?;
            Ok(entries)
        })
    }

    fn prim_override_layer(&self, prim_path: &str, layer_text: &str) -> bool {
        let runtime_overlay = self
            .snapshot_runtime_overlays()
            .active_overrides
            .contains_key(prim_path);
        runtime_overlay
            || layer_contains_override_for_prim(layer_text, prim_path)
            || (layer_text.contains("subLayers") && !self.prim_active(prim_path))
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

/// One ledger grid row for the Stage Ledger Explorer pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageLedgerEntrySnapshot {
    pub prim_path: String,
    pub property: String,
    pub depth: usize,
    pub active: bool,
    pub override_layer: bool,
    pub deviates_from_schema: bool,
    pub value_label: String,
    pub lineage: Vec<String>,
}

/// Alias matching the split-plane spec: shared structural plane handle.
pub type SharedOpenUsdStage = Arc<UsdStageBridge>;

pub fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

fn prim_def_type(path_str: &str, stage: &Stage) -> &'static str {
    if let Some(type_name) = prim_type_name(stage, path_str) {
        return match type_name.as_str() {
            "FinancialAsset" => "FinancialAsset",
            "OtlOperator" => "OtlOperator",
            "OtlTaUberSignal" => "OtlTaUberSignal",
            "PortfolioIntegrator" => "PortfolioIntegrator",
            "Scope" => "Scope",
            _ => "Scope",
        };
    }
    if path_str == MARKETLAB_ROOT || is_legacy_bucket_path(path_str) {
        return "Scope";
    }
    "Scope"
}

fn stage_tree_label(bridge: &UsdStageBridge, path_str: &str, child_name: &str) -> String {
    let user_label = bridge.field_string(path_str, pulsar_marketlab_core::USER_LABEL_ATTR);
    let display =
        pulsar_marketlab_core::prim_display_label(child_name, user_label.as_deref());
    bridge
        .prim_type_name(path_str)
        .map(|type_name| format!("{display} ({type_name})"))
        .unwrap_or(display)
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
        if !should_show_prim_in_stage_tree(stage, &path_str) {
            continue;
        }
        let _label = stage_tree_label(bridge, &path_str, child_name.as_str());
        let active = bridge.0.prim_active(&path_str);
        let indent = "    ".repeat(depth + 1);
        let inner = "    ".repeat(depth + 2);
        let def_type = prim_def_type(&path_str, stage);

        out.push_str(&format!("{indent}def {def_type} \"{child_name}\"\n"));
        if !active {
            out.push_str(&format!("{indent}(\n"));
            out.push_str(&format!("{inner}active = false\n"));
            out.push_str(&format!("{indent})\n"));
        }
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
                if let Ok(Some(value)) =
                    stage.field::<String>(property_path_str.as_str(), "default")
                {
                    if property.starts_with("inputs:") || property.starts_with("outputs:") {
                        if property.contains("script") || property.contains("category") {
                            write_usda_string(out, &inner, &property, &value);
                        } else {
                            write_usda_token(out, &inner, &property, &value);
                        }
                    } else {
                        write_usda_string(out, &inner, &property, &value);
                    }
                } else if let Ok(Some(value)) =
                    stage.field::<f64>(property_path_str.as_str(), "default")
                {
                    write_usda_double(out, &inner, &property, value);
                } else if let Ok(Some(value)) =
                    stage.field::<f32>(property_path_str.as_str(), "default")
                {
                    if property.starts_with("ui:") {
                        write_usda_custom_float(out, &inner, &property, value);
                    } else {
                        write_usda_float(out, &inner, &property, value);
                    }
                } else if let Ok(Some(value)) =
                    stage.field::<[f32; 2]>(property_path_str.as_str(), "default")
                {
                    if property.starts_with("ui:") {
                        write_usda_custom_float2(out, &inner, &property, value);
                    } else {
                        write_usda_float2(out, &inner, &property, value);
                    }
                } else if let Ok(Some(value)) =
                    stage.field::<bool>(property_path_str.as_str(), "default")
                {
                    write_usda_bool(out, &inner, &property, value);
                }
            }
        }

        export_prim_tree(bridge, stage, &child_path, depth + 1, out)?;
        out.push_str(&format!("{indent}}}\n"));
    }
    Ok(())
}

fn write_usda_token(out: &mut String, indent: &str, name: &str, value: &str) {
    let escaped = value.replace('"', "\\\"");
    out.push_str(&format!("{indent}token {name} = \"{escaped}\"\n"));
}

fn write_usda_string(out: &mut String, indent: &str, name: &str, value: &str) {
    let escaped = value.replace('"', "\\\"");
    out.push_str(&format!("{indent}string {name} = \"{escaped}\"\n"));
}

fn write_usda_bool(out: &mut String, indent: &str, name: &str, value: bool) {
    out.push_str(&format!(
        "{indent}bool {name} = {}\n",
        if value { "1" } else { "0" }
    ));
}

fn write_usda_float(out: &mut String, indent: &str, name: &str, value: f32) {
    out.push_str(&format!("{indent}float {name} = {value}\n"));
}

fn write_usda_double(out: &mut String, indent: &str, name: &str, value: f64) {
    out.push_str(&format!("{indent}double {name} = {value}\n"));
}

fn write_usda_custom_float(out: &mut String, indent: &str, name: &str, value: f32) {
    out.push_str(&format!("{indent}custom float {name} = {value}\n"));
}

fn write_usda_float2(out: &mut String, indent: &str, name: &str, value: [f32; 2]) {
    out.push_str(&format!(
        "{indent}float2 {name} = ({}, {})\n",
        value[0], value[1]
    ));
}

fn write_usda_custom_float2(out: &mut String, indent: &str, name: &str, value: [f32; 2]) {
    out.push_str(&format!(
        "{indent}custom float2 {name} = ({}, {})\n",
        value[0], value[1]
    ));
}

const LINEAGE_RELATIONSHIPS: &[&str] = &["inputs:target", "inputs:constituents", "inputs:underlying", "inputs:sources"];

fn layer_contains_override_for_prim(layer_text: &str, prim_path: &str) -> bool {
    let prim_name = prim_path.rsplit('/').next().unwrap_or(prim_path);
    layer_text.contains(&format!("over \"{prim_name}\""))
        || layer_text.contains(&format!("over '{prim_name}'"))
}

fn collect_ledger_entries(
    bridge: &UsdStageBridge,
    stage: &Stage,
    layer_text: &str,
    schema_defaults: &HashMap<String, String>,
    path: &openusd::sdf::Path,
    depth: usize,
    entries: &mut Vec<StageLedgerEntrySnapshot>,
) -> io::Result<()> {
    let children = stage
        .prim_children(path.clone())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    for child_name in children {
        let child_path = path
            .append_path(child_name.as_str())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let path_str = child_path.to_string();
        let active = bridge.0.prim_active(&path_str);
        let override_layer = bridge.prim_override_layer(&path_str, layer_text);

        let inputs_active = read_inputs_active(stage, &path_str).unwrap_or(active);
        entries.push(StageLedgerEntrySnapshot {
            prim_path: path_str.clone(),
            property: "inputs:active".to_string(),
            depth,
            active: inputs_active,
            override_layer,
            deviates_from_schema: schema_deviates(schema_defaults, "inputs:active", &bool_label(inputs_active)),
            value_label: bool_label(inputs_active),
            lineage: lineage_for_property(layer_text, &path_str, "inputs:active"),
        });

        if let Ok(properties) = stage.prim_properties(child_path.clone()) {
            for property in properties {
                if property == "active" {
                    continue;
                }
                let property_path = child_path
                    .append_property(&property)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                let property_path_str = property_path.to_string();
                let value_label = read_property_value_label(stage, &property_path_str, &property);
                let schema_key = normalize_schema_key(&property);
                let deviates = schema_deviates(schema_defaults, &schema_key, &value_label);
                let lineage = if LINEAGE_RELATIONSHIPS.iter().any(|rel| *rel == schema_key.as_str()) {
                    lineage_for_property(layer_text, &path_str, &schema_key)
                } else {
                    Vec::new()
                };
                entries.push(StageLedgerEntrySnapshot {
                    prim_path: path_str.clone(),
                    property: property.clone(),
                    depth: depth + 1,
                    active: inputs_active,
                    override_layer,
                    deviates_from_schema: deviates,
                    value_label,
                    lineage,
                });
            }
        }

        collect_ledger_entries(
            bridge,
            stage,
            layer_text,
            schema_defaults,
            &child_path,
            depth + 1,
            entries,
        )?;
    }
    Ok(())
}

fn read_inputs_active(stage: &Stage, prim_path: &str) -> Option<bool> {
    let property_path = format!("{prim_path}.inputs:active");
    stage
        .field::<bool>(property_path.as_str(), FieldKey::Default)
        .ok()
        .flatten()
        .or_else(|| {
            stage
                .field::<bool>(prim_path, FieldKey::Active)
                .ok()
                .flatten()
        })
}

fn read_property_value_label(stage: &Stage, property_path: &str, property: &str) -> String {
    if let Ok(Some(value)) = stage.field::<bool>(property_path, FieldKey::Default) {
        return bool_label(value);
    }
    if let Ok(Some(value)) = stage.field::<f32>(property_path, FieldKey::Default) {
        return format!("{value}");
    }
    if let Ok(Some(value)) = stage.field::<f64>(property_path, FieldKey::Default) {
        return format!("{value}");
    }
    if let Ok(Some(value)) = stage.field::<String>(property_path, FieldKey::Default) {
        return value;
    }
    if let Ok(Some(value)) = stage.field::<String>(property_path, "default") {
        return value;
    }
    property.to_string()
}

fn normalize_schema_key(property: &str) -> String {
    if property.starts_with("inputs:") || property.starts_with("outputs:") {
        property.to_string()
    } else {
        format!("inputs:{property}")
    }
}

fn bool_label(value: bool) -> String {
    if value { "1".to_string() } else { "0".to_string() }
}

fn schema_deviates(
    schema_defaults: &HashMap<String, String>,
    schema_key: &str,
    value_label: &str,
) -> bool {
    schema_defaults
        .get(schema_key)
        .is_some_and(|default| default != value_label)
}

fn lineage_for_property(layer_text: &str, prim_path: &str, relationship: &str) -> Vec<String> {
    let prim_name = prim_path.rsplit('/').next().unwrap_or(prim_path);
    let mut labels = Vec::new();
    let mut in_prim = false;
    for line in layer_text.lines() {
        let trimmed = line.trim();
        if trimmed.contains(&format!("\"{prim_name}\"")) || trimmed.contains(&format!("'{prim_name}'")) {
            in_prim = true;
        }
        if in_prim && trimmed.contains(relationship) {
            for token in trimmed.split_whitespace() {
                if token.starts_with("</") {
                    labels.push(format!("→ {token}"));
                }
            }
        }
        if in_prim && trimmed == "}" {
            break;
        }
    }
    if labels.is_empty() && (relationship == "inputs:underlying" || relationship == "inputs:sources") {
        labels.push(format!("→ (unbound {relationship})"));
    }
    labels
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
        if !bridge
            .with_stage(|stage| Ok(should_show_prim_in_stage_tree(stage, &path_str)))
            .unwrap_or(true)
        {
            continue;
        }
        let label = stage_tree_label(bridge, &path_str, child_name.as_str());
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
    let prim_path = "/MarketLab/SPY";

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
        assert!(usd.prim_active("/MarketLab/SPY"));
    }

    #[test]
    fn inactive_overlay_blocks_temporal_resolve() {
        let usd =
            UsdStageBridge::open(fixture_path("spy_assets_inactive_overlay.usda")).expect("open");
        assert!(!usd.prim_active("/MarketLab/SPY"));

        let temporal = seeded_temporal_stage(8);
        let value = composed_close_at(&temporal, "/MarketLab/SPY", "close", 4.0, &usd);
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

    #[test]
    fn open_document_without_schema_sidecar_still_loads_operational_prims() {
        let dir = std::env::temp_dir().join(format!("marketlab_open_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let doc = dir.join("solo.usda");
        fs::write(
            &doc,
            r#"#usda 1.0
(
    subLayers = [
        @./schema.usda@
    ]
    defaultPrim = "MarketLab"
)

def Scope "MarketLab"
{
    def FinancialAsset "SPY"
    {
        bool inputs:active = 1
        token inputs:symbol = "SPY"
    }
}
"#,
        )
        .expect("write temp usda");

        let bridge = UsdStageBridge::open(&doc).expect("open document without schema sidecar");
        assert!(bridge.prim_active("/MarketLab/SPY"));
        let rows = bridge.stage_prim_rows().expect("stage tree rows");
        assert!(rows.iter().any(|row| row.path == "/MarketLab/SPY"));
        let _ = fs::remove_dir_all(&dir);
    }
}
