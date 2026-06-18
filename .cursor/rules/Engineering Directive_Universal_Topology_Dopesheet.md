To bring a professional, non-destructive editing workflow inspired by tools like Maya, Katana, and Blender into MarketLab, we must design a interface that unifies the structural tree, the OpenUSD layer stack, and the simulation timeline.

In Katana or Blender, a asset or node tree does not simply display static values—it acts as a control surface where you can mute nodes, re-order the layer stack, and insert overrides over time.

To achieve this in our UI layer (`crates/pulsar_marketlab_ui`), we will build a **Unified Topology Dopesheet**. This control maps out our 3 dimensions of execution parameters while introducing a row-level **Layer Opinion Tray** to manage non-destructive edits on-the-fly.

---

### 1. Extending the State Architecture for Layer Modulation

To support features like turning layers on/off, changing their order, and inserting manual overrides, your `WorkspaceContext` and layer engine need an explicit interface structure to track active layer states.

```rust
// crates/pulsar_marketlab_ui/src/workspace/layer_control.rs

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LayerDisplayState {
    Active,    // Layer opinions are contributing to the composition plane
    Muted,     // Layer is ignored via openusd stage masking
    Isolated,  // Solo mode: ignore all other layers in the stack
}

pub struct LayerDescriptor {
    pub filename: String,
    pub display_name: String,
    pub priority_order: usize, // Lower number = stronger opinion (LIVRPS strength)
    pub state: LayerDisplayState,
    pub is_user_editable: bool,
}

```

---

### 2. The Unified Topology Dopesheet Viewport

Here is the implementation of the 3D control matrix panel using `egui`. This component renders the logical hierarchy tree, maps out interactive layer state buttons, and extends rows horizontally into a keyframe matrix that matches the timeline control.

