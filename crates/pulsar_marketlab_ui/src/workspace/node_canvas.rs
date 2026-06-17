//! Multi-panel node canvas frame: horizontal Blender layout, wiring directives, and sub-canvas tabs.

use gpui::*;
use gpui::prelude::FluentBuilder;

use crate::theme;

// ── DCC chrome palette (re-exported for host crates) ──────────────────────────

pub const DCC_CANVAS_BACKPLATE: u32 = theme::CANVAS_BACKPLATE;
pub const DCC_NODE_HULL: u32 = theme::NODE_HULL;
pub const DCC_NODE_SELECTED: u32 = theme::NODE_HULL_SELECTED;
pub const DCC_HEADER_ACTIVE: u32 = theme::NODE_HEADER;
pub const DCC_TEXT_PRIMARY: u32 = theme::TEXT_PRIMARY;
pub const DCC_TEXT_SECONDARY: u32 = theme::TEXT_SECONDARY;
pub const DCC_BORDER: u32 = theme::NODE_BORDER;
pub const DCC_CAPSULE_WIDTH: f32 = theme::CAPSULE_WIDTH;
pub const DCC_CAPSULE_HEIGHT: f32 = theme::CAPSULE_HEIGHT;
pub const DCC_NODE_CORNER_RADIUS_PX: f32 = theme::NODE_CORNER_RADIUS_PX;

pub use crate::theme::{capsule_socket_world_center, CapsuleSocketSide};

/// Average glyph width as a fraction of font size for semibold UI labels.
const TITLE_CHAR_WIDTH_RATIO: f32 = 0.58;
/// Monospace port labels are slightly wider per character.
const MONO_CHAR_WIDTH_RATIO: f32 = 0.62;

/// Zoom band where node bodies switch to compact single-line labels.
pub const CANVAS_ZOOM_COMPACT: f32 = 0.52;
/// Zoom band where nodes render as header-only / socket-only chrome.
pub const CANVAS_ZOOM_MINIMAL: f32 = 0.24;
/// Expanded header horizontal padding (`p_2` × 2).
const NODE_CARD_HEADER_PAD_X: f32 = 16.0;
/// Accent dot column reserved in the expanded header row.
const NODE_CARD_HEADER_ACCENT_GUTTER: f32 = 16.0;
/// Left inset clearing the collapsed pill input socket cluster.
pub const COLLAPSED_PILL_PAD_LEFT: f32 = 12.0;
/// Right inset inside the collapsed pill shell.
pub const COLLAPSED_PILL_PAD_RIGHT: f32 = 8.0;

/// Which node chrome band owns the title string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeHeaderTitleBudget {
    /// Expanded card header (`text_xs`, `NODE_CARD_WIDTH`).
    ExpandedCard,
    /// Collapsed capsule (`10px`, `CAPSULE_WIDTH`).
    CollapsedCapsule,
}

impl NodeHeaderTitleBudget {
    fn font_size_px(self) -> f32 {
        match self {
            Self::ExpandedCard => 12.0,
            Self::CollapsedCapsule => 10.0,
        }
    }

    fn available_width_px(self) -> f32 {
        match self {
            Self::ExpandedCard => {
                theme::NODE_CARD_WIDTH - NODE_CARD_HEADER_PAD_X - NODE_CARD_HEADER_ACCENT_GUTTER
            }
            Self::CollapsedCapsule => {
                theme::CAPSULE_WIDTH - COLLAPSED_PILL_PAD_LEFT - COLLAPSED_PILL_PAD_RIGHT
            }
        }
    }

    /// Max visible characters before an ellipsis suffix is required.
    pub fn max_chars(self) -> usize {
        let runway = self.available_width_px();
        let char_width = self.font_size_px() * TITLE_CHAR_WIDTH_RATIO;
        if char_width <= f32::EPSILON {
            return 12;
        }
        (runway / char_width).floor().max(4.0) as usize
    }
}

/// How much node chrome to draw at the current zoom scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasZoomDetailLevel {
    /// Full labels, charts, parameters, metrics.
    Full,
    /// Single-line truncated labels; hide charts and parameter grids.
    Compact,
    /// Header + sockets only; no wrapping body text.
    Minimal,
}

