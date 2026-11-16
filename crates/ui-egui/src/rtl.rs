//! Mirrored window layout for a right-to-left interface language (Arabic).
//!
//! egui has no global direction, but `ui.horizontal` runs right to left wherever the enclosing
//! layout prefers it (`Layout::prefer_right_to_left`): a right-to-left row, or a column whose
//! items are aligned to the right. So direction is inherited from a root: each panel, window and
//! menu body is wrapped in [`root`], columns are opened with [`vertical`] (egui's `ui.vertical`
//! always resets to left-aligned), and "far side" clusters use [`far_side`]. Widgets that paint
//! their own parts ask [`flows`] where their leading edge is.
//!
//! An area that egui lays out left to right on its own (a `Grid`, a far-side cluster) stays left
//! to right inside, because everything here follows the enclosing layout rather than the language.
//! The document itself is never mirrored: its direction belongs to the document.

use egui::{Align, Align2, InnerResponse, Layout, Rect, Ui, UiBuilder, pos2};

use craft_ui::direction::{self, Direction};

fn dir(rtl: bool) -> Direction {
    if rtl { Direction::Rtl } else { Direction::Ltr }
}

/// Install the interface direction on `ctx` when the language's has changed: the shared
/// `craft_ui::direction` state, and egui's own chrome (check boxes and radio buttons with the mark
/// at the reading start, submenus, scroll bars, title bars, the font combo) through
/// `Style::right_to_left`.
pub fn sync(ctx: &egui::Context) {
    let want = dir(on());
    if direction::HasDirection::direction(ctx) != want {
        direction::apply(ctx, want);
    }
}

/// Is the interface language written right to left?
pub fn on() -> bool {
    crate::i18n::current().rtl()
}

/// Is `ui` part of the mirrored interface: a right-to-left language, in an area that lays its
/// content out from the right (so rows opened in it run right to left)? Never in a left-to-right
/// language, whose far-side clusters also fill from the right but are not mirrored.
pub fn flows(ui: &Ui) -> bool {
    on() && ui.layout().prefer_right_to_left()
}

/// Run a container's body (a panel, window, popup or menu, which egui starts left-aligned) in the
/// interface's direction. In a left-to-right language this is just `add`.
pub fn root<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if !on() || flows(ui) {
        return add(ui);
    }
    ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Max)), add).inner
}

/// `ui.vertical`, aligned to the side rows start from: a column inside a right-to-left area keeps
/// its content on the right and its rows right to left.
pub fn vertical<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if flows(ui) { ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Max)), add) } else { ui.vertical(add) }
}

/// `ui.horizontal_top`, with its items running from the side reading starts from: a row opened
/// in a right-to-left area is a right-to-left layout of its own, so its first column sits at the
/// right edge.
pub fn horizontal_top<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if flows(ui) { ui.scope_builder(UiBuilder::new().layout(Layout::right_to_left(Align::Min)), add) } else { ui.horizontal_top(add) }
}

/// A button's text whose `>>` or `<<` points the way things move across the window (Add >>, <<
/// Remove): in a mirrored interface the two swap, because the list items move to is on the other side.
pub fn arrows(text: &str) -> String {
    if on() { text.replace(">>", "\u{1}").replace("<<", ">>").replace('\u{1}', "<<") } else { text.to_string() }
}

/// The layout for a row's items that belong at its far end (the side rows end on): filled from
/// that end inwards.
pub fn far_side(ui: &Ui, align: Align) -> Layout {
    if flows(ui) { Layout::left_to_right(align) } else { Layout::right_to_left(align) }
}

/// A row layout in the direction of `ui`, for `allocate_ui_with_layout` and `with_layout`.
pub fn row_layout(ui: &Ui, align: Align) -> Layout {
    if flows(ui) { Layout::right_to_left(align) } else { Layout::left_to_right(align) }
}

