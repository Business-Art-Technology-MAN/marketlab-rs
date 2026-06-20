//! Finance node type identifiers and taxonomy aligned with MarketLab USD schema.

/// Palette / registry categories (matches stage composer scopes).
pub mod category {
    pub const UNIVERSE: &str = "Universe";
    pub const ANALYTICS: &str = "Analytics";
    pub const PORTFOLIOS: &str = "Portfolios";
}

/// Graphy `node_type` strings for finance nodes.
pub mod type_id {
    pub const FINANCIAL_ASSET: &str = "marketlab.universe.financial_asset";
    pub const OTL_OPERATOR: &str = "marketlab.analytics.otl_operator";
    pub const TA_TREND: &str = "marketlab.analytics.ta_trend";
    pub const TA_VOLATILITY: &str = "marketlab.analytics.ta_volatility";
    pub const TA_OSCILLATOR: &str = "marketlab.analytics.ta_oscillator";
    pub const TA_CHANNEL: &str = "marketlab.analytics.ta_channel";
    pub const PORTFOLIO_INTEGRATOR: &str = "marketlab.portfolio.integrator";
}

/// Maps to [`pulsar_marketlab_core`] stage prim `type_name` values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FinanceNodeKind {
    FinancialAsset,
    OtlOperator,
    OtlTaUberSignal,
    PortfolioIntegrator,
}

impl FinanceNodeKind {
    pub fn stage_schema_type(self) -> &'static str {
        match self {
            Self::FinancialAsset => "FinancialAsset",
            Self::OtlOperator => "OtlOperator",
            Self::OtlTaUberSignal => "OtlTaUberSignal",
            Self::PortfolioIntegrator => "PortfolioIntegrator",
        }
    }

    pub fn graphy_type_id(self) -> &'static str {
        match self {
            Self::FinancialAsset => type_id::FINANCIAL_ASSET,
            Self::OtlOperator => type_id::OTL_OPERATOR,
            Self::OtlTaUberSignal => type_id::TA_TREND,
            Self::PortfolioIntegrator => type_id::PORTFOLIO_INTEGRATOR,
        }
    }

    pub fn from_graphy_type_id(type_id: &str) -> Option<Self> {
        match type_id {
            type_id::FINANCIAL_ASSET => Some(Self::FinancialAsset),
            type_id::OTL_OPERATOR => Some(Self::OtlOperator),
            type_id::TA_TREND
            | type_id::TA_VOLATILITY
            | type_id::TA_OSCILLATOR
            | type_id::TA_CHANNEL => Some(Self::OtlTaUberSignal),
            type_id::PORTFOLIO_INTEGRATOR => Some(Self::PortfolioIntegrator),
            _ => None,
        }
    }

    pub fn from_stage_type_name(type_name: &str) -> Option<Self> {
        match type_name {
            "FinancialAsset" => Some(Self::FinancialAsset),
            "OtlOperator" => Some(Self::OtlOperator),
            "OtlTaUberSignal" => Some(Self::OtlTaUberSignal),
            "PortfolioIntegrator" => Some(Self::PortfolioIntegrator),
            _ => None,
        }
    }

    pub fn ta_archetype_token(type_id: &str) -> Option<&'static str> {
        match type_id {
            type_id::TA_TREND => Some("trend"),
            type_id::TA_VOLATILITY => Some("volatility"),
            type_id::TA_OSCILLATOR => Some("oscillator"),
            type_id::TA_CHANNEL => Some("channel"),
            _ => None,
        }
    }
}

/// Portfolio allocation methods exposed in schema.usda.
pub const PORTFOLIO_ALLOCATION_TOKENS: &[&str] = &[
    "Allocation::HierarchicalRiskParity",
    "Allocation::EqualWeight",
    "Allocation::MeanVariance",
];
