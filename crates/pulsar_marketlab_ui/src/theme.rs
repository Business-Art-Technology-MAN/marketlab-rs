//! DCC workstation chrome: low-contrast dark palette and node shell geometry.

use gpui::{px, Hsla, Pixels};

// ── Surfaces ──────────────────────────────────────────────────────────────────

pub const CANVAS_BACKPLATE: u32 = 0x1b1b1f;
pub const PANE_BACKPLATE: u32 = 0x1b1b1f;
/// Alias for pane / inspector panel backgrounds.
pub const PANE_BG: u32 = PANE_BACKPLATE;
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
/// Chart / viewport ledger grid lines (subtle, non-competing).
pub const GRID_LINE: u32 = GRID_MINOR;
pub const GRID_MAJOR_SPACING_PX: f32 = 100.0;
pub const GRID_MINOR_SPACING_PX: f32 = 20.0;

/// Focused node selection halo (accent blue).
pub const NODE_SELECTION_HALO: u32 = 0x3b82f6;
/// Universal DCC accent (selection outlines, primary actions).
pub const ACCENT_DCC_BLUE: u32 = NODE_SELECTION_HALO;

// ── Typography ────────────────────────────────────────────────────────────────

pub const TEXT_PRIMARY: u32 = 0xe5e5ea;
pub const TEXT_SECONDARY: u32 = 0x8e8e93;

// ── Recessed node-body controls ───────────────────────────────────────────────

pub const CONTROL_BG: u32 = 0x32323a;
pub const CONTROL_TEXT: u32 = 0xf2f2f7;
pub const CONTROL_FOCUS: u32 = 0x3b82f6;
pub const CONTROL_CARET: u32 = 0xaeaeb2;
pub const CONTROL_BORDER: u32 = 0x48484f;
pub const CONTROL_HOVER: u32 = 0x3d3d44;

/// Code editor surface (OTL Script Editor tab).
pub const CODE_EDITOR_BG: u32 = 0x141417;
pub const CODE_EDITOR_GUTTER: u32 = 0x111114;

// ── Tabs & accents ────────────────────────────────────────────────────────────

pub const TAB_ACTIVE: u32 = 0x3d3d44;
pub const TAB_IDLE: u32 = 0x1e1e22;
pub const TAB_BORDER: u32 = 0x26262b;
pub const TREE_ROW_SELECTED: u32 = 0x3b3b42;
/// Inspector row hover (spreadsheet, ledger lists).
pub const ROW_HOVER_BG: u32 = CONTROL_HOVER;
/// Selected / active inspector row.
pub const ROW_ACTIVE_BG: u32 = TREE_ROW_SELECTED;

/// AOV / auxiliary signal socket accent.
pub const SOCKET_AOV: u32 = 0x22d3ee;

// ── Menu bar & chips ──────────────────────────────────────────────────────────

pub const TOOLBAR_BG: u32 = 0x050506;
pub const TOOLBAR_BORDER: u32 = 0x27272a;
pub const TOOLBAR_HOVER_BG: u32 = ROW_BACKPLATE_A;
pub const TEXT_HINT: u32 = 0x52525b;

pub const CHIP_IDLE_BG: u32 = TAB_IDLE;
pub const CHIP_ACTIVE_BG: u32 = TAB_ACTIVE;
pub const CHIP_HOVER_BG: u32 = CONTROL_HOVER;
pub const CHIP_BORDER: u32 = TAB_BORDER;

// ── Ledger spreadsheet ────────────────────────────────────────────────────────

pub const LEDGER_HEADER: u32 = 0x1c1c21;
pub const LEDGER_ROW_A: u32 = CODE_EDITOR_BG;
pub const LEDGER_ROW_B: u32 = CODE_EDITOR_GUTTER;
pub const LEDGER_ROW_RISK: u32 = 0x1a1520;
pub const LEDGER_ROW_PORTFOLIO: u32 = 0x102018;
pub const LEDGER_BORDER: u32 = 0x222227;
/// Muted panel borders (inspector cards, viewport ledger).
pub const BORDER_MUTED: u32 = LEDGER_BORDER;
pub const LEDGER_SURFACE: u32 = 0x0f0f12;
pub const TEXT_MUTED: u32 = 0x71717a;
pub const LEDGER_ACCENT: u32 = 0x38bdf8;
pub const PNL_POSITIVE: u32 = 0x10b981;
pub const PNL_NEGATIVE: u32 = 0xf87171;
pub const RISK_WEIGHT_HIGHLIGHT: u32 = 0xf59e0b;

