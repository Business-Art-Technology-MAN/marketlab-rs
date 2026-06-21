//! Dynamic `series_N` input layout for Performance Analytics nodes.

use crate::types::type_id;

pub const PERFORMANCE_BENCHMARK_PIN: &str = "benchmark";

pub fn performance_series_pin_id(index: usize) -> String {
    format!("series_{index}")
}

pub fn performance_series_pin_index(pin_id: &str) -> Option<usize> {
    let suffix = pin_id.strip_prefix("series_")?;
    suffix.parse().ok()
}

pub fn is_performance_series_pin(pin_id: &str) -> bool {
    performance_series_pin_index(pin_id).is_some()
}

pub fn performance_series_pin_count(connected_count: usize) -> usize {
    connected_count.saturating_add(1).max(1)
}

pub fn compact_performance_series_target_pins(
    node_id: &str,
    definition_id: &str,
    connections: &mut [(String, String)],
) -> usize {
    if definition_id != type_id::PERFORMANCE_ANALYTICS {
        return 1;
    }

    let mut indexed: Vec<(usize, usize)> = connections
        .iter()
        .enumerate()
        .filter_map(|(conn_idx, (target_node, target_pin))| {
            if target_node != node_id {
                return None;
            }
            Some((performance_series_pin_index(target_pin)?, conn_idx))
        })
        .collect();
    indexed.sort_by_key(|(pin_idx, _)| *pin_idx);

    for (new_idx, (_, conn_idx)) in indexed.iter().enumerate() {
        let (_, target_pin) = &mut connections[*conn_idx];
        let next = performance_series_pin_id(new_idx);
        if target_pin.as_str() != next {
            *target_pin = next;
        }
    }

    performance_series_pin_count(indexed.len())
}
