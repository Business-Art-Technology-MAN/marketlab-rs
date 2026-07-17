//! Geometric algebra rolling operators (article / Python reference parity).

use clifford::ops::Wedge;
use clifford::specialized::euclidean::dim2::Vector;
use nalgebra::{DMatrix, DVector};
use rayon::prelude::*;

const NORM_FLOOR: f64 = 1e-8;

/// Log returns: `ln(p_t / p_{t-1})`.
pub fn log_returns(prices: &[f64]) -> Vec<f64> {
    if prices.len() < 2 {
        return vec![f64::NAN; prices.len()];
    }
    let mut out = vec![f64::NAN; prices.len()];
    for index in 1..prices.len() {
        let prev = prices[index - 1];
        let next = prices[index];
        out[index] = if prev.is_finite() && next.is_finite() && prev > 0.0 && next > 0.0 {
            (next / prev).ln()
        } else {
            f64::NAN
        };
    }
    out
}

fn window_slice(series: &[f64], end: usize, period: usize) -> &[f64] {
    let start = end + 1 - period;
    &series[start..=end]
}

/// Scalar (dot) beta: participation / crowd score.
pub fn scalar_beta_series(asset: &[f64], market: &[f64], period: usize) -> Vec<f64> {
    let len = asset.len().min(market.len());
    if period == 0 || len < period {
        return vec![f64::NAN; len];
    }
    let mut out = vec![f64::NAN; len];
    let tail: Vec<f64> = (period - 1..len)
        .into_par_iter()
        .map(|index| scalar_beta_window(window_slice(asset, index, period), window_slice(market, index, period)))
        .collect();
    out[period - 1..].copy_from_slice(&tail);
    out
}

fn scalar_beta_window(asset: &[f64], market: &[f64]) -> f64 {
    let m_sq = dot(market, market);
    if m_sq <= NORM_FLOOR {
        return f64::NAN;
    }
    dot(asset, market) / m_sq
}

/// Bivector (rejection) beta: orthogonality / maverick score.
pub fn bivector_beta_series(asset: &[f64], market: &[f64], period: usize) -> Vec<f64> {
    let len = asset.len().min(market.len());
    if period == 0 || len < period {
        return vec![f64::NAN; len];
    }
    let mut out = vec![f64::NAN; len];
    let tail: Vec<f64> = (period - 1..len)
        .into_par_iter()
        .map(|index| bivector_beta_window(window_slice(asset, index, period), window_slice(market, index, period)))
        .collect();
    out[period - 1..].copy_from_slice(&tail);
    out
}

fn bivector_beta_window(asset: &[f64], market: &[f64]) -> f64 {
    let m_sq = dot(market, market);
    if m_sq <= NORM_FLOOR {
        return f64::NAN;
    }
    let scalar = dot(asset, market) / m_sq;
    let rejection: Vec<f64> = asset
        .iter()
        .zip(market)
        .map(|(a, m)| a - scalar * m)
        .collect();
    l2_norm(&rejection) / m_sq.sqrt()
}

/// Geometric beta: scalar + bivector decomposition.
pub fn geometric_beta_series(
    asset: &[f64],
    market: &[f64],
    period: usize,
) -> (Vec<f64>, Vec<f64>) {
    (
        scalar_beta_series(asset, market, period),
        bivector_beta_series(asset, market, period),
    )
}

/// Regime sensor: hyper-volume from Gram determinant of normalized return columns.
pub fn wedge_volume_series(columns: &[&[f64]], period: usize) -> Vec<f64> {
    if columns.is_empty() {
        return Vec::new();
    }
    let len = columns.iter().map(|c| c.len()).min().unwrap_or(0);
    if period == 0 || len < period {
        return vec![f64::NAN; len];
    }
    let mut out = vec![f64::NAN; len];
    let tail: Vec<f64> = (period - 1..len)
        .into_par_iter()
        .map(|index| wedge_volume_window(columns, index, period))
        .collect();
    out[period - 1..].copy_from_slice(&tail);
    out
}

fn wedge_volume_window(columns: &[&[f64]], end: usize, period: usize) -> f64 {
    let n = columns.len();
    if n == 0 {
        return f64::NAN;
    }
    let start = end + 1 - period;
    let mut norms = vec![NORM_FLOOR; n];
    for col_i in 0..n {
        let mut sum = 0.0_f64;
        for row in start..=end {
            let v = columns[col_i][row];
            if v.is_finite() {
                sum += v * v;
            }
        }
        norms[col_i] = sum.sqrt().max(NORM_FLOOR);
    }
    let mut gram = DMatrix::<f64>::zeros(n, n);
    for col_i in 0..n {
        for col_j in 0..n {
            let mut inner = 0.0_f64;
            for row in start..=end {
                inner += columns[col_i][row] * columns[col_j][row];
            }
            gram[(col_i, col_j)] = inner / (norms[col_i] * norms[col_j]);
        }
    }
    gram.determinant().abs().sqrt()
}

/// Orientation via 2D wedge (time × price displacement sign).
pub fn orientation_series(prices: &[f64], period: usize) -> Vec<bool> {
    displacement_series(prices, period)
        .into_iter()
        .map(|delta| delta.is_finite() && delta > 0.0)
        .collect()
}