// ── Chart & wire semantics ────────────────────────────────────────────────────

pub const CANDLE_BULL: u32 = 0x26a69a;
pub const CANDLE_BEAR: u32 = 0xef5350;
pub const WIRE_STRUCTURAL: u32 = TEXT_MUTED;
pub const WIRE_SIGNAL: u32 = 0xa78bfa;
pub const WIRE_PORTFOLIO: u32 = 0x34d399;
pub const SIGNAL_BUY: u32 = WIRE_PORTFOLIO;
pub const SIGNAL_SELL: u32 = CANDLE_BEAR;
pub const CHART_PANE_BG: u32 = LEDGER_SURFACE;
pub const CHART_PANE_BORDER: u32 = 0x2d2d34;

// ── Wealth chart & viewport ───────────────────────────────────────────────────

pub const EQUITY_CURVE_PRIMARY: u32 = WIRE_PORTFOLIO;
pub const EQUITY_PEAK_LINE: u32 = 0x64748b;
pub const REGIME_BAND: u32 = 0x6366f1;
pub const VIEWPORT_CLEAR: u32 = 0x0c0c0e;
pub const VIEWPORT_TRACK_BG: u32 = 0x18181b;
pub const PANEL_SUBTLE_BG: u32 = 0x101014;
pub const LABEL_EMPHASIS: u32 = 0x94a3b8;
pub const TEXT_ON_ACCENT: u32 = 0xffffff;
pub const BENCHMARK_VALUE: u32 = 0xe2e8f0;
pub const BODY_EMPHASIS: u32 = 0xd4d4d8;

// ── Alerts (wiring validation) ────────────────────────────────────────────────

pub const ALERT_BG: u32 = 0x450a0a;
pub const ALERT_BORDER: u32 = 0x991b1b;
pub const ALERT_TEXT: u32 = 0xfca5a5;

// ── Workstation shell ───────────────────────────────────────────────────────────

pub const WORKSTATION_ROOT: u32 = 0x09090b;
pub const SPLIT_HANDLE: u32 = TOOLBAR_BORDER;
pub const SPLIT_HANDLE_HOVER: u32 = 0x3f3f46;
pub const STAGE_TREE_HOVER: u32 = SPLIT_HANDLE_HOVER;

// ── Stage ledger & tree ─────────────────────────────────────────────────────────

pub const STAGE_LEDGER_BG: u32 = 0x111113;
pub const STAGE_LEDGER_BORDER: u32 = 0x1f1f23;
pub const STAGE_WARNING: u32 = 0xfbbf24;
pub const STAGE_WARNING_BG: u32 = 0x422006;
pub const SOCKET_PIN_BORDER: u32 = 0x18181b;
pub const TAB_HOVER_IDLE: u32 = VIEWPORT_TRACK_BG;

pub mod buttons;

/// Alpha-blended DCC token (drawdown fills, regime bands).
pub fn color_with_alpha(hex: u32, alpha: f32) -> Hsla {
    let mut color = chrome_color(hex);
    color.a = alpha;
    color
}

// ── Node shell geometry ───────────────────────────────────────────────────────

/// Default expanded node corner radius (`rounded_md`).
pub const NODE_CORNER_RADIUS_PX: f32 = 8.0;

/// Default expanded node card width (must match host `NODE_WIDTH`).
pub const NODE_CARD_WIDTH: f32 = 220.0;

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

/// Align gpui-component candlestick bullish/bearish colors with DCC chart tokens.
pub fn apply_chart_candle_accents(cx: &mut gpui::App) {
    use gpui_component::Theme;
    let theme = Theme::global_mut(cx);
    theme.bullish = chrome_color(CANDLE_BULL);
    theme.bearish = chrome_color(CANDLE_BEAR);
}
