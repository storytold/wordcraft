//! Title bar (Quick Access Toolbar, title, search, account) and status bar, on the framework's parts.
//!
//! The framework draws the bars; this module keeps the rules the app owns: which commands the Quick Access
//! Toolbar holds, when AutoSave can switch on and what the status bar says.

use crate::{StatusCount, StatusCounts, WordApp};

pub(crate) mod screen_bar;
pub(crate) mod search;
pub(crate) mod status_bar;
pub(crate) mod title_bar;

/// The open state of every popup the title bar and status bar own, and how their controls fit the window. Kept on
/// the window state so a popup stays open across passes.
#[derive(Default)]
pub(crate) struct ChromeOpen {
    /// The Undo drop-down.
    pub(crate) undo: bool,
    /// The actions marked in the open Undo drop-down; back to none whenever the drop-down is closed.
    pub(crate) undo_mark: craft::ui::list::UndoMark,
    /// The command search.
    pub(crate) search: bool,
    /// How the title bar's controls at the end fit the room the leading controls leave them.
    pub(crate) title_fit: Fit,
    /// How the status bar's controls fit the room the page leaves them.
    pub(crate) status_fit: Fit,
}

/// The steps a bar's group of controls takes in a narrow window, from the fullest to the most compact.
pub(crate) const FIT_STEPS: usize = 3;

/// How a bar's group of controls fits a narrow window (#5886): the width each of its [`FIT_STEPS`] steps measured
/// when last shown, and the room the rest of the bar left it in the last pass. A framework bar gives its end region the
/// width it measures and narrows nothing in it, so the group gives way itself: it shows the fullest step that fits.
/// A step not measured yet is tried, so a new window settles within a few passes; the widths don't depend on the
/// room, so a resize picks the step at once.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Fit {
    widths: [Option<f32>; FIT_STEPS],
    room: Option<f32>,
}

impl Fit {
    /// The step to show: the fullest one whose width fits the room, or one not measured yet before it; the most compact
    /// when none fits. The fullest while the room isn't known.
    pub(crate) fn step(&self) -> usize {
        let Some(room) = self.room else { return 0 };
        for (step, width) in self.widths.iter().enumerate() {
            match width {
                None => return step,
                Some(w) if *w <= room + 0.5 => return step,
                Some(_) => {}
            }
        }
        FIT_STEPS - 1
    }

    /// Notes the width `step` took and the room the group had in this pass; true when either changed, so the next pass
    /// may pick another step. A step whose width changed (the language, the interface size) forgets the others' widths,
    /// which are measured again.
    pub(crate) fn note(&mut self, step: usize, width: f32, room: f32) -> bool {
        let (width, room) = (width.max(0.0), room.max(0.0));
        if !width.is_finite() || !room.is_finite() {
            return false;
        }
        let mut changed = self.room.is_none_or(|r| (r - room).abs() > 0.5);
        self.room = Some(room);
        let Some(slot) = self.widths.get_mut(step) else { return changed };
        if slot.is_none_or(|w| (w - width).abs() > 0.5) {
            if slot.is_some() {
                self.widths = [None; FIT_STEPS];
            }
            if let Some(slot) = self.widths.get_mut(step) {
                *slot = Some(width);
            }
            changed = true;
        }
        changed
    }
}

/// The smallest rect holding every rect of `rects` that has an area; `None` when none has.
pub(crate) fn union(rects: impl IntoIterator<Item = egui::Rect>) -> Option<egui::Rect> {
    rects.into_iter().filter(|r| r.is_positive()).reduce(|a, b| a.union(b))
}

/// The direction the interface flows in, for the text that carries a file name or path.
pub(crate) fn interface_direction() -> craft::i18n::Direction {
    if crate::i18n::current().rtl() { craft::i18n::Direction::Rtl } else { craft::i18n::Direction::Ltr }
}

/// Why the caret's language isn't spell-checked (dictionary unreadable or missing), if it isn't.
pub(crate) fn dictionary_problem(lang: &str, unreadable: &Option<(std::path::PathBuf, String)>, missing: Option<&str>) -> Option<String> {
    match (unreadable, missing) {
        (Some((file, reason)), _) => Some(crate::i18n::fmt(
            tl!("The spelling dictionary for {language} couldn't be read, so it isn't checked. File: {file}. Reason: {reason}"),
            &[("language", lang), ("file", &craft::i18n::path(&file.to_string_lossy(), interface_direction())), ("reason", reason)],
        )),
        (None, Some(name)) => Some(crate::i18n::fmt(
            tl!(
                "No spelling dictionary for {language} is installed, so it isn't checked. WordCraft looks for the Hunspell dictionary {name} in the WORDCRAFT_DICTIONARIES folder and in the usual system locations."
            ),
            &[("language", lang), ("name", name)],
        )),
        (None, None) => None,
    }
}

