# MarketLab SRD - UI & Engine Upgrade: Synchronized Chart Playhead & Scrubbing
**Target File Locations:** `crates/pulsar_marketlab/src/main.rs`, `ohlc_chart_pane.rs`

---

### 1. Objective
Implement a synchronized global playhead that renders as an interactive, draggable vertical line directly inside the OHLC Chart Pane, anchoring all backend technical analysis and portfolio return logic to a deterministic index.

### 2. Functional Requirements

#### A. Chart Playhead Vector Rendering
* **Canvas Line Projection:** Inside `ohlc_chart_pane.rs`, map the current workspace frame index (`playhead_current`) to its local absolute pixel coordinate:
  $$X_{playhead} = \text{Canvas Left} + \left( \frac{\text{playhead\_current}}{\text{total\_bars}} \times \text{Canvas Width} \right)$$
* **Visual Style Profile:** Use GPUI's native drawing tool to stroke a 1.5px vertical timeline indicator line through the entire height of the candlestick pane using a prominent amber color token (`0xf59e0b`).

#### B. Mouse Drag Input Processing (Scrubbing Interaction)
* **Hit-Test Range Interaction:** Implement mouse interaction handling within the chart canvas coordinate bounds. When an `on_mouse_down` drag sequence is initialized near the $X_{playhead}$ coordinate position, capture the pointer.
* **Coordinate Inversion Mapping:** On mouse move, invert the pixel offset back into a clean dataset array index:
  $$\text{index}_{target} = \frac{X_{mouse} - \text{Canvas Left}}{\text{Canvas Width}} \times \text{total\_bars}$$
* **State Updates:** Clamp the resulting index safely between `0` and `total_bars - 1`, assign it directly to `playhead_current`, and invoke an immediate global view layout re-render.

#### C. Bounded Engine Evaluation (Fixing Sharpe Drift)
* **Static Array Constraints:** Modify `compute_portfolio_diagnostics` and technical analysis evaluations to read data windows *exclusively bounded* within the index subset `0 ..= playhead_current`.
* **State Isolation:** Decouple the 16ms poll worker from unconstrained array pushing. The `TaExecutionBridge` and tracking matrices must reference the explicit static slices dictated by the global playhead index location, eliminating continuous memory accumulation and calculation drift.