//! Phase B Pillar 2 — asynchronous mock FIX ingestion bridge.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const FIX_TICKS_PATH: &str = "/execution/fix/ticks";
pub const FIX_LAST_PRICE_ATTR: &str = "last_price";
pub const FIX_LAST_QTY_ATTR: &str = "last_qty";

const FIX_EMIT_INTERVAL: Duration = Duration::from_millis(80);
const FIX_TICKS_PER_BAR: u32 = 4;

/// One stage write emitted by the mock FIX bridge (mirrored to UI via message bus).
#[derive(Debug, Clone, PartialEq)]
pub struct FixStageWrite {
    pub prim_path: String,
    pub attribute: String,
    pub time: f64,
    pub value: f32,
}

/// Shared playhead epoch updated by the CSV feeder thread.
pub struct FixPlayheadClock {
    bar_epoch_bits: AtomicU64,
}

impl FixPlayheadClock {
    pub fn new() -> Self {
        Self {
            bar_epoch_bits: AtomicU64::new(0),
        }
    }

    pub fn set_bar_epoch(&self, epoch: f64) {
        if epoch.is_finite() && epoch > 0.0 {
            self.bar_epoch_bits.store(epoch.to_bits(), Ordering::Release);
        } else {
            self.bar_epoch_bits.store(0, Ordering::Release);
        }
    }

    pub fn bar_epoch(&self) -> Option<f64> {
        let bits = self.bar_epoch_bits.load(Ordering::Acquire);
        if bits == 0 {
            return None;
        }
        let value = f64::from_bits(bits);
        if value.is_finite() && value > 0.0 {
            Some(value)
        } else {
            None
        }
    }
}

impl Default for FixPlayheadClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Simulates a live matching-engine FIX line with dense sub-second ticks relative to the bar epoch.
pub fn spawn_mock_fix_bridge<F>(mut emit: F, clock: Arc<FixPlayheadClock>)
where
    F: FnMut(FixStageWrite) + Send + 'static,
{
    std::thread::spawn(move || {
        let mut tick_in_bar: u32 = 0;
        let mut last_bar_epoch: f64 = 0.0;

        loop {
            std::thread::sleep(FIX_EMIT_INTERVAL);

            let Some(bar_epoch) = clock.bar_epoch() else {
                continue;
            };

            if (bar_epoch - last_bar_epoch).abs() > f64::EPSILON {
                last_bar_epoch = bar_epoch;
                tick_in_bar = 0;
            }

            if tick_in_bar >= FIX_TICKS_PER_BAR {
                continue;
            }
            tick_in_bar += 1;

            // Microsecond-scale offset within the daily bar window.
            let offset = tick_in_bar as f64 * 0.000_153;
            let time = bar_epoch + offset;
            let last_price = 450.0 + tick_in_bar as f32 * 0.02;
            let last_qty = 100.0 + tick_in_bar as f32 * 25.0;

            emit(FixStageWrite {
                prim_path: FIX_TICKS_PATH.to_string(),
                attribute: FIX_LAST_PRICE_ATTR.to_string(),
                time,
                value: last_price,
            });
            emit(FixStageWrite {
                prim_path: FIX_TICKS_PATH.to_string(),
                attribute: FIX_LAST_QTY_ATTR.to_string(),
                time,
                value: last_qty,
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playhead_clock_round_trips_epoch() {
        let clock = FixPlayheadClock::new();
        assert!(clock.bar_epoch().is_none());
        clock.set_bar_epoch(1_706_723_200.0);
        assert_eq!(clock.bar_epoch(), Some(1_706_723_200.0));
    }
}
