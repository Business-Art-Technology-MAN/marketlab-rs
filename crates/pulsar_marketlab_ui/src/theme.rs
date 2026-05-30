//! DCC workstation chrome: low-contrast dark palette and node shell geometry.

use gpui::{px, Hsla, Pixels};

// ── Surfaces ──────────────────────────────────────────────────────────────────

pub const CANVAS_BACKPLATE: u32 = 0x1b1b1f;
pub const PANE_BACKPLATE: u32 = 0x1b1b1f;
pub const ROW_BACKPLATE_A: u32 = 0x1e1e22;
pub const ROW_BACKPLATE_B: u32 = 0x1b1b1f;

// ── Node hulls ────────────────────────────────────────────────────────────────

pub const NODE_HULL: u32 = 0x2d2d32;
pub const NODE_HULL_SELECTED: u32 = 0x3b3b42;
pub const NODE_HEADER: u32 = 0x3d3d44;
pub const NODE_BORDER: u32 = 0x121214;

// ── Grid & dividers ───────────────────────────────────────────────────────────

pub const GRID_MAJOR: u32 = 0x32323a;
pub const GRID_MINOR: u32 = 0x222226;
pub const GRID_MAJOR_SPACING_PX: f32 = 100.0;
pub const GRID_MINOR_SPACING_PX: f32 = 20.0;

/// Focused node selection halo (accent blue).
pub const NODE_SELECTION_HALO: u32 = 0x3b82f6;

// ── Typography ────────────────────────────────────────────────────────────────

pub const TEXT_PRIMARY: u32 = 0xe5e5ea;
pub const TEXT_SECONDARY: u32 = 0x8e8e93;

// ── Recessed node-body controls ───────────────────────────────────────────────

pub const CONTROL_BG: u32 = 0x1a1a1e;
pub const CONTROL_TEXT: u32 = 0xe5e5ea;
pub const CONTROL_FOCUS: u32 = 0x3b82f6;
pub const CONTROL_CARET: u32 = 0x8e8e93;
pub const CONTROL_BORDER: u32 = 0x26262b;
pub const CONTROL_HOVER: u32 = 0x222228;

// ── Tabs & accents ────────────────────────────────────────────────────────────

pub const TAB_ACTIVE: u32 = 0x3d3d44;
pub const TAB_IDLE: u32 = 0x1e1e22;
pub const TAB_BORDER: u32 = 0x26262b;
pub const TREE_ROW_SELECTED: u32 = 0x3b3b42;

// ── Node shell geometry ───────────────────────────────────────────────────────

/// Default expanded node corner radius (`rounded_md`).
pub const NODE_CORNER_RADIUS_PX: f32 = 8.0;

/// Blender capsule width (`w_180`).
pub const CAPSULE_WIDTH: f32 = 180.0;

/// Blender capsule height (`h_7` ≈ 1.75rem).
pub const CAPSULE_HEIGHT: f32 = 28.0;

/// Socket inset from the capsule left/right perimeter.
pub const CAPSULE_SOCKET_INSET: f32 = 4.0;

pub fn chrome_color(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

pub fn node_corner_radius() -> Pixels {
    px(NODE_CORNER_RADIUS_PX)
}

/// Which long edge of a collapsed capsule hosts a socket cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsuleSocketSide {
    Input,
    Output,
}

/// World-space anchor for a socket on a collapsed pill shell perimeter.
pub fn capsule_socket_world_center(
    node_x: f32,
    node_y: f32,
    side: CapsuleSocketSide,
    port_index: usize,
    port_count: usize,
) -> (f32, f32) {
    let width = CAPSULE_WIDTH;
    let height = CAPSULE_HEIGHT;
    let center_y = node_y + height * 0.5;
    let x = match side {
        CapsuleSocketSide::Input => node_x + CAPSULE_SOCKET_INSET,
        CapsuleSocketSide::Output => node_x + width - CAPSULE_SOCKET_INSET,
    };
    let y = if port_count <= 1 {
        center_y
    } else {
        let usable = (height - CAPSULE_SOCKET_INSET * 2.0).max(1.0);
        let step = usable / (port_count.saturating_sub(1)) as f32;
        node_y + CAPSULE_SOCKET_INSET + port_index as f32 * step
    };
    (x, y)
}