pub fn canvas_zoom_detail_level(zoom_scale: f32) -> CanvasZoomDetailLevel {
    if zoom_scale < CANVAS_ZOOM_MINIMAL {
        CanvasZoomDetailLevel::Minimal
    } else if zoom_scale < CANVAS_ZOOM_COMPACT {
        CanvasZoomDetailLevel::Compact
    } else {
        CanvasZoomDetailLevel::Full
    }
}

/// Estimate how many characters fit in a horizontal runway at a given font size.
pub fn max_chars_for_runway(runway_px: f32, font_size_px: f32, mono: bool) -> usize {
    if runway_px <= 4.0 || font_size_px <= f32::EPSILON {
        return 0;
    }
    let ratio = if mono {
        MONO_CHAR_WIDTH_RATIO
    } else {
        TITLE_CHAR_WIDTH_RATIO
    };
    let char_width = font_size_px * ratio;
    (runway_px / char_width).floor().max(1.0) as usize
}

/// Truncate to a single line with an ASCII ellipsis when the runway is too narrow.
pub fn truncate_to_runway(text: &str, runway_px: f32, font_size_px: f32, mono: bool) -> String {
    let max_chars = max_chars_for_runway(runway_px, font_size_px, mono);
    let trimmed = sanitize_node_label_text(text);
    if max_chars == 0 {
        return String::new();
    }
    if trimmed.chars().count() <= max_chars {
        return trimmed;
    }
    if max_chars <= 3 {
        return "...".to_string();
    }
    let prefix_len = max_chars - 3;
    format!(
        "{}...",
        trimmed.chars().take(prefix_len).collect::<String>()
    )
}