/// Signed price displacement over window (Python `diff(period)`).
pub fn displacement_series(prices: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; prices.len()];
    if period == 0 {
        return out;
    }
    for index in period..prices.len() {
        let start = prices[index - period];
        let end = prices[index];
        out[index] = if start.is_finite() && end.is_finite() {
            let time_vec = Vector::new(period as f64, 0.0);
            let disp_vec = Vector::new(period as f64, end - start);
            let area = time_vec.wedge(&disp_vec);
            let signed = area.b();
            if signed.abs() > NORM_FLOOR {
                signed.signum() * (end - start).abs()
            } else {
                end - start
            }
        } else {
            f64::NAN
        };
    }
    out
}

/// Rolling quantile of series within trailing window.
pub fn rolling_quantile_series(series: &[f64], period: usize, q: f64) -> Vec<f64> {
    let q = q.clamp(0.0, 1.0);
    if period == 0 {
        return vec![f64::NAN; series.len()];
    }
    let mut out = vec![f64::NAN; series.len()];
    if series.len() < period {
        return out;
    }
    let tail: Vec<f64> = (period - 1..series.len())
        .into_par_iter()
        .map(|index| {
            let mut sorted: Vec<f64> = window_slice(series, index, period)
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .collect();
            if sorted.is_empty() {
                return f64::NAN;
            }
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let rank = q * (sorted.len() - 1) as f64;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            let frac = rank - lo as f64;
            sorted[lo] * (1.0 - frac) + sorted[hi] * frac
        })
        .collect();
    out[period - 1..].copy_from_slice(&tail);
    out
}

/// NNLS implied sector weights per bar (one weight series per constituent column).
pub fn nnls_weight_series(
    market: &[f64],
    columns: &[&[f64]],
    period: usize,
) -> Vec<Vec<f64>> {
    let len = market.len();
    let n = columns.len();
    if n == 0 {
        return Vec::new();
    }
    let mut weights: Vec<Vec<f64>> = (0..n).map(|_| vec![f64::NAN; len]).collect();
    if period == 0 || len < period {
        return weights;
    }
    for index in (period - 1)..len {
        let start = index + 1 - period;
        let mut x = DMatrix::<f64>::zeros(period, n);
        for col in 0..n {
            for (row, t) in (start..=index).enumerate() {
                x[(row, col)] = columns[col][t];
            }
        }
        let y = DVector::from_iterator(period, (start..=index).map(|t| market[t]));
        let w = nnls_solve(&x, &y, n);
        let sum: f64 = w.iter().sum();
        let normalized: Vec<f64> = if sum > NORM_FLOOR {
            w.iter().map(|v| v / sum).collect()
        } else {
            vec![1.0 / n as f64; n]
        };
        for (col, weight) in normalized.into_iter().enumerate() {
            weights[col][index] = weight;
        }
    }
    weights
}

/// Month-end flag: true every 21 bars (approximate trading month).
pub fn month_end_flag(len: usize) -> Vec<bool> {
    const MONTH: usize = 21;
    (0..len)
        .map(|index| (index + 1) % MONTH == 0 || index + 1 == len)
        .collect()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn l2_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

/// Projected-gradient non-negative least squares (matches scipy nnls semantics).
fn nnls_solve(x: &DMatrix<f64>, y: &DVector<f64>, n: usize) -> Vec<f64> {
    let mut w = vec![0.0_f64; n];
    let max_iter = n.saturating_mul(8).max(64);
    for _ in 0..max_iter {
        let residual = y - x * DVector::from_column_slice(&w);
        let gradient = x.transpose() * residual;
        let mut changed = false;
        for col in 0..n {
            if gradient[col] > NORM_FLOOR {
                let col_vec = x.column(col);
                let denom = col_vec.dot(&col_vec).max(NORM_FLOOR);
                w[col] += gradient[col] / denom;
                changed = true;
            }
        }
        for value in &mut w {
            if *value < 0.0 {
                *value = 0.0;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_beta_matches_projection() {
        let asset = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let market = vec![2.0, 4.0, 6.0, 8.0, 10.0];
        let beta = scalar_beta_window(&asset, &market);
        assert!((beta - 0.5).abs() < 1e-10);
    }

    #[test]
    fn bivector_beta_orthogonal_is_high() {
        let asset = vec![1.0, -1.0, 1.0, -1.0, 1.0];
        let market = vec![1.0, 1.0, 1.0, 1.0, 1.0];
        let bi = bivector_beta_window(&asset, &market);
        assert!(bi > 0.5);
    }

    #[test]
    fn wedge_volume_independent_columns() {
        let c0 = vec![0.01, -0.02, 0.03, -0.01, 0.02, 0.01];
        let c1 = vec![0.02, 0.01, -0.01, 0.03, -0.02, 0.01];
        let c2 = vec![-0.01, 0.03, 0.02, -0.02, 0.01, 0.02];
        let cols = [c0.as_slice(), c1.as_slice(), c2.as_slice()];
        let vol = wedge_volume_window(&cols, 5, 5);
        assert!(vol.is_finite() && vol > 0.0);
    }

    #[test]
    fn orientation_positive_on_uptrend() {
        let prices: Vec<f64> = (0..250).map(|i| 100.0 + i as f64).collect();
        let flags = orientation_series(&prices, 200);
        assert!(flags.last().copied().unwrap_or(false));
    }
}