/// Commands the Quick Access Toolbar's menu offers to show or hide with one click (#376).
pub(super) const QAT_MENU: [&str; 10] =
    ["file.new", "file.open", "file.save", "file.print", "edit.undo", "edit.redo", "edit.repeat", "review.spelling", "view.readMode", "insert.table"];

/// Match the toolbar's enabled state for keyboard activation too.
pub(crate) fn qat_enabled(ctx: &egui::Context, app: &WordApp, id: &str) -> bool {
    match id {
        // In the Markdown Source pane Undo and Redo are the field's own typing history (S-L187), which the
        // runtime resolves; the session's steps are not what they act on.
        "edit.undo" | "edit.redo" if app.session.md_source.is_some() => {
            craft::ui::runtime::commands::resolve(ctx, craft::keymap::CommandRef::new(id)).enabled
        }
        "edit.undo" => app.session.can_undo(),
        "edit.redo" => app.session.can_redo(),
        "file.save" => app.has_document(),
        _ => craft::ui::runtime::commands::resolve(ctx, craft::keymap::CommandRef::new(id)).enabled,
    }
}

/// The Undo list's rows, newest first: each step named as the Undo button's tooltip names it.
pub(crate) fn undo_list_rows(app: &WordApp) -> Vec<String> {
    app.session.undo_history().into_iter().map(|l| tl!(l).to_string()).collect()
}

/// What fills the screen (#1678): the usual window, Focus (the document on black, a slim bar to
/// drop the ribbon down) or Full Screen (the pages alone, a slim bar with Search). Both of the
/// latter take the whole screen and hide the title bar, ribbon, status bar, panes and rulers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Normal,
    Focus,
    FullScreen,
}

impl Screen {
    pub fn of(v: &wordcraft_engine::ViewState) -> Self {
        if v.focus_mode {
            Screen::Focus
        } else if v.full_screen_view {
            Screen::FullScreen
        } else {
            Screen::Normal
        }
    }

    /// The command that leaves this screen, if it isn't the usual window.
    pub fn exit_command(self) -> Option<&'static str> {
        match self {
            Screen::Normal => None,
            Screen::Focus => Some("view.focus"),
            Screen::FullScreen => Some("view.fullScreenView"),
        }
    }

    /// The shell's screen mode for the view: Focus, Immersive (Full Screen View), Full Screen With Menu
    /// Bar, else Standard.
    pub fn mode(v: &wordcraft_engine::ViewState) -> craft::ui::shell::ScreenMode {
        use craft::ui::shell::ScreenMode;
        if v.focus_mode {
            ScreenMode::Focus
        } else if v.full_screen_view {
            ScreenMode::Immersive
        } else if v.full_screen {
            ScreenMode::FullScreenWithMenuBar
        } else {
            ScreenMode::Standard
        }
    }
}

/// [`WordApp::cached_missing_fonts`]: the list, and the warnings by font key.
pub type MissingFontsCache =
    (std::sync::Arc<Vec<wordcraft_engine::cmd::font::MissingFont>>, std::sync::Arc<std::collections::HashMap<String, String>>);

/// [`WordApp::missing_fonts`]: the layout the list was read from, and the list.
pub type MissingFontsMemo = (std::sync::Weak<wordcraft_layout::DocLayout>, MissingFontsCache);

impl WordApp {
    /// [`wordcraft_engine::cmd::font::missing_fonts`] for `layout`, and the font box's warnings
    /// by [`font_key`](wordcraft_engine::cmd::font::font_key) (the layout-changing ones, with the
    /// faces drawn instead), read again only when the document is laid out again (adding or
    /// removing a font lays it out again).
    pub fn cached_missing_fonts(&mut self, layout: &std::sync::Arc<wordcraft_layout::DocLayout>) -> MissingFontsCache {
        if !self.missing_fonts.0.upgrade().is_some_and(|seen| std::sync::Arc::ptr_eq(&seen, layout)) {
            let found = wordcraft_engine::cmd::font::missing_fonts(&self.session.doc, layout);
            let warnings = found
                .iter()
                .filter(|f| !f.metric_compatible)
                .map(|f| (wordcraft_engine::cmd::font::font_key(&f.font), f.shown_as.join(", ")))
                .collect();
            self.missing_fonts = (std::sync::Arc::downgrade(layout), (std::sync::Arc::new(found), std::sync::Arc::new(warnings)));
        }
        self.missing_fonts.1.clone()
    }