/// Collapse whitespace and strip line breaks for single-line pill labels.
pub fn sanitize_node_label_text(name: &str) -> String {
    name.trim()
        .replace(['\n', '\r', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Truncate a node display name for pill / card headers (`…` when longer than the budget).
pub fn truncate_node_header_title(name: &str, budget: NodeHeaderTitleBudget) -> String {
    truncate_to_runway(
        name,
        budget.available_width_px(),
        budget.font_size_px(),
        false,
    )
}

/// Truncate against an on-screen runway (accounts for zoom-scaled card width).
pub fn truncate_node_header_title_at_runway(
    name: &str,
    runway_px: f32,
    font_size_px: f32,
) -> String {
    truncate_to_runway(name, runway_px, font_size_px, false)
}

/// Single-line label: never wraps; parent should constrain width.
pub fn render_canvas_single_line(
    label: impl Into<SharedString>,
    font_size: Pixels,
    text_color: Hsla,
) -> impl IntoElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_shrink()
        .overflow_hidden()
        .text_size(font_size)
        .text_color(text_color)
        .truncate()
        .child(label.into())
}

/// Horizontal text runway inside a collapsed capsule (world units, pre-zoom).
pub fn collapsed_pill_text_width(zoom_scale: f32) -> Pixels {
    px((theme::CAPSULE_WIDTH - COLLAPSED_PILL_PAD_LEFT - COLLAPSED_PILL_PAD_RIGHT) * zoom_scale)
}

/// Single-line title chrome for collapsed capsules (left-aligned, prefix-visible truncation).
pub fn render_collapsed_pill_title(
    label: &str,
    budget: NodeHeaderTitleBudget,
    runway_px: f32,
) -> impl IntoElement {
    let font_size = budget.font_size_px();
    let text = if runway_px > f32::EPSILON {
        truncate_to_runway(label, runway_px, font_size, false)
    } else {
        truncate_node_header_title(label, budget)
    };
    div()
        .w_full()
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_start()
        .overflow_hidden()
        .text_size(px(font_size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::chrome_color(theme::TEXT_PRIMARY))
        .truncate()
        .child(text)
}

/// DOM shell for a collapsed Blender capsule node (`rounded_full`, `h_7`, `w_180`).
pub fn render_collapsed_node_capsule(
    display_left: Pixels,
    display_top: Pixels,
    zoom_scale: f32,
    hull_color: Hsla,
    border_color: Hsla,
    label: impl Into<SharedString>,
    on_mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let pill_width = theme::CAPSULE_WIDTH * zoom_scale;
    let pill_height = theme::CAPSULE_HEIGHT * zoom_scale;
    div()
        .absolute()
        .left(display_left)
        .top(display_top)
        .w(px(pill_width))
        .h(px(pill_height))
        .flex()
        .items_center()
        .justify_start()
        .pl(px(COLLAPSED_PILL_PAD_LEFT * zoom_scale))
        .pr(px(COLLAPSED_PILL_PAD_RIGHT * zoom_scale))
        .bg(hull_color)
        .border_1()
        .border_color(border_color)
        .rounded_full()
        .overflow_hidden()
        .cursor_move()
        .on_mouse_down(MouseButton::Left, on_mouse_down)
        .child(render_collapsed_pill_title(
            label.as_ref(),
            NodeHeaderTitleBudget::CollapsedCapsule,
            (theme::CAPSULE_WIDTH - COLLAPSED_PILL_PAD_LEFT - COLLAPSED_PILL_PAD_RIGHT)
                * zoom_scale,
        ))
}

/// Paint a world-space layout grid (20px minor, 100px major) on the node canvas.
pub fn paint_dcc_canvas_grid(
    bounds: Bounds<Pixels>,
    pan_offset: Point<Pixels>,
    zoom_scale: f32,
    window: &mut Window,
) {
    if zoom_scale <= f32::EPSILON {
        return;
    }

    let origin_x: f32 = bounds.origin.x.into();
    let origin_y: f32 = bounds.origin.y.into();
    let width: f32 = bounds.size.width.into();
    let height: f32 = bounds.size.height.into();
    let pan_x: f32 = pan_offset.x.into();
    let pan_y: f32 = pan_offset.y.into();

    let world_left = (-pan_x) / zoom_scale;
    let world_top = (-pan_y) / zoom_scale;
    let world_right = (width - pan_x) / zoom_scale;
    let world_bottom = (height - pan_y) / zoom_scale;

    let minor = theme::GRID_MINOR_SPACING_PX;
    let major = theme::GRID_MAJOR_SPACING_PX;
    let minor_stroke = rgba(theme::GRID_MINOR);
    let major_stroke = rgba(theme::GRID_MAJOR);

    let x_start = (world_left / minor).floor() as i32;
    let x_end = (world_right / minor).ceil() as i32;
    for i in x_start..=x_end {
        let world_x = i as f32 * minor;
        let screen_x = origin_x + world_x * zoom_scale + pan_x;
        if screen_x < origin_x - 1.0 || screen_x > origin_x + width + 1.0 {
            continue;
        }
        let is_major = (world_x.round() as i32).rem_euclid(major as i32) == 0;
        let stroke = if is_major { major_stroke } else { minor_stroke };
        let mut path = PathBuilder::stroke(px(1.0));
        path.move_to(point(px(screen_x), px(origin_y)));
        path.line_to(point(px(screen_x), px(origin_y + height)));
        if let Ok(built) = path.build() {
            window.paint_path(built, stroke);
        }
    }

    let y_start = (world_top / minor).floor() as i32;
    let y_end = (world_bottom / minor).ceil() as i32;
    for i in y_start..=y_end {
        let world_y = i as f32 * minor;
        let screen_y = origin_y + world_y * zoom_scale + pan_y;
        if screen_y < origin_y - 1.0 || screen_y > origin_y + height + 1.0 {
            continue;
        }
        let is_major = (world_y.round() as i32).rem_euclid(major as i32) == 0;
        let stroke = if is_major { major_stroke } else { minor_stroke };
        let mut path = PathBuilder::stroke(px(1.0));
        path.move_to(point(px(origin_x), px(screen_y)));
        path.line_to(point(px(origin_x + width), px(screen_y)));
        if let Ok(built) = path.build() {
            window.paint_path(built, stroke);
        }
    }
}

// ── Socket paint helpers ──────────────────────────────────────────────────────

/// Semantic socket coloring aligned with three-tier port wire kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketWireKind {
    /// Tier 1 structural / USD path reference.
    StructuralPath,
    /// Tier 2 numeric OTL / TA signal.
    NumericSignal,
    /// Tier 2 arbitrary output variable (AOV).
    Aov,
    /// Terminal integrator / execution sink.
    PortfolioExecution,
}

pub fn socket_color(kind: SocketWireKind) -> Hsla {
    theme::chrome_color(match kind {
        SocketWireKind::StructuralPath => theme::WIRE_STRUCTURAL,
        SocketWireKind::NumericSignal => theme::WIRE_SIGNAL,
        SocketWireKind::Aov => theme::SOCKET_AOV,
        SocketWireKind::PortfolioExecution => theme::WIRE_PORTFOLIO,
    })
}

/// Render a DOM socket pin dot with the wire-kind color.
pub fn socket_pin(kind: SocketWireKind) -> impl IntoElement {
    div()
        .w(px(8.0))
        .h(px(8.0))
        .rounded_full()
        .bg(socket_color(kind))
        .border_1()
        .border_color(rgb(theme::SOCKET_PIN_BORDER))
}

/// Paint a filled socket circle on the canvas layer.
pub fn paint_socket_dot(window: &mut Window, center: Point<Pixels>, kind: SocketWireKind) {
    let radius = px(4.5);
    let diameter = radius * 2.0;
    window.paint_quad(fill(
        Bounds {
            origin: point(center.x - radius, center.y - radius),
            size: size(diameter, diameter),
        },
        socket_color(kind),
    ));
}

/// Paint a cubic bezier wire between two canvas points.
pub fn paint_bezier_wire(
    start: Point<Pixels>,
    end: Point<Pixels>,
    window: &mut Window,
    stroke: Hsla,
) {
    let start_x: f32 = start.x.into();
    let end_x: f32 = end.x.into();
    let start_y: f32 = start.y.into();
    let end_y: f32 = end.y.into();
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let spread = (dx.abs() * 0.5 + dy.abs() * 0.2 + 56.0).clamp(56.0, 240.0);
    let c1_x = if dx >= 0.0 {
        start_x + spread
    } else {
        start_x - spread
    };
    let c2_x = if dx >= 0.0 {
        end_x - spread
    } else {
        end_x + spread
    };
    let mut builder = PathBuilder::stroke(px(1.75));
    builder.move_to(start);
    builder.cubic_bezier_to(
        end,
        point(px(c1_x), start.y),
        point(px(c2_x), end.y),
    );
    if let Ok(path) = builder.build() {
        window.paint_path(path, stroke);
    }
}

/// Generic wire descriptor for painting active graph links.
#[derive(Clone, Debug)]
pub struct GraphWireSegment {
    pub from: Point<Pixels>,
    pub to: Point<Pixels>,
    pub kind: SocketWireKind,
}

pub fn paint_wires_for_graph(window: &mut Window, wires: &[GraphWireSegment]) {
    for wire in wires {
        paint_bezier_wire(wire.from, wire.to, window, socket_color(wire.kind));
    }
}

/// Red alert strip for graph compiler wiring validation failures.
pub fn render_wiring_alerts(messages: &[String]) -> impl IntoElement {
    let mut alerts = div().flex_col().gap_1();
    for message in messages {
        alerts = alerts.child(
            div()
                .px_3()
                .py_1()
                .bg(rgb(theme::ALERT_BG))
                .border_1()
                .border_color(rgb(theme::ALERT_BORDER))
                .text_size(px(10.0))
                .font_family("monospace")
                .text_color(rgb(theme::ALERT_TEXT))
                .child(format!("⚠ wiring: {message}")),
        );
    }

    div().when(!messages.is_empty(), |this| {
        this.absolute()
            .top_10()
            .left_4()
            .right_4()
            .child(alerts)
    })
}

// ── Horizontal Blender layout ─────────────────────────────────────────────────

/// Column width for left-to-right tier flow (Asset → OTL → Integrator).
pub const BLENDER_COLUMN_WIDTH: f32 = 280.0;
pub const BLENDER_ROW_HEIGHT: f32 = 196.0;
pub const BLENDER_ORIGIN_X: f32 = 48.0;
pub const BLENDER_ORIGIN_Y: f32 = 64.0;

/// Tier index: 0 = structural asset, 1 = OTL shader, 2 = terminal integrator.
pub fn blender_tier_index(tier: u8) -> f32 {
    tier as f32
}

/// World `(x, y)` for a node placed in the Blender horizontal paradigm.
pub fn blender_slot_position(tier: u8, row: usize) -> (f32, f32) {
    (
        BLENDER_ORIGIN_X + blender_tier_index(tier) * BLENDER_COLUMN_WIDTH,
        BLENDER_ORIGIN_Y + row as f32 * BLENDER_ROW_HEIGHT,
    )
}

// ── Stage relationship compilation ────────────────────────────────────────────

/// Execution input slot targeted when a wire is dropped on a node port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionSlotKind {
    /// Tier-1 asset path bound to an OTL `inputs:underlying` rel.
    UnderlyingInput,
    /// Tier-2 signal bound to a portfolio `inputs:sources` rel.
    SignalInput,
}

/// Compiled `stage.set_relationship()` directive from a canvas wire drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRelationshipDirective {
    pub target_prim_path: String,
    pub relationship: String,
    pub source_prim_path: String,
}

