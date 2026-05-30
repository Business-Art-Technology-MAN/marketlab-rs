//! High-density Stage Ledger Explorer: USD property grid with four parsing tracks.

use gpui::*;
use gpui_component::scroll::ScrollableElement;

use super::context::WorkspaceContext;

/// One row in the stage ledger property grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageLedgerEntry {
    pub prim_path: String,
    pub property: String,
    pub depth: usize,
    /// TRACK 1 — `inputs:active` / composed prim activation.
    pub active: bool,
    /// TRACK 2 — override layer or runtime overlay active on this prim.
    pub override_layer: bool,
    /// TRACK 3 — value differs from `schema.usda` default.
    pub deviates_from_schema: bool,
    pub value_label: String,
    /// TRACK 4 — `inputs:target` / `inputs:constituents` lineage labels.
    pub lineage: Vec<String>,
}

/// GPUI view hosting the ledger grid bound to [`WorkspaceContext`].
pub struct StageLedgerExplorer {
    workspace: Entity<WorkspaceContext>,
}

impl StageLedgerExplorer {
    pub fn new(workspace: Entity<WorkspaceContext>, cx: &mut Context<Self>) -> Self {
        cx.observe(&workspace, |_this, _workspace, cx| {
            cx.notify();
        })
        .detach();
        Self { workspace }
    }
}

impl Render for StageLedgerExplorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.workspace.read(cx).ledger_entries().to_vec();
        render_stage_ledger_grid(entries)
    }
}

/// Repaint hosts whenever the shared workspace context model notifies.
pub fn install_workspace_context_observer<H: 'static>(
    workspace: &Entity<WorkspaceContext>,
    cx: &mut Context<H>,
) {
    cx.observe(workspace, |_host, _workspace, cx| {
        cx.notify();
    })
    .detach();
}

pub fn render_stage_ledger(
    workspace: Entity<WorkspaceContext>,
    cx: &App,
) -> impl IntoElement {
    let entries = workspace.read(cx).ledger_entries().to_vec();
    render_stage_ledger_grid(entries)
}

fn render_stage_ledger_grid(entries: Vec<StageLedgerEntry>) -> impl IntoElement {
    let mut grid = div().flex_col().gap_0p5().flex_1().min_h_0().overflow_y_scrollbar();

    if entries.is_empty() {
        grid = grid.child(
            div()
                .text_xs()
                .text_color(rgb(0x71717a))
                .child("No stage properties to explore."),
        );
    } else {
        grid = grid.child(render_ledger_header());
        for (row_index, entry) in entries.into_iter().enumerate() {
            grid = grid.child(render_ledger_row(entry, row_index));
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
                .pb_1()
                .text_size(px(10.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xfbbf24))
                .child("Stage Ledger Explorer // 4-track parse"),
        )
        .child(grid.px_2().pb_2())
}

fn render_ledger_header() -> impl IntoElement {
    div()
        .flex()
        .gap_2()
        .px_1()
        .pb_1()
        .text_size(px(9.0))
        .font_weight(FontWeight::BOLD)
        .font_family("monospace")
        .text_color(rgb(0x52525b))
        .child(div().w(px(140.0)).child("Prim / Property"))
        .child(div().w(px(72.0)).child("Value"))
        .child(div().flex_1().child("Lineage"))
}

fn render_ledger_row(entry: StageLedgerEntry, row_index: usize) -> impl IntoElement {
    let opacity = if entry.active { 1.0 } else { 0.4 };
    let value_weight = if entry.deviates_from_schema {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    };
    let value_color = if entry.deviates_from_schema {
        rgb(0xfbbf24)
    } else {
        rgb(0xa1a1aa)
    };

    let lineage_text = if entry.lineage.is_empty() {
        String::new()
    } else {
        entry.lineage.join("  ")
    };

    let mut row = div()
        .id(("stage-ledger-row", row_index))
        .flex()
        .gap_2()
        .px_1()
        .py_0p5()
        .bg(rgb(0x111113))
        .border_1()
        .border_color(rgb(0x1f1f23))
        .opacity(opacity)
        .pl(px(8.0 + 10.0 * entry.depth as f32));

    let prim_property = if entry.property.is_empty() {
        entry.prim_path.clone()
    } else {
        format!("{} · {}", entry.prim_path, entry.property)
    };

    row = row.child(
        div()
            .w(px(140.0))
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0xe4e4e7))
                    .child(prim_property),
            ),
    );

    if entry.override_layer {
        row = row.child(
            div()
                .flex_shrink_0()
                .px_1p5()
                .py_0p5()
                .rounded_xs()
                .bg(rgb(0x422006))
                .text_size(px(8.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xfbbf24))
                .child("⚠️ OVERRIDE ACTIVE"),
        );
    }

    row = row
        .child(
            div()
                .w(px(72.0))
                .text_size(px(9.0))
                .font_family("monospace")
                .font_weight(value_weight)
                .text_color(value_color)
                .child(entry.value_label),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(8.0))
                .font_family("monospace")
                .text_color(rgb(0x38bdf8))
                .child(lineage_text),
        );

    row
}
