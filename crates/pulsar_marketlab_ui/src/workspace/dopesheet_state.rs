//! Persistent UI layout state for the topology dopesheet.
//!
//! Isolated from [`WorkspaceContext`] and engine sweep lifecycles so disclosure
//! twists and scroll position survive stage recomposition and backtest runs.

use std::collections::HashSet;

use gpui::{Pixels, ScrollHandle};

/// Visual layout state for the topology dopesheet (not tied to USD or engine snapshots).
#[derive(Clone, Debug)]
pub struct DopesheetUiState {
    /// User toggles for triangle twists (semantics depend on [`Self::is_expanded`]).
    expanded_paths: HashSet<String>,
    /// Host-owned vertical scroll (shared between hierarchy + matrix row stacks).
    pub vertical_scroll_handle: ScrollHandle,
    /// Host-owned horizontal scroll (shared between date header + matrix rows).
    pub matrix_horizontal_scroll_handle: ScrollHandle,
    /// Last measured width of the matrix viewport (for horizontal cell clipping).
    pub matrix_viewport_width: Pixels,
}

impl Default for DopesheetUiState {
    fn default() -> Self {
        Self {
            expanded_paths: HashSet::new(),
            vertical_scroll_handle: ScrollHandle::default(),
            matrix_horizontal_scroll_handle: ScrollHandle::default(),
            matrix_viewport_width: Pixels::ZERO,
        }
    }
}

impl DopesheetUiState {
    pub fn set_matrix_viewport_width(&mut self, width: Pixels) {
        self.matrix_viewport_width = width;
    }

    /// Shallow tree rows default open; deeper rows default closed unless toggled.
    pub fn is_expanded(&self, path: &str, tree_depth: usize) -> bool {
        if tree_depth <= 1 {
            !self.expanded_paths.contains(path)
        } else {
            self.expanded_paths.contains(path)
        }
    }

    pub fn toggle_expansion(&mut self, path: &str) {
        if self.expanded_paths.contains(path) {
            self.expanded_paths.remove(path);
        } else {
            self.expanded_paths.insert(path.to_string());
        }
    }
}
