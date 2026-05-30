//! Tier-2 OTL standard-library presets for canvas spawn menus.

/// Spawnable OTL math / transform preset (`OtlOperator` / `NodeType::OtlShader`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OtlStdlibPreset {
    pub id: &'static str,
    pub menu_label: &'static str,
    pub display_name: &'static str,
    pub default_script: &'static str,
}

pub const OTL_STDLIB_PRESETS: &[OtlStdlibPreset] = &[
    OtlStdlibPreset {
        id: "mix",
        menu_label: "Mix Node",
        display_name: "Mix",
        default_script: "mix(input, sma(input, 5), 0.5)",
    },
    OtlStdlibPreset {
        id: "step",
        menu_label: "Step Node",
        display_name: "Step",
        default_script: "step(sma(input, 10) - sma(input, 20), 1.0)",
    },
    OtlStdlibPreset {
        id: "clamp",
        menu_label: "Clamp Node",
        display_name: "Clamp",
        default_script: "clamp(input, -0.02, 0.02)",
    },
    OtlStdlibPreset {
        id: "sma",
        menu_label: "SMA Node",
        display_name: "SMA",
        default_script: "sma(input, 14)",
    },
    OtlStdlibPreset {
        id: "ema",
        menu_label: "EMA Node",
        display_name: "EMA",
        default_script: "ema(input, 14)",
    },
    OtlStdlibPreset {
        id: "cross",
        menu_label: "Cross Node",
        display_name: "Cross",
        default_script: "cross(sma(input, 5), sma(input, 20))",
    },
    OtlStdlibPreset {
        id: "formula",
        menu_label: "OTL Formula Node",
        display_name: "OTL Formula",
        default_script: "input",
    },
];

impl OtlStdlibPreset {
    pub fn by_id(id: &str) -> Option<&'static OtlStdlibPreset> {
        OTL_STDLIB_PRESETS
            .iter()
            .find(|preset| preset.id.eq_ignore_ascii_case(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulsar_marketlab_core::compile_script;

    #[test]
    fn otl_stdlib_presets_compile() {
        for preset in OTL_STDLIB_PRESETS {
            let _ = compile_script(preset.default_script)
                .unwrap_or_else(|err| panic!("{} failed: {err}", preset.id));
        }
    }
}