/// The x coordinate `inset` points in from the leading edge of `rect` (left, or right when `rtl`).
pub fn lead_x(rtl: bool, rect: Rect, inset: f32) -> f32 {
    let d = dir(rtl);
    direction::start_x(&d, rect) + direction::forward(&d) * inset
}

/// The x coordinate `inset` points in from the trailing edge of `rect`.
pub fn trail_x(rtl: bool, rect: Rect, inset: f32) -> f32 {
    lead_x(!rtl, rect, inset)
}

/// Text anchored at its leading edge and vertically centred: left-centre, or right-centre when `rtl`.
pub fn lead_center(rtl: bool) -> Align2 {
    direction::align2(&dir(rtl), Align2::LEFT_CENTER)
}

/// Text anchored at its leading edge and top.
pub fn lead_top(rtl: bool) -> Align2 {
    direction::align2(&dir(rtl), Align2::LEFT_TOP)
}

/// Where to put something `width` wide so that it starts `inset` in from the leading edge of
/// `rect`: its left x.
pub fn lead_left(rtl: bool, rect: Rect, inset: f32, width: f32) -> f32 {
    if rtl { rect.max.x - inset - width } else { rect.min.x + inset }
}

/// The leading top corner of `rect`.
pub fn lead_corner(rtl: bool, rect: Rect) -> egui::Pos2 {
    pos2(lead_x(rtl, rect, 0.0), rect.min.y)
}

/// A side panel on the side reading starts from (left; right in a right-to-left interface).
pub fn start_panel(id: &'static str) -> egui::Panel {
    direction::start_panel(&dir(on()), id)
}

/// A side panel on the side reading ends on (right; left in a right-to-left interface).
pub fn end_panel(id: &'static str) -> egui::Panel {
    direction::end_panel(&dir(on()), id)
}

/// `egui::Popup::menu`, with its items aligned to the side reading starts from.
pub fn menu(response: &egui::Response) -> egui::Popup<'_> {
    let popup = egui::Popup::menu(response);
    if on() { popup.layout(menu_layout()) } else { popup }
}

/// A right-to-left menu's layout: items against the right edge. Not justified, because egui
/// starts a full-width button's text at its left.
fn menu_layout() -> Layout {
    Layout::top_down(Align::Max)
}

/// A menu item: `text`, with its keyboard shortcut at the far side. `min_width` keeps a menu's
/// items one width.
pub fn menu_item<'a>(ui: &Ui, text: &'a str, shortcut: &'a str, min_width: f32) -> egui::Button<'a> {
    let button = if flows(ui) {
        egui::Button::new(egui::RichText::new(shortcut).weak()).right_text(text)
    } else {
        egui::Button::new(text).shortcut_text(shortcut)
    };
    button.min_size(egui::vec2(min_width, 0.0))
}

/// `Response::context_menu`, with its items aligned to the side reading starts from.
pub fn context_menu(response: &egui::Response, add: impl FnOnce(&mut Ui)) {
    let popup = egui::Popup::context_menu(response);
    let popup = if on() { popup.layout(menu_layout()) } else { popup };
    popup.show(add);
}

/// Run `add` in a left-to-right island inside a right-to-left area, for content egui can only
/// lay out from the left (a `Grid`, a horizontally scrolling strip). The island is set against the
/// leading (right) edge using the width it had last frame. Outside a right-to-left area this is
/// just `add`.
pub fn island<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if !flows(ui) {
        return add(ui);
    }
    // The island gets all the room there is, so its content takes its natural width; it starts
    // where content of last frame's width would end at the leading edge. A change of width shows
    // one frame late, so that frame asks for another.
    let key = ui.next_auto_id().with("rtl_island_width");
    let known = ui.data(|d| d.get_temp::<f32>(key));
    let room = ui.available_rect_before_wrap();
    let available = if room.width().is_finite() { room.width().max(0.0) } else { 0.0 };
    let width = known.unwrap_or(available);
    let left = room.max.x - width;
    if !left.is_finite() {
        return add(ui);
    }
    let max_rect = Rect::from_min_max(pos2(left, room.min.y), pos2(left + available.max(width), room.max.y));
    let shown = ui.scope_builder(UiBuilder::new().max_rect(max_rect).layout(Layout::top_down(Align::Min)), add);
    let now = shown.response.rect.width();
    if now.is_finite() && known.is_none_or(|w| (w - now).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(key, now));
        ui.ctx().request_repaint();
    }
    shown.inner
}

