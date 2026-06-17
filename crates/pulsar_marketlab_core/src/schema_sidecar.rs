//! Co-located `schema.usda` sidecar helpers for on-disk stage documents.
//!
//! In-memory stages embed [`schema_class_definitions_usda`] inline so class
//! definitions are active without a physical sublayer path.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{FINANCIAL_SCHEMA_USDA, taxonomy::METADATA_LIBRARY_USDA};

/// Filename written beside saved stage documents.
pub const SCHEMA_SIDECAR_FILENAME: &str = "schema.usda";

/// Relative sublayer anchor written in saved stage root metadata.
pub const SCHEMA_SUBLAYER_REF: &str = "@./schema.usda@";

pub use crate::taxonomy::{
    METADATA_LIBRARY_SIDECAR_FILENAME, METADATA_SUBLAYER_REF,
};

const SCHEMA_CLASS_BODY_MARKER: &str = "over \"Plugins\"";

/// Canonical `FinancialAsset` / `OtlOperator` / `PortfolioIntegrator` class specs.
pub fn schema_class_definitions_usda() -> &'static str {
    FINANCIAL_SCHEMA_USDA
        .find(SCHEMA_CLASS_BODY_MARKER)
        .map(|start| &FINANCIAL_SCHEMA_USDA[start..])
        .unwrap_or(FINANCIAL_SCHEMA_USDA)
}

/// Pristine schema-validated empty stage for cold start and new documents.
pub fn initial_stage_usda() -> String {
    format!(
        "#usda 1.0\n(\n    defaultPrim = \"MarketLab\"\n)\n\n{}\n\ndef Scope \"MarketLab\"\n{{\n}}\n",
        schema_class_definitions_usda()
    )
}

/// Inject compiled schema class definitions into an operational session layer.
pub fn embed_schema_inline_in_layer(layer_usda: &str) -> String {
    if layer_usda.contains("class \"FinancialAsset\"") {
        return layer_usda.to_string();
    }
    let body = schema_class_definitions_usda();
    if let Some(idx) = layer_usda.find("\n)\n\n") {
        let insert_at = idx + "\n)\n\n".len();
        let mut out = String::with_capacity(layer_usda.len() + body.len() + 4);
        out.push_str(&layer_usda[..insert_at]);
        out.push_str(body);
        out.push_str("\n\n");
        out.push_str(&layer_usda[insert_at..]);
        return out;
    }
    format!("{body}\n\n{layer_usda}")
}

/// Schema layer text written beside a saved stage document (no nested sublayer refs).
pub fn schema_sidecar_usda() -> String {
    format!(
        "#usda 1.0\n(\n)\n\n{}",
        schema_class_definitions_usda()
    )
}

/// Directory that should contain `schema.usda` for a document at `document_path`.
pub fn schema_sidecar_directory(document_path: &Path) -> PathBuf {
    document_path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// Absolute path to the schema sidecar for a saved stage document.
pub fn schema_sidecar_path_for_document(document_path: &Path) -> PathBuf {
    schema_sidecar_directory(document_path).join(SCHEMA_SIDECAR_FILENAME)
}

/// Write bundled schema text next to a document when the sidecar is missing.
pub fn ensure_schema_sidecar_for_document(document_path: &Path) -> io::Result<()> {
    let sidecar = schema_sidecar_path_for_document(document_path);
    if sidecar.is_file() {
        return Ok(());
    }
    if let Some(parent) = sidecar.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sidecar, schema_sidecar_usda())?;
    Ok(())
}

/// Absolute path to the taxonomy metadata sidecar for a saved stage document.
pub fn metadata_sidecar_path_for_document(document_path: &Path) -> PathBuf {
    schema_sidecar_directory(document_path).join(METADATA_LIBRARY_SIDECAR_FILENAME)
}

/// Write bundled taxonomy library text next to a document when the sidecar is missing.
pub fn ensure_metadata_library_sidecar_for_document(document_path: &Path) -> io::Result<()> {
    let sidecar = metadata_sidecar_path_for_document(document_path);
    if sidecar.is_file() {
        return Ok(());
    }
    if let Some(parent) = sidecar.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sidecar, METADATA_LIBRARY_USDA)?;
    Ok(())
}
