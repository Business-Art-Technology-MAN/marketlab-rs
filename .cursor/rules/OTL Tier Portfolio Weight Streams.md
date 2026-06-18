# OTL Tier Portfolio Weight Streams

## Split-plane invariant
- **Compile time** (`compile_portfolio_engine`, `ObjectCodegenRegistry`): capture structural metadata only — `source_prim_paths`, `portfolio_prim_path`, leg counts. No `serialize_portfolio_weights`, no price math.
- **Runtime** (`PortfolioExecutionEngine::track_portfolio_metrics_at_bar`): normalize allocator leg weights, encode via `serialize_portfolio_weights_from_slices`, store in `weight_encodings`, optionally call `ExecutionContext::weight_track`.

## Side channels
- `MarketTimelineWindow::track_token` + `set_current_frame` for deferred closure drivers.
- `TimelineExecutionResult::token_streams` attribute `outputs:weights` populated from tier portfolio sweeps (parity with legacy `integrate_portfolio`).

## Registry wiring
`build_tier_compile_registry` must receive upstream prim paths from `inputs:sources` graph edges (`index_to_path`), not display names.