impl StageRelationshipDirective {
    pub fn as_stage_instruction(&self) -> String {
        format!(
            "stage.set_relationship(\"{}\", \"{}\", \"{}\")",
            self.target_prim_path, self.relationship, self.source_prim_path
        )
    }
}

/// Map a wire drop onto the textual stage relationship directive.
pub fn compile_relationship_directive(
    target_prim_path: &str,
    source_prim_path: &str,
    slot: ExecutionSlotKind,
) -> StageRelationshipDirective {
    let relationship = match slot {
        ExecutionSlotKind::UnderlyingInput => "inputs:underlying".to_string(),
        ExecutionSlotKind::SignalInput => "inputs:sources".to_string(),
    };
    StageRelationshipDirective {
        target_prim_path: target_prim_path.to_string(),
        relationship,
        source_prim_path: source_prim_path.to_string(),
    }
}

/// Infer the execution slot from the downstream prim's composed schema type.
pub fn execution_slot_for_target_type(type_name: &str) -> Option<ExecutionSlotKind> {
    match type_name {
        "OtlOperator" | "OtlTaUberSignal" => Some(ExecutionSlotKind::UnderlyingInput),
        "PortfolioIntegrator" => Some(ExecutionSlotKind::SignalInput),
        _ => None,
    }
}