/// `grid.show(ui, add)`. egui's `Grid` cannot be laid out from the right (it places cells from a
/// left edge that a right-to-left parent does not have, and says so with a debug assertion), so
/// inside a right-to-left area it is drawn in an [`island`]. Its columns keep their order.
pub fn grid<R>(ui: &mut Ui, grid: egui::Grid, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    island(ui, |ui| grid.show(ui, add))
}

/// A form of rows and cells. Where the interface is mirrored (`flows`), a row's cells run from the
/// right edge, the first cell at the right, and every column is as wide as its widest cell, so the
/// labels sit at the reading start with their fields after them, to their left. Elsewhere it is
/// exactly an `egui::Grid` with the same id, columns and spacing. A `Grid` itself cannot be laid
/// out from the right, which is why [`grid`] keeps its columns in order.
pub struct Table {
    id: &'static str,
    columns: usize,
    spacing: Option<egui::Vec2>,
    striped: bool,
    top: bool,
    min_row_height: Option<f32>,
}

impl Table {
    pub fn new(id: &'static str) -> Self {
        Self { id, columns: 2, spacing: None, striped: false, top: false, min_row_height: None }
    }

    /// The number of columns; 0 leaves an `egui::Grid` without a fixed count.
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns;
        self
    }

    /// Cells align to the top of their row (they are centred in it otherwise, as a mirrored form
    /// wants for a label and its field; tall cells such as a column of widgets want the top).
    pub fn top(mut self) -> Self {
        self.top = true;
        self
    }

    pub fn min_row_height(mut self, height: f32) -> Self {
        self.min_row_height = Some(height);
        self
    }

    pub fn spacing(mut self, spacing: egui::Vec2) -> Self {
        self.spacing = Some(spacing);
        self
    }

    /// Shade every other row (only where the table is an `egui::Grid`: a mirrored table has no stripes).
    pub fn striped(mut self, striped: bool) -> Self {
        self.striped = striped;
        self
    }

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Rows) -> R) -> R {
        if !flows(ui) {
            let spec = egui::Grid::new(self.id);
            let spec = if self.columns > 0 { spec.num_columns(self.columns) } else { spec };
            let spec = if self.striped { spec.striped(true) } else { spec };
            let spec = match self.min_row_height {
                Some(height) => spec.min_row_height(height),
                None => spec,
            };
            let spec = match self.spacing {
                Some(spacing) => spec.spacing(spacing),
                None => spec,
            };
            return grid(ui, spec, |ui| add(&mut Rows { ui, mirror: None })).inner;
        }
        let key = ui.scope_id().with(("rtl_table_widths", self.id));
        let prev = ui.data(|d| d.get_temp::<Vec<f32>>(key)).unwrap_or_default();
        let spacing = self.spacing.unwrap_or(ui.spacing().item_spacing);
        let gap = spacing.x;
        // Rows stack top to bottom from the reading edge, whatever way the area around runs (a ribbon
        // group lays its content out in a row).
        let shown = ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Max)), |ui| {
            ui.spacing_mut().item_spacing.y = spacing.y;
            let mirror = Mirror { prev: prev.clone(), cur: Vec::new(), gap, top: self.top, min_row: self.min_row_height };
            let mut rows = Rows { ui, mirror: Some(mirror) };
            let out = add(&mut rows);
            (out, rows.mirror.map(|m| m.cur).unwrap_or_default())
        });
        let (out, cur) = shown.inner;
        let same = prev.len() == cur.len() && prev.iter().zip(&cur).all(|(a, b)| (a - b).abs() <= 0.5);
        if !same {
            ui.data_mut(|d| d.insert_temp(key, cur));
            ui.ctx().request_repaint();
        }
        out
    }
}

