# System Requirements Document (SRD)
## Playhead Removal Clean Slate

**Document Version:** 10.0.0  
**Target:** `crates/pulsar_marketlab` / `crates/pulsar_marketlab_ui`  

---

## 1. Intent

Remove interactive playhead scrubbing, timeline transport controls, and the dopesheet bar matrix. The workstation now displays **terminal-bar** portfolio metrics (end of full historical sweep) while preserving full-range `execute_timeline` backtests.

## 2. Removed

- `playhead_current`, transport bar, matrix scrubbing, `WorkstationTimelineHost`
- Engine in/out frame windowing — sweeps always use full OHLC history
- Session snapshot `playhead_current` field

## 3. Retained

- Bottom panel: USD layer stack + logical strategy hierarchy (no value matrix)
- `historical_bar_count` for bar-series length
- `terminal_bar_index()` for inspector/chart lookups
- Canvas debounce and interaction gating (unchanged)

## 4. Future Interactivity

Time navigation will return via a new viewport model (not legacy playhead). Until then, all analytics reflect the **last bar** of the loaded series.
