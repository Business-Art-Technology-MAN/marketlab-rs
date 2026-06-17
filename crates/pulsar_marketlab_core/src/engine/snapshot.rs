//! Timeline snapshot utilities: O(1) frame → calendar date lookups for hot UI paths.

/// Maps absolute frame indices to formatted calendar date strings (e.g. `2026-04-22`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoricalTimelineMap {
    pub frame_to_date: Vec<String>,
}

impl HistoricalTimelineMap {
    pub fn from_dates(dates: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            frame_to_date: dates.into_iter().map(Into::into).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.frame_to_date.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frame_to_date.is_empty()
    }

    /// Resolve a frame index to a display date, falling back to zero-padded indices.
    pub fn get_date(&self, frame: usize) -> String {
        self.frame_to_date
            .get(frame)
            .cloned()
            .unwrap_or_else(|| format!("{frame:03}"))
    }
}

/// Adaptive chronological stride for timeline rulers based on per-cell viewport width.
pub fn chronological_stride(cell_width: f32) -> usize {
    if cell_width < 15.0 {
        10
    } else if cell_width < 40.0 {
        5
    } else {
        2
    }
}

/// Compress ISO dates to `MM-DD` when cells are too narrow for full timestamps.
pub fn format_timeline_tick(full_date: &str, cell_width: f32) -> String {
    if cell_width < 55.0 && full_date.len() == 10 && full_date.as_bytes().get(4) == Some(&b'-') {
        full_date[5..10].to_string()
    } else {
        full_date.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_date_falls_back_to_frame_index() {
        let map = HistoricalTimelineMap::from_dates(["2026-01-01"]);
        assert_eq!(map.get_date(0), "2026-01-01");
        assert_eq!(map.get_date(3), "003");
    }

    #[test]
    fn compresses_iso_dates_when_narrow() {
        assert_eq!(
            format_timeline_tick("2026-04-22", 40.0),
            "04-22"
        );
        assert_eq!(
            format_timeline_tick("2026-04-22", 80.0),
            "2026-04-22"
        );
    }
}
