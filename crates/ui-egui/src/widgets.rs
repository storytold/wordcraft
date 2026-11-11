//! Ribbon widgets: large and small buttons, split buttons, groups, combo fields, colour grids.

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use serde_json::Value;

use crate::theme::{Tokens, regular};
use crate::{WordApp, icons};

mod buttons;
mod combo;
mod groups;
mod icon_buttons;
mod tooltip;

pub use buttons::{
    BIG_ICON_Y, BIG_LABEL_Y, ButtonAction, CONTENT_H, DROPDOWN, LABEL_H, LABEL_LINE_H, LABEL_PITCH, MenuButtonOptions, big, big_toggle, enabled,
    label_line, menu_button, menu_enabled, small, split,
};
pub(crate) use buttons::{bg, run_from_ribbon};
pub use combo::{
    ADD_FONT, COMBO_PREVIEW_ROW_H, DrawnState, FontComboOptions, LOAD_SYSTEM_FONTS, color_grid, combo, combo_hint, combo_list, drawn_state,
    font_combo, font_combo_with_recent, font_combo_with_recent_hint, font_matches,
};
#[cfg(test)]
use combo::{Row, highlight_row, section_row, section_rows};
pub(crate) use combo::{current_pick, note_hover, set_current_pick, swatch, take_hover};
pub use groups::{command_icon, command_menu, customize_menu_items, group, group_titled, row};
pub use icon_buttons::{
    dialog_tabs, icon_button, icon_menu_item, icon_only_button, icon_text_button, icon_text_button_width, icon_text_menu_button, primary_button,
};
#[cfg(test)]
use tooltip::show_hint;
#[cfg(test)]
pub(crate) use tooltip::tooltip_lines;
pub use tooltip::{key_text, shortcut_text};
// #653: a two-line label ends inside the content, above the group title row.
const _: () = assert!(BIG_LABEL_Y + LABEL_PITCH + LABEL_LINE_H <= CONTENT_H);
const _: () = assert!(BIG_ICON_Y + 16.0 <= BIG_LABEL_Y);

#[cfg(test)]
thread_local! {
    /// Groups drawn taller than [`CONTENT_H`] (title and height): their content runs into the
    /// group label below, as View › Show did with four check boxes in one column.
    pub(crate) static TALL_GROUPS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };

    /// Group labels as drawn (title, the label band's rectangle and the text inside it): the
    /// band must sit below the content and hold the whole text, never clipped by the card.
    pub(crate) static GROUP_LABELS: std::cell::RefCell<Vec<DrawnGroupLabel>> = const { std::cell::RefCell::new(Vec::new()) };

    /// Top edges of the ribbon buttons drawn inside the group being laid out: distinct tops are
    /// the control rows the group shows (a large button spans one row, like Word's UIA rows).
    pub(crate) static DRAWN_ROW_TOPS: std::cell::RefCell<Vec<f32>> = const { std::cell::RefCell::new(Vec::new()) };

    /// Each group's drawn row count (title and distinct control rows) this frame.
    pub(crate) static GROUP_ROWS: std::cell::RefCell<Vec<(String, usize)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Record a drawn ribbon button's top edge for the group row count (tests).
#[cfg(test)]
pub(crate) fn note_row_top(y: f32) {
    DRAWN_ROW_TOPS.with(|t| t.borrow_mut().push(y));
}

/// The distinct tops recorded since the last clear: the rows a group drew.
#[cfg(test)]
pub(crate) fn drawn_rows() -> usize {
    DRAWN_ROW_TOPS.with(|t| {
        let mut tops: Vec<i32> = t.borrow().iter().map(|y| (y * 2.0).round() as i32).collect();
        tops.sort_unstable();
        tops.dedup();
        tops.len()
    })
}

/// Forget the tops recorded for the group being laid out (tests).
#[cfg(test)]
pub(crate) fn clear_row_tops() {
    DRAWN_ROW_TOPS.with(|t| t.borrow_mut().clear());
}

/// Record a group's drawn row count for this frame (tests).
#[cfg(test)]
pub(crate) fn note_group_rows(title: &str, rows: usize) {
    GROUP_ROWS.with(|g| g.borrow_mut().push((title.to_string(), rows)));
}

/// A group label as painted, for tests of the card's three bands.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct DrawnGroupLabel {
    pub title: String,
    pub band: Rect,
    pub text: Rect,
}

#[cfg(test)]
mod tests;