struct Mirror {
    /// The width each column had last frame.
    prev: Vec<f32>,
    /// The width each column has now (the widest of its cells).
    cur: Vec<f32>,
    gap: f32,
    top: bool,
    min_row: Option<f32>,
}

/// The rows of a [`Table`].
pub struct Rows<'a> {
    ui: &'a mut Ui,
    mirror: Option<Mirror>,
}

impl Rows<'_> {
    /// One row: add its cells in reading order with [`Cells::cell`].
    pub fn row<R>(&mut self, add: impl FnOnce(&mut Cells) -> R) -> R {
        match &mut self.mirror {
            None => {
                let out = add(&mut Cells { ui: self.ui, mirror: None, col: 0, pending: 0.0 });
                self.ui.end_row();
                out
            }
            Some(mirror) => {
                let (top, min_row) = (mirror.top, mirror.min_row);
                let body = |ui: &mut Ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if let Some(height) = min_row {
                        ui.set_min_height(height);
                    }
                    add(&mut Cells { ui, mirror: Some(mirror), col: 0, pending: 0.0 })
                };
                if top { self.ui.horizontal_top(body).inner } else { self.ui.horizontal(body).inner }
            }
        }
    }
}

/// The cells of one row of a [`Table`].
pub struct Cells<'a> {
    ui: &'a mut Ui,
    mirror: Option<&'a mut Mirror>,
    col: usize,
    /// The space still to leave before the next cell: what its column is wider than the last cell.
    pending: f32,
}

impl Cells<'_> {
    /// The next cell of the row (its widgets, in the order they read).
    pub fn cell<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        let Some(mirror) = self.mirror.as_deref_mut() else {
            return add(self.ui);
        };
        if self.pending > 0.0 {
            self.ui.add_space(self.pending);
        }
        let shown = self.ui.scope(add);
        let width = shown.response.rect.width();
        let width = if width.is_finite() { width.max(0.0) } else { 0.0 };
        if mirror.cur.len() <= self.col {
            mirror.cur.resize(self.col + 1, 0.0);
        }
        if let Some(w) = mirror.cur.get_mut(self.col) {
            *w = w.max(width);
        }
        let column = mirror.prev.get(self.col).copied().unwrap_or(0.0);
        self.pending = (column - width).max(0.0) + mirror.gap;
        self.col += 1;
        shown.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, fixture_lang, set_current};

    /// Where a two-cell row puts its label and its field, as the centres of their rectangles.
    fn row_centres() -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut centres = (0.0, 0.0);
        // The second frame has the column widths the first measured.
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                root(ui, |ui| {
                    Table::new("t").columns(2).show(ui, |t| {
                        t.row(|r| {
                            let label = r.cell(|ui| ui.label("Label:"));
                            let field = r.cell(|ui| ui.add(egui::TextEdit::singleline(&mut String::new()).desired_width(80.0)));
                            centres = (label.rect.center().x, field.rect.center().x);
                        });
                    });
                });
            });
            out.textures_delta.clear();
        }
        centres
    }

    /// A form reads from the reading start: the label of a row first, its field after it, so in a
    /// right-to-left interface the label is at the right and the field to its left.
    #[test]
    fn a_form_row_puts_the_label_at_the_reading_start() {
        set_current(Lang::EN);
        let (label, field) = row_centres();
        assert!(label < field, "left to right: label {label}, field {field}");
        set_current(fixture_lang("ar", "\tHome\tالرئيسية\n"));
        let (label, field) = row_centres();
        set_current(Lang::EN);
        assert!(label > field, "right to left: label {label}, field {field}");
    }
}
