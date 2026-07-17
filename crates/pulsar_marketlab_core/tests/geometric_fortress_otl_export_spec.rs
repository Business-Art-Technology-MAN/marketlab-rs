//! Compile-check exported Geometric Fortress OTL example scripts.

use pulsar_marketlab_core::{compile_script, compile_unified_script, CompiledSeries};

fn read_otl(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("read {}: {err}", path.display());
    })
}

fn strip_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn geometric_fortress_otl_scripts_compile() {
    let singles = [
        "examples/geometric_fortress/strategy/geometric_beta_multi.otl",
    ];
    for path in singles {
        let src = strip_comments(&read_otl(path));
        compile_unified_script(&src).unwrap_or_else(|err| panic!("{path}: {err}"));
    }

    let osl_shaders = [
        "examples/geometric_fortress/sensors/regime_wedge_volume.otl",
        "examples/geometric_fortress/sensors/regime_thresholds.otl",
        "examples/geometric_fortress/sensors/orientation_200.otl",
        "examples/geometric_fortress/sensors/bivector_beta_60.otl",
        "examples/geometric_fortress/sensors/scalar_beta_60.otl",
        "examples/geometric_fortress/sensors/nnls_sector_gravity.otl",
        "examples/geometric_fortress/strategy/fortress_signal.otl",
        "examples/geometric_fortress/strategy/fortress_weight_spy.otl",
        "examples/geometric_fortress/strategy/fortress_weight_tlt.otl",
        "examples/geometric_fortress/strategy/fortress_weight_gld.otl",
        "examples/geometric_fortress/strategy/fortress_weight_cash.otl",
    ];
    for path in osl_shaders {
        let src = strip_comments(&read_otl(path));
        let _ = compile_script(&src).unwrap_or_else(|err| panic!("{path}: {err}"));
    }
}

#[test]
fn geometric_beta_multi_has_two_channels() {
    let src = strip_comments(&read_otl(
        "examples/geometric_fortress/strategy/geometric_beta_multi.otl",
    ));
    let compiled = compile_unified_script(&src).expect("compile");
    let CompiledSeries::Multi(_, ports) = compiled else {
        panic!("expected multi output geometric_beta");
    };
    assert_eq!(ports, vec!["outputs:scalar", "outputs:bivector"]);
}