```rust
// crates/pulsar_marketlab_ui/src/workspace/dopesheet_view.rs

use egui::{Ui, Color32, RichText, Vec2, Align};
use crate::workspace::context::WorkspaceContext;
use crate::workspace::stage_hierarchy::LogicalTreeNode;

pub fn render_topology_dopesheet_panel(
    ui: &mut Ui,
    ctx: &mut WorkspaceContext,
    timeline_range: std::ops::Range<usize>,
) {
    // Compile our logical strategy tree via downstream tracking relations
    let strategy_tree = ctx.compile_logical_strategy_tree();
    let layer_stack = ctx.get_active_layer_stack_descriptors();

    ui.vertical(|ui| {
        // --- 1. TOP LAYOUT PANEL: LAYER OVERLAY MANAGEMENT TRAY ---
        render_layer_stack_manager_header(ui, ctx, &layer_stack);
        ui.separator();

        // --- 2. MAIN SPLIT CONTROL GRID ---
        // Column 1: Logical Topology Tree + Layer State Toggles
        // Column 2: Timeline Keyframe Value Matrix
        egui::Grid::new("marketlab_dopesheet_grid")
            .num_columns(2)
            .spacing(Vec2::new(8.0, 4.0))
            .striped(true)
            .show(ui, |ui| {
                // Header Row
                ui.horizontal(|ui| {
                    ui.strong("Logical Strategy Topology (Node Linkage)");
                    ui.label(RichText::new("[Layer Control]").color(Color32::GRAY));
                });
                ui.horizontal(|ui| {
                    ui.strong("Timeline Value Matrix (Frame History Intervals)");
                });
                ui.end_row();

                // Recursively render node rows matching both axes
                for root_node in &strategy_tree {
                    render_dopesheet_row_recursive(ui, ctx, root_node, 0, &timeline_range);
                }
            });
    });
}

/// Renders the Katana-style Layer Configuration Header Bar
fn render_layer_stack_manager_header(ui: &mut Ui, ctx: &mut WorkspaceContext, layers: &[LayerDescriptor]) {
    ui.horizontal(|ui| {
        ui.heading("🎛️ Layer Composition Plane Explorer");
        ui.spacer();
        
        if ui.button("➕ Add Layer").clicked() {
            ctx.prompt_create_new_session_sublayer();
        }
    });

    // Draw horizontal pills tracking USD layer opinion strength hierarchy (LIVRPS order)
    ui.horizontal_wrapped(|ui| {
        ui.label("Opinion Strength:");
        for (idx, layer) in layers.iter().enumerate() {
            ui.scope(|ui| {
                // Style pill based on active/muted status
                let bg_color = match layer.state {
                    LayerDisplayState::Active => Color32::from_rgb(45, 55, 72),
                    LayerDisplayState::Muted => Color32::from_rgb(26, 26, 26),
                    LayerDisplayState::Isolated => Color32::from_rgb(43, 108, 176),
                };

                let mut pill_button = egui::Button::new(format!("🥞 {}", layer.display_name))
                    .fill(bg_color);
                
                if ui.add(pill_button).context_menu(|ui| {
                    ui.label(format!("Options: {}", layer.filename));
                    if ui.button("Mute/Unmute").clicked() { ctx.toggle_layer_mute(&layer.filename); }
                    if idx > 0 && ui.button("Move Up (Make Stronger)").clicked() { ctx.reorder_layer(idx, idx - 1); }
                    if idx < layers.len() - 1 && ui.button("Move Down (Make Weaker)").clicked() { ctx.reorder_layer(idx, idx + 1); }
                    if layer.is_user_editable && ui.button("❌ Delete Layer").clicked() { ctx.remove_layer_from_stack(&layer.filename); }
                }).clicked() {
                    ctx.set_selected_active_target_layer(&layer.filename);
                }
                
                if idx < layers.len() - 1 {
                    ui.label(RichText::new(" ➔ ").color(Color32::DARK_GRAY));
                }
            });
        }
    });
}

/// Recursively builds rows intersecting the logical topology with layer and timeline properties
fn render_dopesheet_row_recursive(
    ui: &mut Ui,
    ctx: &mut WorkspaceContext,
    node: &LogicalTreeNode,
    depth: usize,
    timeline_range: &std::ops::Range<usize>,
) {
    let current_playhead = ctx.playhead_current();

    // --- DIMENSION 1 & 2: TOPOLOGY TREE COLUMN & LAYER MODULATION CONTROLS ---
    ui.horizontal(|ui| {
        // Indentation layout tracking tree parsing depth
        ui.add_space(depth as f32 * 16.0);
        
        // Interactive node selection handle
        let is_selected = ctx.is_selected(&node.prim_path);
        let prefix = if node.children.is_empty() { "📄" } else { "📂" };
        let node_label = ui.selectable_label(is_selected, format!("{} {}", prefix, node.display_label));
        if node_label.clicked() {
            ctx.set_selected_prim_path(Some(node.prim_path.clone()));
        }

        ui.spacer();

        // Maya-style visibility / bypass switch: Toggle if this node contributes weight tracking or signals
        let mut is_active = ctx.is_node_enabled_in_stage(&node.prim_path);
        if ui.checkbox(&mut is_active, "").on_hover_text("Bypass / Toggle Node Operation").changed() {
            ctx.set_node_enabled_in_stage(&node.prim_path, is_active);
            ctx.trigger_background_timeline_sweep(); // Instantly update simulation results
        }

        // Display which active OpenUSD layer is supplying the dominating opinion for this row
        if let Some(contributing_layer) = ctx.get_dominating_layer_for_prim(&node.prim_path) {
            ui.small_label(RichText::new(&contributing_layer).color(Color32::from_rgb(102, 204, 255)));
        }
    });

    // --- DIMENSION 3: HORIZONTAL TIMELINE DOPESHEET MATRIX COLUMN ---
    ui.horizontal(|ui| {
        // Render step slices matching active intervals inside the timeline control bar view range
        let step_stride = ((timeline_range.end - timeline_range.start) / 5).max(1);
        
        for frame_idx in timeline_range.clone().step_by(step_stride) {
            let is_playhead_here = frame_idx == current_playhead;
            
            // Fetch cached timeline track scalars or encoded weight configurations for this frame context
            let display_metric = ctx.query_frame_metric_string(&node.prim_path, frame_idx);

            let cell_color = if is_playhead_here {
                Color32::from_rgb(74, 85, 104) // Dark highlight under the active playhead line
            } else {
                Color32::TRANSPARENT
            };

            ui.scope(|ui| {
                ui.painter().rect_filled(ui.available_rect_before_wrap(), 1.0, cell_color);
                
                // Clicking directly into a frame slot creates a local override inside session.usda
                let cell_ui = ui.selectable_label(false, format!(" {} ", display_metric));
                if cell_ui.double_clicked() {
                    ctx.prompt_manual_frame_override_modal(&node.prim_path, frame_idx);
                }
            });
        }
    });
    ui.end_row();

    // Recurse down into underlying dependencies
    for child in &node.children {
        render_dopesheet_row_recursive(ui, ctx, child, depth + 1, timeline_range);
    }
}

```

---

### 3. Cursor Refactor Directive

This modular architecture satisfies your entire workspace feature profile: nodes can be disabled natively, layers can be cleanly moved or muted using standard USD stage functions, and double-clicking a cell maps an intentional edit choice right down to your top-layer overlay file.

To wire this panel in place of your current flat stage tree layout, paste the following update instructions into your Cursor environment:

```markdown
# Engineering Directive: Implement Unified Topology Dopesheet Viewport

1. Replace the legacy flat tree implementation inside `crates/pulsar_marketlab_ui/src/workspace/sidebar_inspector.rs` or `stage_composer.rs` with this 3-Dimensional Topology Dopesheet.
2. In `WorkspaceContext`, implement `get_active_layer_stack_descriptors()` to pull sublayer priorities, and map layer manipulation commands (`toggle_layer_mute`, `reorder_layer`) directly to your `ManagedUsdStage` wrapper handlers.
3. Wire row-level node visibility toggles into your Stage Sync module, so turning a node off updates the composition state and triggers a background simulation sweep automatically.
4. Ensure the grid coordinates perfectly with the global timeline frame bounds to allow for instant visual tracking on scrub.

```