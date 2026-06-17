use pulsar_marketlab_ui::workspace::{
    build_stage_graph_snapshot, ManagedUsdStage,
};

const FINANCE_DB_ASSET_USDA: &str = r#"#usda 1.0
(
    defaultPrim = "MarketLab"
)

def Scope "MarketLab"
{
    def FinancialAsset "node_asset_aapl"
    {
        token inputs:symbol = "AAPL"
        string info:sector = "Information Technology"
        string info:industry = "Software & Services"
        string info:market_cap_class = "Mega-Cap"
        string info:currency = "USD"
        string info:country = "United States"
        string info:user_label = "Apple Inc."
    }
}
"#;

#[test]
fn composed_asset_meta_hydrates_info_namespace_from_stage() {
    let stage = ManagedUsdStage::open_from_usda_text(FINANCE_DB_ASSET_USDA).expect("stage");
    let snapshot = build_stage_graph_snapshot(&stage);
    let meta = snapshot
        .asset_registry
        .get("/MarketLab/node_asset_aapl")
        .expect("asset registry entry");
    assert_eq!(meta.symbol, "AAPL");
    assert_eq!(meta.sector, "Information Technology");
    assert_eq!(meta.industry, "Software & Services");
    assert_eq!(meta.market_cap_class, "Mega-Cap");
    assert_eq!(meta.currency, "USD");
    assert_eq!(meta.country, "United States");
    assert_eq!(meta.user_label, "Apple Inc.");
}
