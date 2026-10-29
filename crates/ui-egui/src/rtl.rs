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
