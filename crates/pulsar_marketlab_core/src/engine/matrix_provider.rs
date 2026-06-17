//! Pre-computed rolling covariance tensors and allocation-free matrix views for tier sweeps.

use std::collections::HashMap;

use nalgebra::DMatrix;

/// Default rolling lookback when a portfolio prim does not specify a window.
pub const DEFAULT_COVARIANCE_LOOKBACK: usize = 20;

/// Stateless, allocation-free view over a pre-built covariance tensor cache.
pub struct RollingMatrixWindow<'a> {
    /// Ordered asset paths matching row/column indices in each covariance matrix.
    pub asset_paths: &'a [String],
    pub lookback_period: usize,
    matrices: &'a [DMatrix<f64>],
}

impl<'a> RollingMatrixWindow<'a> {
    /// Construct a view over an external matrix tensor slice (e.g. custom provider tables).
    pub fn new(
        asset_paths: &'a [String],
        lookback_period: usize,
        matrices: &'a [DMatrix<f64>],
    ) -> Self {
        Self {
            asset_paths,
            lookback_period,
            matrices,
        }
    }

    /// Preferred constructor: borrow the pre-computed tensor block directly.
    pub fn from_cache(cache: &'a PrecomputedMatrixCache) -> Self {
        Self::new(&cache.asset_paths, cache.lookback_period, &cache.matrices)
    }

    /// Borrow a pre-computed covariance matrix for `bar_index` (O(1), no heap traffic).
    #[inline]
    pub fn get_covariance_matrix(&self, bar_index: usize) -> &'a DMatrix<f64> {
        &self.matrices[bar_index.min(self.matrices.len().saturating_sub(1))]
    }
}

/// Timeline tensor of covariance matrices built once before the bar-by-bar sweep.
pub struct PrecomputedMatrixCache {
    pub asset_paths: Vec<String>,
    pub lookback_period: usize,
    matrices: Vec<DMatrix<f64>>,
    pub path_to_index: HashMap<String, usize>,
}

impl PrecomputedMatrixCache {
    pub fn lookback_period(&self) -> usize {
        self.lookback_period
    }

    pub fn total_bars(&self) -> usize {
        self.matrices.len()
    }

    #[inline]
    pub fn matrix_at(&self, bar_index: usize) -> &DMatrix<f64> {
        &self.matrices[bar_index.min(self.matrices.len().saturating_sub(1))]
    }

    pub fn path_index(&self, prim_path: &str) -> Option<usize> {
        self.path_to_index.get(prim_path).copied()
    }

    /// Pre-compute covariance matrices for every bar from historical price columns.
    pub fn build_from_vectors(
        asset_paths: &[String],
        price_vectors: &HashMap<String, &[f64]>,
        total_bars: usize,
        lookback_period: usize,
    ) -> Self {
        let mut path_to_index = HashMap::new();
        for (idx, path) in asset_paths.iter().enumerate() {
            path_to_index.insert(path.clone(), idx);
        }

        let num_assets = asset_paths.len();
        let mut matrices = vec![DMatrix::zeros(num_assets, num_assets); total_bars.max(1)];

        if total_bars == 0 || num_assets == 0 || lookback_period < 2 {
            return Self {
                asset_paths: asset_paths.to_vec(),
                lookback_period,
                matrices,
                path_to_index,
            };
        }

        let mut returns_matrix = DMatrix::zeros(lookback_period, num_assets);

        for bar in lookback_period..total_bars {
            for asset_idx in 0..num_assets {
                let path = &asset_paths[asset_idx];
                let Some(prices) = price_vectors.get(path) else {
                    continue;
                };
                for step in 0..lookback_period {
                    let t_idx = bar - lookback_period + step;
                    if t_idx > 0 && t_idx < prices.len() {
                        let ratio = prices[t_idx] / prices[t_idx - 1];
                        let log_return = ratio.ln();
                        returns_matrix[(step, asset_idx)] =
                            if log_return.is_finite() { log_return } else { 0.0 };
                    }
                }
            }

            let mut cov = DMatrix::zeros(num_assets, num_assets);
            let denom = (lookback_period.saturating_sub(1)).max(1) as f64;

            for i in 0..num_assets {
                let mut mean_i = 0.0;
                for step in 0..lookback_period {
                    mean_i += returns_matrix[(step, i)];
                }
                mean_i /= lookback_period as f64;

                for j in i..num_assets {
                    let mut mean_j = 0.0;
                    for step in 0..lookback_period {
                        mean_j += returns_matrix[(step, j)];
                    }
                    mean_j /= lookback_period as f64;

                    let mut sum = 0.0;
                    for step in 0..lookback_period {
                        sum += (returns_matrix[(step, i)] - mean_i)
                            * (returns_matrix[(step, j)] - mean_j);
                    }
                    let val = sum / denom;
                    cov[(i, j)] = val;
                    cov[(j, i)] = val;
                }
            }

            matrices[bar] = cov;
        }

        Self {
            asset_paths: asset_paths.to_vec(),
            lookback_period,
            matrices,
            path_to_index,
        }
    }
}

