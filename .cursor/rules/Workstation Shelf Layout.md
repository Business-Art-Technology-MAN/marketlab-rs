# Workstation Shelf Layout Contract

## Top-Stacked Collapse Behavior

Workstation panels that use `render_shelf_stack` + `render_collapsible_shelf` must obey:

1. **Collapsed shelves** render **header chrome only** (`flex_shrink_0`) and stack from the **top** of the panel downward.
2. **Expanded shelves** with `grow_when_expanded: true` receive `flex_1 min_h_0` and consume all remaining vertical space **below** the collapsed header stack, stopping at the next shelf header or panel bottom.
3. Never assign `flex_1` to collapsed shelves. Never leave dead vertical gaps between collapsed headers.

Implementation: `crates/pulsar_marketlab_ui/src/workspace/workstation_shelf.rs`.

## Context Tower Shelves (independent controls)

The Context Tower uses **separate shelves**, not tabs:

| Shelf ID | Title |
|----------|-------|
| `TowerInspector` | Param Inspector |
| `TowerOtlEditor` | OTL Script Editor |
| `TowerLayerStack` | USD Layer Stack |

Do not merge Param Inspector and OTL Editor into a single tabbed shelf.
