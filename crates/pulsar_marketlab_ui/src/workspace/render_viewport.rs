//! Render Viewport pane: financial ledger grid and playhead scrubber.

use gpui::*;
use gpui_component::scroll::ScrollableElement;

#[derive(Clone, Debug)]
pub struct LedgerRow {
    pub tick: String,
    pub asset: String,
    pub grade_type: String,
    pub value: String,
}

pub trait RenderViewportPane: Sized {
    fn ledger_rows(&self) -> Vec<LedgerRow>;
    fn playhead_current(&self) -> usize;
    fn playhead_total(&self) -> usize;
    fn playhead_time_label(&self) -> String;
    fn playhead_scrubbing(&self) -> bool;
    fn set_playhead_scrubbing(&mut self, scrubbing: bool);
    fn playhead_slider_bounds(&self) -> Option<Bounds<Pixels>>;
    fn set_playhead_slider_bounds(&mut self, bounds: Bounds<Pixels>);
    fn set_playhead_from_slider(&mut self, normalized: f32, cx: &mut Context<Self>);
    fn sync_view_window_on_scrub(&mut self, cx: &mut Context<Self>);
    fn render_playhead_chart(&mut self, cx: &mut Context<Self>) -> impl IntoElement;
    fn status_log_lines(&self) -> &[String];
}

struct PlayheadTrackDrag;

impl Render for PlayheadTrackDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_0()
    }
}

pub fn render_render_viewport<H: RenderViewportPane + 'static>(
    view: Entity<H>,
    host: &mut H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let rows = host.ledger_rows();
    let playhead_current = host.playhead_current();
    let playhead_total = host.playhead_total().max(1);
    let playhead_label = host.playhead_time_label();
    let slider_value = if playhead_total <= 1 {
        0.0
    } else {
        playhead_current as f32 / (playhead_total - 1) as f32
    };

    let mut ledger = div().flex_col().gap_1().flex_1().min_h_0().overflow_y_scrollbar();
    if rows.is_empty() {
        ledger = ledger.child(
            div()
                .text_xs()
                .text_color(rgb(0x71717a))
                .child("Ledger empty — start CSV playback or scrub the timeline."),
        );
    } else {
        ledger = ledger.child(
            div()
                .flex()
                .gap_2()
                .px_1()
                .pb_1()
                .text_size(px(9.0))
                .font_weight(FontWeight::BOLD)
                .font_family("monospace")
                .text_color(rgb(0x52525b))
                .child(div().w(px(72.0)).child("Tick"))
                .child(div().w(px(64.0)).child("Asset"))
                .child(div().w(px(88.0)).child("Grade"))
                .child(div().flex_1().text_right().child("Value")),
        );
        for row in rows {
            ledger = ledger.child(
                div()
                    .flex()
                    .gap_2()
                    .px_1()
                    .py_0p5()
                    .bg(rgb(0x141417))
                    .border_1()
                    .border_color(rgb(0x222227))
                    .font_family("monospace")
                    .text_size(px(9.0))
                    .child(div().w(px(72.0)).text_color(rgb(0x71717a)).child(row.tick))
                    .child(div().w(px(64.0)).text_color(rgb(0xffffff)).child(row.asset))
                    .child(
                        div()
                            .w(px(88.0))
                            .text_color(rgb(0x38bdf8))
                            .child(row.grade_type),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_right()
                            .text_color(rgb(0x10b981))
                            .child(row.value),
                    ),
            );
        }
    }

    let status_lines: Vec<String> = host.status_log_lines().to_vec();
    let chart = host.render_playhead_chart(cx);

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .child(chart)
        .child(
            div()
                .flex_shrink_0()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .font_family("monospace")
                        .child(
                            div()
                                .text_color(rgb(0x71717a))
                                .child(format!(
                                    "Playhead t={playhead_label} [{playhead_current}/{}]",
                                    playhead_total.saturating_sub(1)
                                )),
                        )
                        .child(
                            div()
                                .text_color(rgb(0x34d399))
                                .child("View window index"),
                        ),
                )
                .child(
                    div()
                        .on_children_prepainted({
                            let view = view.clone();
                            move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                                if let Some(track_bounds) = union_track_bounds(&bounds) {
                                    view.update(cx, |host, _cx| {
                                        host.set_playhead_slider_bounds(track_bounds);
                                    });
                                }
                            }
                        })
                        .id("playhead-slider-track")
                        .h(px(24.0))
                        .w_full()
                        .relative()
                        .bg(rgb(0x18181b))
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(0x27272a))
                        .cursor(CursorStyle::PointingHand)
                        .child(
                            div().absolute().top_0().left_0().right_0().bottom_0(),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            {
                                let view = view.clone();
                                move |event: &MouseDownEvent, _window, cx| {
                                    view.update(cx, |host, cx| {
                                        host.set_playhead_scrubbing(true);
                                        if let Some(bounds) = host.playhead_slider_bounds() {
                                            apply_playhead_at_position(host, event.position, bounds, cx);
                                        }
                                    });
                                    cx.stop_propagation();
                                }
                            },
                        )
                        .on_drag(PlayheadTrackDrag, {
                            let view = view.clone();
                            move |_, _offset, _window, cx| {
                                view.update(cx, |host, cx| {
                                    host.set_playhead_scrubbing(true);
                                    cx.notify();
                                });
                                cx.new(|_| PlayheadTrackDrag)
                            }
                        })
                        .on_drag_move({
                            let view = view.clone();
                            move |event: &DragMoveEvent<PlayheadTrackDrag>, window, cx| {
                                let _ = event;
                                view.update(cx, |host, cx| {
                                    if let Some(bounds) = host.playhead_slider_bounds() {
                                        apply_playhead_at_position(
                                            host,
                                            window.mouse_position(),
                                            bounds,
                                            cx,
                                        );
                                    }
                                });
                            }
                        })
                        .capture_any_mouse_up({
                            let view = view.clone();
                            move |_: &MouseUpEvent, _window, cx| {
                                view.update(cx, |host, cx| {
                                    if host.playhead_scrubbing() {
                                        host.set_playhead_scrubbing(false);
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .w(px(2.0))
                                .bg(rgb(0x34d399))
                                .left(relative(slider_value.max(0.0).min(1.0))),
                        ),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x34d399))
                        .child("Financial Ledger"),
                )
                .child(ledger),
        )
        .child(render_status_console(&status_lines))
}

fn render_status_console(lines: &[String]) -> impl IntoElement {
    let mut log_lines = div().flex_col().gap_0p5().mt_2().flex_1();
    for line in lines {
        log_lines = log_lines.child(
            div()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child(line.clone()),
        );
    }

    div()
        .h(px(96.0))
        .w_full()
        .bg(rgb(0x0c0c0e))
        .border_t_1()
        .border_color(rgb(0x222227))
        .p_3()
        .flex_col()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0x34d399))
                .child("Pipeline Status Console"),
        )
        .child(log_lines)
}

fn union_track_bounds(bounds: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
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

fn apply_playhead_at_position<H: RenderViewportPane>(
    host: &mut H,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cx: &mut Context<H>,
) {
    let origin_x: f32 = bounds.origin.x.into();
    let width: f32 = bounds.size.width.into();
    if width <= f32::EPSILON {
        return;
    }
    let mouse_x: f32 = position.x.into();
    let normalized = ((mouse_x - origin_x) / width).clamp(0.0, 1.0);
    host.set_playhead_from_slider(normalized, cx);
}