/// Infer the execution slot from the downstream prim path prefix (legacy fallback).
pub fn execution_slot_for_target_prim(target_prim_path: &str) -> Option<ExecutionSlotKind> {
    if target_prim_path.starts_with("/analytics/") || target_prim_path.contains("/rsi")
        || target_prim_path.contains("/macd")
    {
        return Some(ExecutionSlotKind::UnderlyingInput);
    }
    if target_prim_path.starts_with("/portfolios/") || target_prim_path.contains("Portfolio") {
        return Some(ExecutionSlotKind::SignalInput);
    }
    None
}

// ── Sub-canvas environment tabs ───────────────────────────────────────────────

/// One drill-down tab isolating an aggregator block's constituent graph space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasEnvironmentTab {
    pub label: String,
    /// `None` = root pipeline canvas; `Some` = scoped to an aggregator node id.
    pub scope_node_id: Option<usize>,
    /// USD prim path for the aggregator block (used in tab subtitle).
    pub scope_path: Option<String>,
}

impl CanvasEnvironmentTab {
    pub fn root() -> Self {
        Self {
            label: "Pipeline".to_string(),
            scope_node_id: None,
            scope_path: None,
        }
    }

    pub fn aggregator(label: impl Into<String>, node_id: usize, scope_path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            scope_node_id: Some(node_id),
            scope_path: Some(scope_path.into()),
        }
    }

    pub fn is_root(&self) -> bool {
        self.scope_node_id.is_none()
    }
}

// ── Node canvas pane trait + frame renderer ───────────────────────────────────

