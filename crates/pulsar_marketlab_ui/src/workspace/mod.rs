//! Nested resizable workstation layout: upper tri-pane row + lower render viewport.

mod menu_bar;
mod node_canvas;
mod param_inspector;
mod render_viewport;
mod split_layout;
mod stage_composer;

pub use menu_bar::{MenuBar, MenuBarHost};

pub use node_canvas::{
    paint_socket_dot, paint_wires_for_graph, render_wiring_alerts, socket_color, socket_pin,
    GraphWireSegment, SocketWireKind,
};
pub use param_inspector::{ParamInspectorPane, render_param_inspector};
pub use render_viewport::{LedgerRow, RenderViewportPane, render_render_viewport};
pub use split_layout::{
    SplitHandle, SplitLayoutHost, WorkstationSplitLayout, render_split_handle,
};
pub use stage_composer::{
    StageComposerPane, StagePrimRow, install_stage_composition_observer, render_stage_composer,
};

use gpui::*;

fn union_bounds(bounds: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
    let mut iter = bounds.iter();
    let first = *iter.next()?;
    Some(iter.fold(first, |acc, next| {
        let top_left = point(
            acc.origin.x.min(next.origin.x),
            acc.origin.y.min(next.origin.y),
        );
        let bottom_right = point(
            acc.bottom_right().x.max(next.bottom_right().x),
            acc.bottom_right().y.max(next.bottom_right().y),
        );
        Bounds::from_corners(top_left, bottom_right)
    }))
}

/// Host type rendered inside the four workstation panes.
pub trait WorkstationLayoutHost:
    Sized
    + SplitLayoutHost
    + StageComposerPane
    + ParamInspectorPane
    + RenderViewportPane
    + MenuBarHost
    + 'static
{
    fn render_node_canvas(&mut self, cx: &mut Context<Self>) -> impl IntoElement;
}

/// Build the nested splitter tree with GPUI drag-captured handles.
pub fn render_workstation_layout<H: WorkstationLayoutHost>(
    host: &mut H,
    window: &mut Window,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let view = cx.entity();
    let layout = host.split_layout().clamp();
    let menu_bar = MenuBar::render(view.clone(), host, window, cx);
    let stage_composer = render_stage_composer(view.clone(), host, cx);
    let param_inspector = render_param_inspector(view.clone(), host, window, cx);
    let render_viewport = render_render_viewport(view.clone(), host, cx);
    let node_canvas = host.render_node_canvas(cx);

    div()
        .on_children_prepainted({
            let view = view.clone();
            move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                if let Some(root_bounds) = union_bounds(&bounds) {
                    view.update(cx, |host, _cx| host.set_split_container_bounds(root_bounds));
                }
            }
        })
        .id("marketlab-workstation-root")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(0x09090b))
        .on_key_down({
            let view = view.clone();
            move |event: &KeyDownEvent, _window: &mut Window, cx: &mut App| {
                if !event.keystroke.modifiers.control {
                    return;
                }
                view.update(cx, |host, cx| match event.keystroke.key.as_str() {
                    "n" => host.on_file_new(cx),
                    "o" => host.on_file_open(cx),
                    "s" if event.keystroke.modifiers.shift => host.on_file_save_as(cx),
                    "s" => host.on_file_save(cx),
                    _ => {}
                });
            }
        })
        .child(menu_bar)
        .child(
            div()
                .on_children_prepainted({
                    let view = view.clone();
                    move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                        if let Some(upper_bounds) = union_bounds(&bounds) {
                            view.update(cx, |host, _cx| host.set_upper_row_bounds(upper_bounds));
                        }
                    }
                })
                .id("marketlab-workstation-upper")
                .relative()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .h(relative(layout.upper_share))
                .min_h(px(160.0))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(relative(layout.stage_share))
                                .min_w(px(140.0))
                                .child(pane_shell("Stage Composer", stage_composer)),
                        )
                        .child(render_split_handle(
                            view.clone(),
                            SplitHandle::StageCanvas,
                            Axis::Horizontal,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(180.0))
                                .child(pane_shell("Node Canvas", node_canvas)),
                        )
                        .child(render_split_handle(
                            view.clone(),
                            SplitHandle::CanvasInspector,
                            Axis::Horizontal,
                        ))
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(relative(layout.inspector_share))
                                .min_w(px(220.0))
                                .child(pane_shell("Param Inspector", param_inspector)),
                        ),
                ),
        )
        .child(render_split_handle(view.clone(), SplitHandle::Vertical, Axis::Vertical))
        .child(
            div()
                .flex_1()
                .min_h(px(140.0))
                .child(pane_shell("Render Viewport", render_viewport)),
        )
}

fn pane_shell(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .size_full()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(rgb(0x0c0c0e))
        .border_1()
        .border_color(rgb(0x222227))
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(rgb(0x222227))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child(title),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .child(body),
        )
}
