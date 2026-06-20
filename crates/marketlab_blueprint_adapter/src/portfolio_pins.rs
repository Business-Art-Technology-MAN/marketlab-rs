//! Dynamic signal input layout for Portfolio Integrator nodes.
//!
//! The EventGraph shows `connected + 1` input pins (always one spare). Connections are
//! compacted to sequential `signal_0..signal_{n-1}` indices after topology changes.

use crate::types::type_id;

pub fn portfolio_signal_pin_id(index: usize) -> String {
    format!("signal_{index}")
}

pub fn portfolio_signal_pin_index(pin_id: &str) -> Option<usize> {
    let suffix = pin_id.strip_prefix("signal_")?;
    suffix.parse().ok()
}

pub fn is_portfolio_signal_pin(pin_id: &str) -> bool {
    portfolio_signal_pin_index(pin_id).is_some()
}

/// Number of signal input pins that should exist for `connected_count` wired sources.
pub fn portfolio_signal_pin_count(connected_count: usize) -> usize {
    connected_count.saturating_add(1).max(1)
}

/// Compact incoming portfolio wires to sequential pins and return the required pin count.
pub fn compact_portfolio_signal_target_pins(
    node_id: &str,
    definition_id: &str,
    connections: &mut [(String, String)],
) -> usize {
    if definition_id != type_id::PORTFOLIO_INTEGRATOR {
        return 1;
    }

    let mut indexed: Vec<(usize, usize)> = connections
        .iter()
        .enumerate()
        .filter_map(|(conn_idx, (target_node, target_pin))| {
            if target_node != node_id {
                return None;
            }
            Some((portfolio_signal_pin_index(target_pin)?, conn_idx))
        })
        .collect();
    indexed.sort_by_key(|(pin_idx, _)| *pin_idx);

    for (new_idx, (_, conn_idx)) in indexed.iter().enumerate() {
        let (_, target_pin) = &mut connections[*conn_idx];
        let next = portfolio_signal_pin_id(new_idx);
        if target_pin.as_str() != next {
            *target_pin = next;
        }
    }

    portfolio_signal_pin_count(indexed.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_renumbers_and_reports_required_pin_count() {
        let mut wires = vec![
            ("fund".to_string(), "signal_2".to_string()),
            ("fund".to_string(), "signal_0".to_string()),
            ("other".to_string(), "signal_1".to_string()),
        ];
        let count = compact_portfolio_signal_target_pins(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            &mut wires,
        );
        assert_eq!(count, 3);
        assert_eq!(wires[0].1, "signal_1");
        assert_eq!(wires[1].1, "signal_0");
        assert_eq!(wires[2].1, "signal_1");
    }

    #[test]
    fn empty_portfolio_keeps_one_spare_pin() {
        let mut wires = Vec::new();
        let count = compact_portfolio_signal_target_pins(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            &mut wires,
        );
        assert_eq!(count, 1);
    }
}
