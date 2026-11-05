# Navigation tree styles

The Navigation pane's **Headings** tab draws the outline as a tree. Two styles are available;
the choice is saved in `ui.json` under `navTreeStyle` and can be changed in two places:

- **File › Options › Display › Navigation tree** (the segmented `Option A` / `Option B` control);
- the **▾ menu** of the Navigation pane (the current style is marked with ✓).

| Style | Key | Look |
|---|---|---|
| A (default) | `zed_linear` | Clean rail: continuous trunks and a circular chevron node per parent; the blue elbow appears only on the active branch (the caret's heading and its ancestors) and on the selected row's pill. |
| B | `fleet_connected` | Every child is connected by an elbow; nested trunks end on the last child's curve, while a root rail runs on to the next root. |

Shared geometry: trunks 1.3 px (1.6 px with a soft glow on the active branch), a six-pixel
quadratic elbow, a circular chevron node (7 px at level 1, 6.5 px below) that the trunk enters
above and leaves below, and the selection pill starting where the active curve ends.

The geometry is computed by pure functions in `crates/ui-egui/src/panes.rs`
(`nav_row_geometry`) and unit-tested without a GPU.
