//! Stage Composer pane: USD layer tree with prim activation toggles.

use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::scroll::ScrollableElement;

/// One row in the USD layer hierarchy.
#[derive(Clone, Debug)]
pub struct StagePrimRow {
    pub path: String,
    pub label: String,
    pub depth: usize,
    pub active: bool,
}

pub trait StageComposerPane: Sized {
    fn stage_prim_rows(&self, cx: &App) -> Vec<StagePrimRow>;
    fn set_prim_active(&mut self, path: &str, active: bool, cx: &mut Context<Self>);
}

/// Repaint the workstation whenever the shared USD stage composition entity notifies.
pub fn install_stage_composition_observer<H: StageComposerPane + 'static, W: 'static>(
    usd_stage: &Entity<W>,
    cx: &mut Context<H>,
) {
    cx.observe(usd_stage, |_host, _stage, cx| {
        cx.notify();
    })
    .detach();
}

pub fn render_stage_composer<H: StageComposerPane + 'static>(
    view: Entity<H>,
    host: &H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let rows = host.stage_prim_rows(cx);
    let mut tree = div().flex_col().gap_1().p_3().overflow_y_scrollbar();

    if rows.is_empty() {
        tree = tree.child(
            div()
                .text_xs()
                .text_color(rgb(0x71717a))
                .child("No USD prims loaded."),
        );
    } else {
        for (row_index, row) in rows.into_iter().enumerate() {
            let path = row.path.clone();
            tree = tree.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(12.0 * row.depth as f32))
                    .child(
                        Checkbox::new(("stage-active", row_index))
                            .checked(row.active)
                            .on_click({
                                let view = view.clone();
                                move |checked, window, cx| {
                                    let path = path.clone();
                                    view.update(cx, |host, cx| {
                                        host.set_prim_active(&path, *checked, cx);
                                    });
                                    let _ = window;
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_family("monospace")
                            .text_color(if row.active {
                                rgb(0xe4e4e7)
                            } else {
                                rgb(0x52525b)
                            })
                            .child(row.label),
                    ),
            );
        }
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .pt_2()
                .text_size(px(10.0))
                .text_color(rgb(0x34d399))
                .child("USD Layer Tree // FieldKey::Active"),
        )
        .child(tree.flex_1().min_h_0())
}