    /// The font box's warning for `font` (a stored name): the faces its text is drawn in
    /// instead, when the status bar lists it as missing and changing the layout. Read from the
    /// same list as the status bar, so the two never disagree. At a collapsed caret, where the
    /// box shows the font typing will use (an empty paragraph's, or one just picked at the end
    /// of a line), the substitute layout will draw it in is named instead.
    pub fn font_box_warning(&mut self, font: &str) -> Option<String> {
        let ahead = self.session.sel.is_collapsed();
        self.font_box_warning_or(font, ahead, |doc| wordcraft_engine::cmd::font::shown_as(doc, font))
    }

    /// The Font dialog's font box's warning for `font`, the font OK applies: from the status
    /// bar's list, else the substitute layout will draw it in, whatever is selected, as no text
    /// may show the font yet.
    pub fn font_dialog_warning(&mut self, font: &str) -> Option<String> {
        self.font_box_warning_or(font, true, |doc| wordcraft_engine::cmd::font::shown_as(doc, font))
    }

    /// The Font dialog's Asian text font box's warning for `font_ea` (the Latin text font being
    /// `font`): the face layout draws Chinese, Japanese and Korean text in when `font_ea` is
    /// missing (the run's font or a fallback face). Not from the status bar's list, which is by
    /// font name: a font also used for Latin text is listed with the faces that text is drawn
    /// in. Whatever is selected: selected Latin text shows no East Asian font, but text typed or
    /// pasted into it would be in it.
    pub fn east_asian_font_box_warning(&mut self, font_ea: &str, font: &str) -> Option<String> {
        let shown = wordcraft_engine::cmd::font::east_asian_shown_as(&self.session.doc, font_ea, font)?;
        wordcraft_engine::cmd::font::warns(font_ea, &shown).then_some(shown)
    }

    /// The font box's warning for `font`: from the status bar's list, else, when the box shows
    /// a font to come (`ahead`), the family `unshown` says its text will be drawn in.
    fn font_box_warning_or(&mut self, font: &str, ahead: bool, unshown: impl FnOnce(&wordcraft_doc::Document) -> Option<String>) -> Option<String> {
        let layout = self.session.layout();
        let key = wordcraft_engine::cmd::font::font_key(font);
        if let Some(w) = self.cached_missing_fonts(&layout).1.get(&key) {
            return Some(w.clone());
        }
        // A font to come (typing at a collapsed caret, or what the Font dialog applies) may be in
        // no shown text yet, so the list can't have it: name the family it will be drawn in. A
        // ribbon box over a selection shows the selected text's font, which the list covers.
        let shown = ahead.then(|| unshown(&self.session.doc)).flatten()?;
        wordcraft_engine::cmd::font::warns(font, &shown).then_some(shown)
    }

    /// The document's counts, recomputed only when the document (or the count option) changes:
    /// words as the status bar and the Word Count dialog count them, and characters with and
    /// without spaces as the dialog does (#2797).
    pub fn cached_status_counts(&mut self) -> StatusCounts {
        // The count option changes the counts without a document change.
        let key = self.session.rev().wrapping_mul(2) | u64::from(self.session.prefs.count_notes);
        if self.status_counts.0 != key {
            let text = self.session.counted_text();
            self.status_counts = (
                key,
                StatusCounts {
                    words: self.session.word_count(),
                    chars: wordcraft_doc::count_chars(&text),
                    chars_no_spaces: wordcraft_doc::count_chars_no_spaces(&text),
                },
            );
        }
        self.status_counts.1
    }
}

/// The status bar's count line (#2797): the whole document's `total`, or "selected of total" when
/// part of it is selected, worded as the Word Count dialog words the same counts.
pub(crate) fn status_count_text(unit: StatusCount, total: usize, selected: Option<usize>) -> String {
    match (unit, selected) {
        (StatusCount::Words, None) => crate::i18n::fmt(tl!("{words} words"), &[("words", &total.to_string())]),
        (StatusCount::Words, Some(n)) => {
            crate::i18n::fmt(tl!("{selected} of {words} words"), &[("selected", &n.to_string()), ("words", &total.to_string())])
        }
        (StatusCount::Characters, None) => crate::i18n::fmt(tl!("{chars} characters"), &[("chars", &total.to_string())]),
        (StatusCount::Characters, Some(n)) => {
            crate::i18n::fmt(tl!("{selected} of {chars} characters"), &[("selected", &n.to_string()), ("chars", &total.to_string())])
        }
        (StatusCount::CharactersNoSpaces, None) => crate::i18n::fmt(tl!("{chars} characters (no spaces)"), &[("chars", &total.to_string())]),
        (StatusCount::CharactersNoSpaces, Some(n)) => {
            crate::i18n::fmt(tl!("{selected} of {chars} characters (no spaces)"), &[("selected", &n.to_string()), ("chars", &total.to_string())])
        }
    }
}

#[cfg(test)]
mod tests;
