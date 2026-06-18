//! Finance workstation status-bar telemetry (§4).

use std::collections::HashSet;

use pulsar_marketlab_core::StageGraphSnapshot;

use crate::asset_data::FinanceAssetPreview;
use crate::compile::FinanceCompileReport;
use crate::sweep::FinanceSweepResult;

/// Bottom-tray diagnostic LED state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinanceDiagnosticState {
    Normal,
    AmberEvaluation,
}

/// Snapshot for the finance editor status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceWorkspaceTelemetry {
    pub system_memory_line: String,
    pub nodal_cache_health_pct: u8,
    pub nodal_cache_gauge: String,
    pub diagnostic_state: FinanceDiagnosticState,
    pub diagnostic_detail: String,
    pub fault_node_ids: Vec<String>,
}

/// Build status-bar telemetry from editor caches.
pub fn build_finance_workspace_telemetry(
    finance_node_count: usize,
    snapshot: Option<&StageGraphSnapshot>,
    asset_previews: &std::collections::HashMap<String, FinanceAssetPreview>,
    compile_report: Option<&FinanceCompileReport>,
    sweep: Option<&FinanceSweepResult>,
    compile_failed: bool,
) -> FinanceWorkspaceTelemetry {
    let heap_mib = estimate_finance_heap_mib(snapshot, asset_previews, sweep);
    let system_memory_line = format!("System: {} / {heap_mib} MB", system_ram_gib_label());
    let nodal_cache_health_pct =
        finance_nodal_cache_health_pct(finance_node_count, snapshot, asset_previews, sweep);
    let nodal_cache_gauge = format_nodal_cache_gauge(nodal_cache_health_pct);

    let mut fault_node_ids = HashSet::new();
    if let Some(report) = compile_report {
        fault_node_ids.extend(finance_fault_node_ids_from_warnings(&report.warnings));
    }
    if let Some(sweep) = sweep {
        fault_node_ids.extend(finance_fault_node_ids_from_warnings(&sweep.warnings));
    }

    let has_sweep_error = sweep.and_then(|result| result.error.as_ref()).is_some();
    let has_warnings = compile_report.map(|r| !r.warnings.is_empty()).unwrap_or(false)
        || sweep.map(|r| !r.warnings.is_empty()).unwrap_or(false);
    let amber = compile_failed || has_sweep_error || has_warnings;

    let diagnostic_state = if amber {
        FinanceDiagnosticState::AmberEvaluation
    } else {
        FinanceDiagnosticState::Normal
    };

    let diagnostic_detail = if compile_failed {
        "Compilation evaluation fault".to_string()
    } else if let Some(error) = sweep.and_then(|result| result.error.as_ref()) {
        format!("Sweep evaluation fault: {error}")
    } else if has_warnings {
        "Layer resolution / configuration warnings active".to_string()
    } else {
        "Evaluation nominal".to_string()
    };

    FinanceWorkspaceTelemetry {
        system_memory_line,
        nodal_cache_health_pct,
        nodal_cache_gauge,
        diagnostic_state,
        diagnostic_detail,
        fault_node_ids: fault_node_ids.into_iter().collect(),
    }
}

/// Parse quoted node ids from compile / sweep warnings (`'node_id'`).
pub fn finance_fault_node_ids_from_warnings(warnings: &[String]) -> HashSet<String> {
    warnings
        .iter()
        .filter_map(|warning| {
            let start = warning.find('\'')?;
            let rest = &warning[start + 1..];
            let end = rest.find('\'')?;
            let id = rest[..end].trim();
            if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            }
        })
        .collect()
}

pub fn format_nodal_cache_gauge(health_pct: u8) -> String {
    const SLOTS: usize = 10;
    let filled = ((health_pct as usize * SLOTS + 99) / 100).min(SLOTS);
    let bar: String = (0..SLOTS)
        .map(|index| if index < filled { '█' } else { '░' })
        .collect();
    format!("Nodal Cache Health: [{bar}] {health_pct}%")
}

