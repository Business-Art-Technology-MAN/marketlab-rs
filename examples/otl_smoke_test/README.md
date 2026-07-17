# OTL smoke test

Minimal script to verify the OTL editor, canonical grammar, and compile console feed.

## Script

`examples/otl_smoke_test/sma_smoke.otl` — 3-period SMA on `source` (the wired asset close series).

## Try it in the finance editor

1. Run `cargo run --manifest-path crates/marketlab_finance_editor/Cargo.toml`
2. Open the **OTL Editor** tab
3. **Open** → select `examples/otl_smoke_test/sma_smoke.otl`
4. Click **Compile Script** (or add an OTL Operator on the canvas, paste the script, compile)

**Console feed (bottom of OTL Editor):**

- Success: `[ OK: Compiled Series Closure ] N ms`
- Preview line: `feed preview (last 3 bars): …` — SMA evaluated on a synthetic 100→107 price ramp

After compile, the editor rewrites the buffer to canonical `shader … { … }` form.

## CLI compile check

```bash
cargo test -p pulsar_marketlab_core sma_smoke -- --nocapture
```