/// True when the allocation token should use the pre-computed covariance tensor.
#[inline]
pub fn uses_covariance_optimizer(allocation_method: &str) -> bool {
    allocation_method.contains("HierarchicalRiskParity")
        || allocation_method.contains("MeanVariance")
}

/// Fill `out` with normalized weights using a covariance slice (no heap allocation).
pub fn allocation_weights_from_covariance(
    allocation_method: &str,
    cov: &DMatrix<f64>,
    out: &mut [f64],
) {
    let n = out
        .len()
        .min(cov.nrows())
        .min(cov.ncols());
    if n == 0 {
        return;
    }

    if allocation_method.contains("EqualWeight") {
        let uniform = 1.0 / n as f64;
        out[..n].fill(uniform);
        return;
    }

    let mut sum = 0.0_f64;
    for i in 0..n {
        let variance = cov[(i, i)].max(1e-12);
        let score = if allocation_method.contains("MeanVariance") {
            1.0 / variance
        } else {
            1.0 / variance.sqrt()
        };
        out[i] = score;
        sum += score;
    }

    if sum > f64::EPSILON {
        for weight in &mut out[..n] {
            *weight /= sum;
        }
    } else {
        let uniform = 1.0 / n as f64;
        out[..n].fill(uniform);
    }
}

/// Copy a principal sub-matrix of `full` into row-major `scratch` for leg-local solves.
pub fn fill_subcovariance_block(
    full: &DMatrix<f64>,
    path_to_index: &HashMap<String, usize>,
    source_paths: &[impl AsRef<str>],
    scratch: &mut [f64],
    leg_count: usize,
) {
    let n = leg_count;
    if scratch.len() < n * n {
        return;
    }
    scratch[..n * n].fill(0.0);
    for (row, path_row) in source_paths.iter().take(n).enumerate() {
        let Some(&i) = path_to_index.get(path_row.as_ref()) else {
            continue;
        };
        for (col, path_col) in source_paths.iter().take(n).enumerate() {
            let Some(&j) = path_to_index.get(path_col.as_ref()) else {
                continue;
            };
            scratch[row * n + col] = full[(i, j)];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rolling_matrix_provider_allocation_free_borrow() {
        let asset_paths = vec![
            "/MarketLab/Universe/node_01".to_string(),
            "/MarketLab/Universe/node_02".to_string(),
        ];

        let mut price_vectors = HashMap::new();
        let asset_01_prices = vec![100.0, 101.0, 102.0, 103.0, 104.0, 105.0];
        let asset_02_prices = vec![50.0, 49.0, 48.0, 47.0, 46.0, 45.0];

        price_vectors.insert(asset_paths[0].clone(), asset_01_prices.as_slice());
        price_vectors.insert(asset_paths[1].clone(), asset_02_prices.as_slice());

        let cache = PrecomputedMatrixCache::build_from_vectors(&asset_paths, &price_vectors, 6, 3);

        let provider = RollingMatrixWindow::from_cache(&cache);

        let cov_matrix = provider.get_covariance_matrix(4);

        assert_eq!(cov_matrix.nrows(), 2);
        assert_eq!(cov_matrix.ncols(), 2);
        assert!(cov_matrix[(0, 1)] == cov_matrix[(1, 0)]);
    }

    #[test]
    fn covariance_weights_normalize_to_one() {
        let cov = DMatrix::from_row_slice(2, 2, &[0.04, 0.01, 0.01, 0.09]);
        let mut weights = [0.0; 2];
        allocation_weights_from_covariance("Allocation::HierarchicalRiskParity", &cov, &mut weights);
        let sum: f64 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }
}
