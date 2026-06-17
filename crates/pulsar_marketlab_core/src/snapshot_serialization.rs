//! Tokenized portfolio weight snapshots (openusd 0.3 parse-safe `string outputs:weights`).

use std::collections::HashMap;

use crate::PortfolioTrackingFrame;

/// Encode aligned prim-path / weight slices without a map allocation (hot-path friendly).
pub fn serialize_portfolio_weights_from_slices(
    paths: &[impl AsRef<str>],
    weights: &[f64],
) -> String {
    let count = paths.len().min(weights.len());
    if count == 0 {
        return String::new();
    }
    let mut tokens = Vec::with_capacity(count);
    for index in 0..count {
        let weight = weights[index];
        if weight.abs() <= f64::EPSILON {
            continue;
        }
        tokens.push(format!("{}:{:.4}", paths[index].as_ref(), weight));
    }
    tokens.join(",")
}

/// Encodes a runtime map of prim-path weights into an openusd 0.3 parse-safe string.
pub fn serialize_portfolio_weights(weights: &HashMap<String, f64>) -> String {
    if weights.is_empty() {
        return String::new();
    }
    let mut paths: Vec<&String> = weights.keys().collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| format!("{}:{:.4}", path, weights[path]))
        .collect::<Vec<_>>()
        .join(",")
}

/// Decodes `path:weight` pairs back into a map for UI chart rendering.
pub fn deserialize_portfolio_weights(encoded_weights: &str) -> HashMap<String, f64> {
    let mut weight_map = HashMap::new();
    let trimmed = encoded_weights.trim();
    if trimmed.is_empty() {
        return weight_map;
    }

    for pair in trimmed.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((path, weight_str)) = pair.rsplit_once(':') else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if let Ok(weight) = weight_str.trim().parse::<f64>() {
            weight_map.insert(path.to_string(), weight);
        }
    }
    weight_map
}

/// Build per-bar encoded weight strings from a portfolio tracking matrix.
pub fn per_bar_weight_encodings(matrix: &[PortfolioTrackingFrame]) -> Vec<String> {
    if matrix.is_empty() {
        return Vec::new();
    }
    let max_bar = matrix
        .iter()
        .map(|frame| frame.timestamp)
        .max()
        .unwrap_or(0)
        .max(0) as usize;
    let mut out = Vec::with_capacity(max_bar + 1);
    for bar in 0..=max_bar {
        let mut weights: HashMap<String, f64> = HashMap::new();
        for frame in matrix.iter().filter(|frame| frame.timestamp == bar as i64) {
            let magnitude = frame.altered_portfolio_weight.abs();
            if magnitude <= f64::EPSILON {
                continue;
            }
            weights.insert(frame.asset_id.clone(), magnitude);
        }
        let sum: f64 = weights.values().sum();
        if sum > f64::EPSILON {
            for value in weights.values_mut() {
                *value /= sum;
            }
        }
        out.push(serialize_portfolio_weights(&weights));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_encoding_matches_map_encoding() {
        let paths = ["/MarketLab/Universe/node_00000001", "/MarketLab/Universe/node_00000002"];
        let weights = [0.5, 0.5];
        let from_slices = serialize_portfolio_weights_from_slices(&paths, &weights);
        let from_map = serialize_portfolio_weights(&HashMap::from([
            (paths[0].to_string(), 0.5),
            (paths[1].to_string(), 0.5),
        ]));
        assert_eq!(from_slices, from_map);
    }

    #[test]
    fn round_trip_weight_encoding() {
        let weights = HashMap::from([
            ("/MarketLab/Universe/node_00000001".to_string(), 0.5),
            ("/MarketLab/Universe/node_00000002".to_string(), 0.5),
        ]);
        let encoded = serialize_portfolio_weights(&weights);
        let decoded = deserialize_portfolio_weights(&encoded);
        assert_eq!(decoded.len(), 2);
        assert!((decoded["/MarketLab/Universe/node_00000001"] - 0.5).abs() < 1e-6);
        assert!((decoded["/MarketLab/Universe/node_00000002"] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn per_bar_encodings_track_timeline_length() {
        let matrix = vec![
            PortfolioTrackingFrame {
                timestamp: 0,
                asset_id: "/MarketLab/Universe/node_a".to_string(),
                closure_raw_weight: 1.0,
                altered_portfolio_weight: 1.0,
                current_nominal_price: 100.0,
                calculated_units: 1.0,
                investment_return: 0.0,
            },
            PortfolioTrackingFrame {
                timestamp: 1,
                asset_id: "/MarketLab/Universe/node_a".to_string(),
                closure_raw_weight: 0.5,
                altered_portfolio_weight: 0.5,
                current_nominal_price: 101.0,
                calculated_units: 1.0,
                investment_return: 0.01,
            },
            PortfolioTrackingFrame {
                timestamp: 1,
                asset_id: "/MarketLab/Universe/node_b".to_string(),
                closure_raw_weight: 0.5,
                altered_portfolio_weight: 0.5,
                current_nominal_price: 50.0,
                calculated_units: 2.0,
                investment_return: 0.0,
            },
        ];
        let bars = per_bar_weight_encodings(&matrix);
        assert_eq!(bars.len(), 2);
        assert_eq!(deserialize_portfolio_weights(&bars[0]).len(), 1);
        assert_eq!(deserialize_portfolio_weights(&bars[1]).len(), 2);
    }
}
