//! OTL Uber Signal archetypes: fixed port topology and script composition.

/// High-level TA node family (immutable port signatures per variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaArchetype {
    Trend,
    Volatility,
    Oscillator,
    Channel,
}

impl TaArchetype {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Trend => "trend",
            Self::Volatility => "volatility",
            Self::Oscillator => "oscillator",
            Self::Channel => "channel",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "trend" => Some(Self::Trend),
            "volatility" => Some(Self::Volatility),
            "oscillator" => Some(Self::Oscillator),
            "channel" => Some(Self::Channel),
            _ => None,
        }
    }

    pub fn default_algorithm(self) -> &'static str {
        match self {
            Self::Trend => "sma",
            Self::Volatility => "stddev",
            Self::Oscillator => "rsi",
            Self::Channel => "bollinger_bands",
        }
    }

    pub fn default_period(self) -> u32 {
        match self {
            Self::Trend | Self::Oscillator => 14,
            Self::Volatility | Self::Channel => 20,
        }
    }

    pub fn algorithms(self) -> &'static [&'static str] {
        match self {
            Self::Trend => &["sma", "ema", "wma", "hma", "tema"],
            Self::Volatility => &["stddev", "variance", "atr", "historical_volatility"],
            Self::Oscillator => &["rsi", "cci", "stochastic", "roc", "macd"],
            Self::Channel => &["bollinger_bands", "keltner_channels", "donchian_channels"],
        }
    }

    pub fn canonical_input_ports(self) -> &'static [&'static str] {
        &["source_stream"]
    }

    pub fn canonical_output_ports(self) -> &'static [&'static str] {
        match self {
            Self::Trend | Self::Volatility => &["result"],
            Self::Oscillator => &["oscillator", "signal_line"],
            Self::Channel => &["upper_band", "basis_line", "lower_band"],
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Trend => "Trend & Location",
            Self::Volatility => "Risk & Dispersion",
            Self::Oscillator => "Oscillator",
            Self::Channel => "Channels & Bands",
        }
    }

    pub fn spawn_menu_label(self) -> &'static str {
        match self {
            Self::Trend => "Trend & Location Node",
            Self::Volatility => "Risk & Dispersion Node",
            Self::Oscillator => "Oscillator Node",
            Self::Channel => "Channels & Bands Node",
        }
    }

    /// GPUI header accent (0xRRGGBB).
    pub fn accent_rgb(self) -> u32 {
        match self {
            Self::Trend => 0x60a5fa,
            Self::Volatility => 0xf97316,
            Self::Oscillator => 0xc084fc,
            Self::Channel => 0x34d399,
        }
    }
}

/// Which shared hyperparameter slots are active for the current algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TaHyperparamVisibility {
    pub period: bool,
    pub multiplier: bool,
    pub annualization: bool,
    pub signal_period: bool,
}

pub fn hyperparameter_visibility(config: &TaUberSignalConfig) -> TaHyperparamVisibility {
    let alg = config.algorithm.to_ascii_lowercase();
    let mut out = TaHyperparamVisibility {
        period: true,
        ..Default::default()
    };
    match config.archetype {
        TaArchetype::Trend | TaArchetype::Volatility => {}
        TaArchetype::Oscillator => {
            out.signal_period = matches!(alg.as_str(), "macd" | "stochastic");
        }
        TaArchetype::Channel => {
            out.multiplier = matches!(alg.as_str(), "bollinger_bands" | "keltner_channels");
        }
    }
    if alg == "historical_volatility" {
        out.annualization = true;
    }
    out
}

pub fn node_display_name(config: &TaUberSignalConfig) -> String {
    format!(
        "{} · {}",
        config.archetype.display_name(),
        algorithm_display_label(&config.algorithm)
    )
}

pub fn algorithm_display_label(algorithm: &str) -> String {
    match algorithm.to_ascii_lowercase().as_str() {
        "sma" => "SMA".into(),
        "ema" => "EMA".into(),
        "wma" => "WMA".into(),
        "hma" => "HMA".into(),
        "tema" => "TEMA".into(),
        "stddev" => "StdDev".into(),
        "variance" => "Variance".into(),
        "atr" => "ATR".into(),
        "historical_volatility" => "Hist. Vol".into(),
        "rsi" => "RSI".into(),
        "cci" => "CCI".into(),
        "stochastic" => "Stochastic".into(),
        "roc" => "ROC".into(),
        "macd" => "MACD".into(),
        "bollinger_bands" => "Bollinger".into(),
        "keltner_channels" => "Keltner".into(),
        "donchian_channels" => "Donchian".into(),
        other => other.to_string(),
    }
}

