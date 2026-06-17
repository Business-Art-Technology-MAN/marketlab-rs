//! Collapsible workstation "shelves" for dense Context Tower and Stage Composer panels.

use std::collections::HashSet;

use gpui::*;
use gpui_component::scroll::ScrollableElement;

/// Identifies a collapsible shelf region in the workstation layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkstationShelfId {
    StageHierarchy,
    StageLedger,
    TowerInspector,
    TowerOtlEditor,
    TowerLayerStack,
    TowerStageComposer,
    InspectorPipelineOverview,
    InspectorGlobalPerformance,
}

impl WorkstationShelfId {
    pub const fn element_key(self) -> &'static str {
        match self {
            Self::StageHierarchy => "stage-hierarchy",
            Self::StageLedger => "stage-ledger",
            Self::TowerInspector => "tower-inspector",
            Self::TowerOtlEditor => "tower-otl-editor",
            Self::TowerLayerStack => "tower-layer-stack",
            Self::TowerStageComposer => "tower-stage-composer",
            Self::InspectorPipelineOverview => "inspector-pipeline-overview",
            Self::InspectorGlobalPerformance => "inspector-global-performance",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::StageHierarchy => 0,
            Self::StageLedger => 1,
            Self::TowerInspector => 2,
            Self::TowerOtlEditor => 3,
            Self::TowerLayerStack => 4,
            Self::TowerStageComposer => 5,
            Self::InspectorPipelineOverview => 6,
            Self::InspectorGlobalPerformance => 7,
        }
    }
}

/// Expanded/collapsed state for workstation shelves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkstationShelfState {
    collapsed: HashSet<WorkstationShelfId>,
}

impl WorkstationShelfState {
    pub fn is_expanded(&self, shelf: WorkstationShelfId) -> bool {
        !self.collapsed.contains(&shelf)
    }

    pub fn set_expanded(&mut self, shelf: WorkstationShelfId, expanded: bool) {
        if expanded {
            self.collapsed.remove(&shelf);
        } else {
            self.collapsed.insert(shelf);
        }
    }

    pub fn toggle(&mut self, shelf: WorkstationShelfId) {
        if self.collapsed.insert(shelf) {
            return;
        }
        self.collapsed.remove(&shelf);
    }
}

pub trait WorkstationShelfHost: Sized + 'static {
    fn workstation_shelf_state(&self) -> &WorkstationShelfState;
    fn toggle_workstation_shelf(&mut self, shelf: WorkstationShelfId, cx: &mut Context<Self>);
}

/// Vertical stack container: collapsed headers pin to the top; expanded shelves fill downward.
pub fn render_shelf_stack(children: impl IntoElement) -> impl IntoElement {
    div()
        .id("workstation-shelf-stack")
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .justify_start()
        .child(children)
}

/// Collapsible shelf chrome with optional flex growth and internal scrolling.
pub fn render_collapsible_shelf<H: WorkstationShelfHost>(
    view: Entity<H>,
    shelves: &WorkstationShelfState,
    shelf: WorkstationShelfId,
    title: impl Into<String>,
    grow_when_expanded: bool,
    body: impl IntoElement,
) -> impl IntoElement {
    let expanded = shelves.is_expanded(shelf);
    let chevron = if expanded { "▾" } else { "▸" };
    let title = title.into();

    let mut shelf_root = div()
        .id(("workstation-shelf", shelf.index()))
        .w_full()
        .flex()
        .flex_col()
        .min_h_0()
        .border_b_1()
        .border_color(crate::theme::chrome_color(crate::theme::GRID_MAJOR));

    shelf_root = shelf_root.flex_shrink_0();
    if grow_when_expanded && expanded {
        shelf_root = shelf_root.flex_1().min_h_0();
    }

    let header = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1p5()
        .bg(crate::theme::chrome_color(crate::theme::PANE_BACKPLATE))
        .cursor(CursorStyle::PointingHand)
        .hover(|style| style.bg(crate::theme::chrome_color(crate::theme::ROW_HOVER_BG)))
        .on_mouse_up(MouseButton::Left, {
            let view = view.clone();
            move |_event, _window, cx| {
                cx.stop_propagation();
                view.update(cx, |host, cx| host.toggle_workstation_shelf(shelf, cx));
            }
        })
        .child(
            div()
                .w(px(10.0))
                .text_size(px(10.0))
                .text_color(crate::theme::chrome_color(crate::theme::TEXT_HINT))
                .child(chevron),
        )
        .child(
            div()
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family("monospace")
                .text_color(crate::theme::chrome_color(crate::theme::TEXT_SECONDARY))
                .child(title),
        );

    shelf_root = shelf_root.child(header);

    if expanded {
        shelf_root = shelf_root.child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scrollbar()
                .overflow_x_hidden()
                .child(body),
        );
    }

    shelf_root
}
