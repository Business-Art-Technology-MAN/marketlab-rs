//! Workstation OpenUSD layer stack: an isolated project stage container.
//!
//! A MarketLab **project** is not a fixed portfolio template — it is a blank OpenUSD
//! stage context. The default stack scaffolds empty `session` / `signals` layers with
//! **no** `PortfolioIntegrator` instances; users may run OTL sandbox graphs (loose mode)
//! or compose hierarchical portfolios (structured mode). Imported portfolio `.usda` files
//! are injected as non-destructive sublayers between the signals cache and universe base.

/// User-authored session layer (active edit target).
pub const SESSION_LAYER_FILENAME: &str = "session.usda";

/// Graph-engine compiled topology cache.
pub const SIGNALS_LAYER_FILENAME: &str = "signals.usda";

/// Read-only institutional asset universe.
pub const SP500_UNIVERSE_LAYER_FILENAME: &str = "sp500_universe.usda";

/// Offline FinanceDatabase mirror populated exclusively by the ingestion pipeline.
pub const FINANCE_DATABASE_EQUITIES_LAYER_FILENAME: &str = "finance_database_equities.usda";

/// Ordered LIVRPS stack identifiers exposed to the UI layer list.
pub const WORKSTATION_LAYER_STACK: &[&str] = &[
    SESSION_LAYER_FILENAME,
    SIGNALS_LAYER_FILENAME,
    SP500_UNIVERSE_LAYER_FILENAME,
];

pub const SESSION_SUBLAYER_REF: &str = "@./session.usda@";
pub const SIGNALS_SUBLAYER_REF: &str = "@./signals.usda@";
pub const SP500_SUBLAYER_REF: &str = "@./sp500_universe.usda@";
pub const FINANCE_DATABASE_EQUITIES_SUBLAYER_REF: &str =
    "@./finance_database_equities.usda@";

/// Hierarchy scopes under `/MarketLab` (workstation spec §2–§3).
pub const UNIVERSE_SCOPE: &str = "Universe";
pub const SIGNALS_SCOPE: &str = "Signals";
pub const PORTFOLIOS_SCOPE: &str = "Portfolios";

/// Metadata token for immutable prim paths + mutable display labels.
pub const USER_LABEL_ATTR: &str = "info:user_label";

/// Resolve UI label: `info:user_label` when set, else prim leaf name.
pub fn prim_display_label(leaf: &str, user_label: Option<&str>) -> String {
    user_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| leaf.to_string())
}

/// Scaffolded FinanceDatabase mirror layer filled by the offline ingestion pipeline.
pub fn finance_database_equities_empty_layer_usda() -> &'static str {
    r#"#usda 1.0
(
    doc = "Hierarchical catalog data compiled from JerBouma/FinanceDatabase master metrics library."
)

def Scope "MarketLab"
{
    def Scope "Universe"
    {
    }
}
"#
}

/// Read-only SP500 universe root mounting the FinanceDatabase asset sublayer.
pub fn sp500_universe_layer_usda() -> String {
    format!(
        r#"#usda 1.0
(
    subLayers = [
        {FINANCE_DATABASE_EQUITIES_SUBLAYER_REF}
    ]
)

def Scope "MarketLab"
{{
    def Scope "Universe" (
        doc = "Immutable universe layout mounting finance database vendor properties."
    )
    {{
    }}
}}
"#
    )
}

/// Empty signals cache layer populated by graph compilation.
pub fn signals_layer_usda() -> &'static str {
    r#"#usda 1.0
(
)

def Scope "MarketLab"
{
    def Scope "Signals" (
        doc = "Compiled OTL / TA signal topology (graph engine output)."
    )
    {
    }
    def Scope "Portfolios" (
        doc = "Portfolio integrator instances compiled from the node canvas."
    )
    {
    }
}
"#
}

/// Session edit layer scaffold; operational prims are composed into this layer at save time.
pub fn session_layer_usda() -> String {
    crate::initial_stage_usda()
}

/// Root stage metadata referencing the three-layer workstation stack.
pub fn workstation_root_layer_header() -> String {
    format!(
        "#usda 1.0\n(\n    subLayers = [\n        {SESSION_SUBLAYER_REF}\n        {SIGNALS_SUBLAYER_REF}\n        {SP500_SUBLAYER_REF}\n    ]\n    defaultPrim = \"MarketLab\"\n)\n\n"
    )
}

/// Root metadata with an arbitrary ordered sublayer list (e.g. after portfolio import).
pub fn workstation_root_layer_header_with_stack(filenames: &[&str]) -> String {
    let refs: Vec<String> = filenames.iter().map(|name| format!("@./{name}@")).collect();
    let mut out = String::from("#usda 1.0\n(\n    subLayers = [\n");
    for reference in &refs {
        out.push_str("        ");
        out.push_str(reference);
        out.push_str("\n");
    }
    out.push_str("    ]\n    defaultPrim = \"MarketLab\"\n)\n\n");
    out
}

/// Blank session layer: schema classes only, zero portfolio integrator instances.
pub fn blank_project_session_layer_usda() -> String {
    session_layer_usda()
}

/// Suggested insert index for an imported portfolio sublayer (above universe base).
pub fn portfolio_import_insert_index(ordered_layers: &[String]) -> usize {
    ordered_layers
        .iter()
        .position(|layer| layer == SP500_UNIVERSE_LAYER_FILENAME)
        .unwrap_or_else(|| ordered_layers.len().saturating_sub(1))
}

/// Derive a stable on-disk filename for an imported portfolio layer.
pub fn imported_portfolio_layer_filename(source_stem: &str) -> String {
    let sanitized: String = source_stem
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let stem = if sanitized.is_empty() {
        "portfolio".to_string()
    } else {
        sanitized
    };
    format!("imported_{stem}.usda")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finance_database_layer_is_empty_universe_scaffold() {
        let layer = finance_database_equities_empty_layer_usda();
        assert!(layer.contains("FinanceDatabase"));
        assert!(layer.contains("def Scope \"Universe\""));
        assert!(!layer.contains("subLayers"));
    }

    #[test]
    fn sp500_universe_mounts_finance_database_sublayer() {
        let layer = sp500_universe_layer_usda();
        assert!(layer.contains(FINANCE_DATABASE_EQUITIES_SUBLAYER_REF));
        assert!(layer.contains("def Scope \"Universe\""));
    }
}