/// Typed TA configuration stored on canvas nodes and serialized to USD.
#[derive(Debug, Clone, PartialEq)]
pub struct TaUberSignalConfig {
    pub archetype: TaArchetype,
    pub algorithm: String,
    pub period: u32,
    pub multiplier: f32,
    pub annualization: f32,
    pub signal_period: u32,
}

impl TaUberSignalConfig {
    pub fn new(archetype: TaArchetype) -> Self {
        Self {
            algorithm: archetype.default_algorithm().to_string(),
            period: archetype.default_period(),
            multiplier: 2.0,
            annualization: 252.0,
            signal_period: 9,
            archetype,
        }
    }

    pub fn trend(algorithm: impl Into<String>) -> Self {
        let mut config = Self::new(TaArchetype::Trend);
        config.algorithm = algorithm.into();
        config
    }

    pub fn with_period(mut self, period: u32) -> Self {
        self.period = period.max(1);
        self
    }

    pub fn algorithm_allowed(&self) -> bool {
        self.archetype
            .algorithms()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&self.algorithm))
    }

    pub fn normalize_algorithm(&mut self) {
        let target = self.algorithm.to_ascii_lowercase();
        if let Some(found) = self
            .archetype
            .algorithms()
            .iter()
            .find(|name| name.eq_ignore_ascii_case(&target))
        {
            self.algorithm = (*found).to_string();
        } else {
            self.algorithm = self.archetype.default_algorithm().to_string();
        }
    }

    /// Stable prim leaf segment: `{archetype}_{algorithm}_{period}`.
    pub fn leaf_signature(&self) -> String {
        format!(
            "{}_{}_{}",
            self.archetype.as_token(),
            self.algorithm,
            self.period
        )
    }
}

/// Infer archetype from a legacy or canonical algorithm id.
pub fn infer_archetype_from_algorithm(algorithm: &str) -> TaArchetype {
    let id = algorithm.trim().to_ascii_lowercase();
    for archetype in [
        TaArchetype::Trend,
        TaArchetype::Volatility,
        TaArchetype::Oscillator,
        TaArchetype::Channel,
    ] {
        if archetype
            .algorithms()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&id))
        {
            return archetype;
        }
    }
    TaArchetype::Oscillator
}

/// Build canonical `inputs:script_src` for the vectorized OTL compiler.
pub fn compose_uber_script_src(config: &TaUberSignalConfig) -> String {
    let period = config.period.max(1);
    let alg = config.algorithm.to_ascii_lowercase();
    match config.archetype {
        TaArchetype::Trend => format!("ta::{alg}(input, {period})"),
        TaArchetype::Volatility => match alg.as_str() {
            "historical_volatility" => format!(
                "ta::historical_volatility(input, {period}, {})",
                config.annualization
            ),
            "variance" => format!("ta::variance(input, {period})"),
            "atr" => format!("ta::atr(input, {period})"),
            _ => format!("ta::{alg}(input, {period})"),
        },
        TaArchetype::Oscillator => match alg.as_str() {
            "macd" => format!(
                "ta::macd(input, {}, {})",
                period,
                config.signal_period.max(1)
            ),
            "stochastic" => format!(
                "ta::stochastic(input, {period}, {})",
                config.signal_period.max(1)
            ),
            _ => format!("ta::{alg}(input, {period})"),
        },
        TaArchetype::Channel => format!(
            "ta::{alg}(input, {period}, {})",
            config.multiplier
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ports_match_archetype() {
        assert_eq!(
            TaArchetype::Channel.canonical_output_ports(),
            &["upper_band", "basis_line", "lower_band"]
        );
        assert_eq!(
            TaArchetype::Oscillator.canonical_output_ports(),
            &["oscillator", "signal_line"]
        );
    }

    #[test]
    fn compose_trend_sma_script() {
        let config = TaUberSignalConfig::trend("sma").with_period(14);
        assert_eq!(compose_uber_script_src(&config), "ta::sma(input, 14)");
    }

    #[test]
    fn infer_rsi_as_oscillator() {
        assert_eq!(
            infer_archetype_from_algorithm("rsi"),
            TaArchetype::Oscillator
        );
    }
}
