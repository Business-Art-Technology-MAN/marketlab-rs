//! Static indicator registry for geometric algebra operators.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaInputKind {
    Slice,
    DualSlice,
    ConstituentMatrix,
    DualWithMatrix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaValueType {
    F64,
    Bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaOutputInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub value_type: GaValueType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaParamInfo {
    pub key: &'static str,
    pub default_int: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaIndicatorInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub input_kind: GaInputKind,
    pub params: &'static [GaParamInfo],
    pub outputs: &'static [GaOutputInfo],
}

const PERIOD: GaParamInfo = GaParamInfo {
    key: "period",
    default_int: 60,
};

const PERIOD_Q: [GaParamInfo; 2] = [
    GaParamInfo {
        key: "period",
        default_int: 252,
    },
    GaParamInfo {
        key: "q",
        default_int: 0,
    },
];

const ORIENT_OUT: [GaOutputInfo; 1] = [GaOutputInfo {
    id: "oriented",
    label: "Oriented",
    value_type: GaValueType::Bool,
}];

const SCALAR_OUT: [GaOutputInfo; 1] = [GaOutputInfo {
    id: "scalar",
    label: "Scalar Beta",
    value_type: GaValueType::F64,
}];

const BIVECTOR_OUT: [GaOutputInfo; 1] = [GaOutputInfo {
    id: "bivector",
    label: "Bivector Beta",
    value_type: GaValueType::F64,
}];

const VOLUME_OUT: [GaOutputInfo; 1] = [GaOutputInfo {
    id: "volume",
    label: "Wedge Volume",
    value_type: GaValueType::F64,
}];

const GEO_BETA_OUT: [GaOutputInfo; 2] = [
    GaOutputInfo {
        id: "scalar",
        label: "Scalar Beta",
        value_type: GaValueType::F64,
    },
    GaOutputInfo {
        id: "bivector",
        label: "Bivector Beta",
        value_type: GaValueType::F64,
    },
];

const F64_OUT: [GaOutputInfo; 1] = [GaOutputInfo {
    id: "value",
    label: "Value",
    value_type: GaValueType::F64,
}];

static INDICATORS: &[GaIndicatorInfo] = &[
    GaIndicatorInfo {
        id: "wedge_volume",
        label: "Wedge Volume",
        category: "regime",
        input_kind: GaInputKind::ConstituentMatrix,
        params: &[PERIOD],
        outputs: &VOLUME_OUT,
    },
    GaIndicatorInfo {
        id: "scalar_beta",
        label: "Scalar Beta",
        category: "selection",
        input_kind: GaInputKind::DualSlice,
        params: &[PERIOD],
        outputs: &SCALAR_OUT,
    },
    GaIndicatorInfo {
        id: "bivector_beta",
        label: "Bivector Beta",
        category: "selection",
        input_kind: GaInputKind::DualSlice,
        params: &[PERIOD],
        outputs: &BIVECTOR_OUT,
    },
    GaIndicatorInfo {
        id: "geometric_beta",
        label: "Geometric Beta",
        category: "selection",
        input_kind: GaInputKind::DualSlice,
        params: &[PERIOD],
        outputs: &GEO_BETA_OUT,
    },
    GaIndicatorInfo {
        id: "orientation",
        label: "Bivector Orientation",
        category: "safety",
        input_kind: GaInputKind::Slice,
        params: &[PERIOD],
        outputs: &ORIENT_OUT,
    },
    GaIndicatorInfo {
        id: "displacement",
        label: "Displacement",
        category: "safety",
        input_kind: GaInputKind::Slice,
        params: &[PERIOD],
        outputs: &F64_OUT,
    },
    GaIndicatorInfo {
        id: "rolling_quantile",
        label: "Rolling Quantile",
        category: "statistics",
        input_kind: GaInputKind::Slice,
        params: &PERIOD_Q,
        outputs: &F64_OUT,
    },
    GaIndicatorInfo {
        id: "nnls_weights",
        label: "NNLS Weights",
        category: "gravity",
        input_kind: GaInputKind::DualWithMatrix,
        params: &[PERIOD],
        outputs: &F64_OUT,
    },
    GaIndicatorInfo {
        id: "month_end",
        label: "Month End Flag",
        category: "calendar",
        input_kind: GaInputKind::Slice,
        params: &[],
        outputs: &ORIENT_OUT,
    },
];

pub fn list_indicators() -> &'static [GaIndicatorInfo] {
    INDICATORS
}

pub fn get_indicator(id: &str) -> Option<&'static GaIndicatorInfo> {
    INDICATORS.iter().find(|info| info.id == id)
}