pub fn finance_nodal_cache_health_pct(
    finance_node_count: usize,
    snapshot: Option<&StageGraphSnapshot>,
    asset_previews: &std::collections::HashMap<String, FinanceAssetPreview>,
    sweep: Option<&FinanceSweepResult>,
) -> u8 {
    if finance_node_count == 0 {
        return 0;
    }
    let Some(snapshot) = snapshot else {
        return 0;
    };

    let prim_ratio =
        (snapshot.prims.len() as f32 / finance_node_count.max(1) as f32).clamp(0.0, 1.0);
    let preview_ratio =
        (asset_previews.len() as f32 / finance_node_count.max(1) as f32).clamp(0.0, 1.0);
    let sweep_ratio = if sweep.map(FinanceSweepResult::succeeded).unwrap_or(false) {
        1.0
    } else if sweep.is_some() {
        0.35
    } else {
        0.0
    };

    let score = prim_ratio * 45.0 + preview_ratio * 35.0 + sweep_ratio * 20.0;
    score.round().clamp(0.0, 100.0) as u8
}

fn estimate_finance_heap_mib(
    snapshot: Option<&StageGraphSnapshot>,
    asset_previews: &std::collections::HashMap<String, FinanceAssetPreview>,
    sweep: Option<&FinanceSweepResult>,
) -> u64 {
    let mut bytes = 0u64;
    if let Some(snapshot) = snapshot {
        bytes += snapshot.prims.len() as u64 * 640;
        bytes += snapshot.wires.len() as u64 * 320;
        bytes += snapshot.asset_registry.len() as u64 * 256;
    }
    for preview in asset_previews.values() {
        bytes += preview.bars.len() as u64 * 48;
    }
    if let Some(sweep) = sweep {
        bytes += sweep.timeline_len as u64 * 16;
        for portfolio in &sweep.portfolios {
            bytes += portfolio.wealth_series.len() as u64 * 8;
        }
    }
    (bytes / (1024 * 1024)).max(1)
}

fn system_ram_gib_label() -> String {
    format!("{} GB", detect_system_ram_gib().unwrap_or(16))
}

fn detect_system_ram_gib() -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        return detect_system_ram_gib_windows();
    }
    #[cfg(target_os = "linux")]
    {
        return detect_system_ram_gib_linux();
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(target_os = "windows")]
fn detect_system_ram_gib_windows() -> Option<u64> {
    use std::mem::MaybeUninit;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
    }

    unsafe {
        let mut status = MaybeUninit::<MemoryStatusEx>::uninit();
        (*status.as_mut_ptr()).length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if GlobalMemoryStatusEx(status.as_mut_ptr()) == 0 {
            return None;
        }
        let status = status.assume_init();
        Some((status.total_phys / (1024 * 1024 * 1024)).max(1))
    }
}

#[cfg(target_os = "linux")]
fn detect_system_ram_gib_linux() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in contents.lines() {
        if let Some(kib) = line.strip_prefix("MemTotal:") {
            let kib = kib.trim().trim_end_matches(" kB").parse::<u64>().ok()?;
            return Some((kib / (1024 * 1024)).max(1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_renders_ten_blocks() {
        let gauge = format_nodal_cache_gauge(10);
        assert!(gauge.contains("10%"));
        assert!(gauge.contains('█'));
        assert!(gauge.contains('░'));
    }

    #[test]
    fn parses_warning_node_ids() {
        let warnings = vec![
            "Asset node 'spy' has no csv_path".to_string(),
            "Portfolio 'fund' has no inbound signal wires".to_string(),
        ];
        let ids = finance_fault_node_ids_from_warnings(&warnings);
        assert!(ids.contains("spy"));
        assert!(ids.contains("fund"));
    }

    #[test]
    fn amber_when_sweep_errors() {
        let sweep = FinanceSweepResult {
            error: Some("bad path".to_string()),
            ..FinanceSweepResult::default()
        };
        let telemetry = build_finance_workspace_telemetry(
            2,
            None,
            &std::collections::HashMap::new(),
            None,
            Some(&sweep),
            false,
        );
        assert_eq!(
            telemetry.diagnostic_state,
            FinanceDiagnosticState::AmberEvaluation
        );
    }
}