pub trait NodeCanvasPane: Sized {
    fn canvas_tabs(&self) -> &[CanvasEnvironmentTab];
    fn active_canvas_tab(&self) -> usize;
    fn set_active_canvas_tab(&mut self, index: usize, cx: &mut Context<Self>);
    fn open_aggregator_canvas(
        &mut self,
        node_id: usize,
        label: String,
        scope_path: String,
        cx: &mut Context<Self>,
    );
    fn wiring_alert_messages(&self) -> Vec<String>;
    /// Bind an upstream output prim onto a downstream input slot (wire release).
    fn connect_primitives(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
        cx: &mut Context<Self>,
    );
    /// Remove a previously bound relationship edge (wire disconnect).
    fn disconnect_primitives(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
        cx: &mut Context<Self>,
    );
    fn render_canvas_graph(&mut self, cx: &mut Context<Self>) -> impl IntoElement;
}

/// Wire-release handler: terminate the visual path and bind USD relationship prims.
pub fn on_wire_released<H: NodeCanvasPane + 'static>(
    view: &Entity<H>,
    source_prim_path: impl Into<String>,
    target_prim_path: impl Into<String>,
    cx: &mut App,
) {
    let source = source_prim_path.into();
    let target = target_prim_path.into();
    view.update(cx, |host, cx| {
        host.connect_primitives(&source, &target, cx);
    });
}

/// Wire-disconnect handler: remove the matching relationship from the stage graph.
pub fn on_wire_disconnected<H: NodeCanvasPane + 'static>(
    view: &Entity<H>,
    source_prim_path: impl Into<String>,
    target_prim_path: impl Into<String>,
    cx: &mut App,
) {
    let source = source_prim_path.into();
    let target = target_prim_path.into();
    view.update(cx, |host, cx| {
        host.disconnect_primitives(&source, &target, cx);
    });
}

fn render_canvas_tab_bar<H: NodeCanvasPane + 'static>(
    view: Entity<H>,
    host: &H,
    _cx: &mut Context<H>,
) -> impl IntoElement {
    let tabs = host.canvas_tabs();
    let active = host.active_canvas_tab().min(tabs.len().saturating_sub(1));
    let mut bar = div().flex().items_center().gap_1().px_3().pt_2().pb_1();

    for (index, tab) in tabs.iter().enumerate() {
        let is_active = index == active;
        let bg = if is_active {
            theme::chrome_color(theme::TAB_ACTIVE)
        } else {
            theme::chrome_color(theme::TAB_IDLE)
        };
        let border = if is_active {
            theme::chrome_color(theme::NODE_HEADER)
        } else {
            theme::chrome_color(theme::TAB_BORDER)
        };
        let text = if is_active {
            theme::chrome_color(theme::TEXT_PRIMARY)
        } else {
            theme::chrome_color(theme::TEXT_SECONDARY)
        };

        let mut tab_label = tab.label.clone();
        if let Some(path) = tab.scope_path.as_deref() {
            tab_label = format!("{tab_label} · {path}");
        }

        bar = bar.child(
            div()
                .id(("canvas-tab", index))
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(bg)
                .border_1()
                .border_color(border)
                .cursor_pointer()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(text)
                .child(tab_label)
                .on_mouse_down(
                    MouseButton::Left,
                    {
                        let view = view.clone();
                        move |_, _window, cx| {
                            view.update(cx, |host, cx| {
                                host.set_active_canvas_tab(index, cx);
                            });
                        }
                    },
                ),
        );
    }

    bar.child(
        div()
            .ml_2()
            .text_size(px(9.0))
            .font_family("monospace")
            .text_color(theme::chrome_color(theme::TEXT_SECONDARY))
            .child("Asset · OTL · Integrator"),
    )
}

/// Build the node canvas frame: environment tabs, interactive graph body, wiring alerts.
pub fn render_node_canvas<H: NodeCanvasPane + 'static>(
    view: Entity<H>,
    host: &mut H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let tab_bar = render_canvas_tab_bar(view.clone(), host, cx);
    let alerts = render_wiring_alerts(&host.wiring_alert_messages());
    let graph = host.render_canvas_graph(cx);

    div()
        .id("node-canvas-frame")
        .size_full()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(theme::chrome_color(theme::CANVAS_BACKPLATE))
        .relative()
        .child(tab_bar)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .relative()
                .child(graph)
                .child(alerts),
        )
}
