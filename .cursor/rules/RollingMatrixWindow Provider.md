# RollingMatrixWindow Provider

## Split-plane rules
- **Sweep activation**: `PrecomputedMatrixCache::build_from_vectors` allocates the full covariance tensor once.
- **Bar loop**: `RollingMatrixWindow::get_covariance_matrix` and `allocation_weights_from_covariance` borrow pre-sized buffers only — no `DMatrix::zeros` inside tier nodes.

## Portfolio tier wiring
- `ExecutionContext::covariance_cache` is attached in `TimelineTierWorkspace::new`.
- `PortfolioExecutionEngine::apply_covariance_weights` runs when `inputs:id` contains `HierarchicalRiskParity` or `MeanVariance`.
- Sub-covariance blocks are extracted via `fill_subcovariance_block` into `subcov_scratch` (sized at compile time).
