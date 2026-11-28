//! The status bar: page, counts, part being edited, language, missing fonts, Overtype, the message, and the
//! view and zoom controls.

use std::cell::RefCell;

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::theme::ControlSize;
use craft::theme::variant::Variant;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::{Button, ButtonOutput, IconButton, ToggleButton};
use craft::ui::contract::{Bind, BodyCx, Gap, Props as _, Show as _, ShowContainer as _, TooltipSpec, Trigger};
use craft::ui::display::{Divider, Text};
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::value_drag::{Slider, SliderScale};
use egui::Ui;
use serde_json::json;
use wordcraft_doc::{PartKind, StoryRef};

use super::{dictionary_problem, status_count_text};
use crate::WordApp;

/// The largest zoom `view.zoom` takes, in percent (its range in `crates/engine/src/cmd/view.rs`): the zoom button
/// keeps the room of this value's text, so the controls before it hold still as the percentage changes (#1975).
const MOST_ZOOM_PERCENT: u32 = 500;

/// Fills the shell's status slot; with no document it shows nothing (#1129).
pub(crate) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, app: &mut WordApp) {
    if !app.has_document() {
        return;
    }
    // The controls give way in a narrow window (#5886): the step that fits the room the page left them in the last
    // pass.
    let step = app.chrome.status_fit.step();
    // The bar runs its body and its trailing region one after the other, never one inside the other, so each
    // borrow below succeeds.
    let app = RefCell::new(app);
    let (page, group) = (std::cell::Cell::new(None), std::cell::Cell::new(None));
    let bar = Bar::new(tl!("Status Bar").to_string(), BarKind::Status)
        .trailing(|ui, _| {
            if let Ok(mut a) = app.try_borrow_mut() {
                group.set(controls(&mut a, ui, step));
            }
        })
        .show(ui, |ui, _| {
            if let Ok(mut a) = app.try_borrow_mut() {
                page.set(Some(readouts(&mut a, ui)));
            }
        });
    if let (Some(page), Some(group), Ok(mut a)) = (page.get(), group.get(), app.try_borrow_mut()) {
        // The texts scroll under the controls, and one shown only in part isn't drawn; the page is always shown, so
        // the controls have the bar less the page and the bar's pads and gaps.
        let room = bar.output.response.rect.width() - page.width() - STATUS_FIT_GAP;
        if a.chrome.status_fit.note(step, group.width(), room) {
            ui.ctx().request_repaint();
        }
    }
}

/// The room the status bar keeps beside the page and the controls: its pads at both ends and the gaps before and
/// after its end region.
const STATUS_FIT_GAP: f32 = 32.0;

/// A status item: a ghost button of `text`, with an optional tooltip.
fn item(ui: &mut Ui, text: String, tip: Option<String>) -> ButtonOutput {
    let mut button = Button::new(text).variant(Variant::GHOST);
    if let Some(tip) = tip {
        button = button.tooltip(TooltipSpec::new(tip));
    }
    button.show(ui)
}

/// The page in view (#3880), as the scrollbar's tip names it.
fn page_in_view(app: &mut WordApp) -> (usize, usize) {
    let l = app.session.layout();
    let last = l.pages.len().saturating_sub(1);
    let page = if let Some(p) = app.canvas.viewed_page {
        p.min(last) + 1
    } else if app.session.selects_whole_story() {
        app.session.page_hint.min(last) + 1
    } else {
        l.caret_on(&app.session.sel.focus, app.session.page_hint).map(|c| c.page + 1).unwrap_or(1)
    };
    (page, l.pages.len())
}

/// The members before the controls: what the document is and where the caret is. Returns the page item's rect.
fn readouts(app: &mut WordApp, ui: &mut Ui) -> egui::Rect {
    let (page, pages) = page_in_view(app);
    let text = crate::i18n::fmt(tl!("Page {page} of {pages}"), &[("page", &page.to_string()), ("pages", &pages.to_string())]);
    // Bound to Go To, which opens its tab: the tooltip ends with Go To's key as this platform writes it
    // ("Go To (Ctrl+Alt+G)", "Go To (⌘⌥G)" on macOS, #3172).
    let page = Button::command(CommandRef::new("edit.goto")).label(text).variant(Variant::GHOST).tooltip(tl!("Go To").to_string()).show(ui);
    count(app, ui);
    if app.session.sel.focus.story != StoryRef::Body && !crate::canvas::in_text_box(app) {
        let label = match crate::canvas::part_kind(app, app.session.sel.focus.story) {
            Some(PartKind::Footnote) => tl!("Editing footnote"),
            Some(PartKind::Endnote) => tl!("Editing endnote"),
            _ => tl!("Editing header/footer"),
        };
        Text::new(label.to_string()).show(ui);
    }
    language(app, ui);
    if app.session.doc.settings.track_changes {
        Text::new(tl!("Track Changes: On").to_string()).show(ui);
    }
    missing_fonts(app, ui);
    if app.session.prefs.overtype && item(ui, tl!("Overtype").to_string(), Some(tl!("Click to switch back to inserting").to_string())).clicked() {
        let _ = app.run("text.overtype", json!({"value": false}));
    }
    if let Some(message) = app.status_message() {
        // The whole message is in the tooltip; the bar has room for the beginning of it (#2402).
        let text = message.text.clone();
        message.tooltip(text).show(ui);
    }
    page.response.rect
}

/// The live count: words (the default, as Word's status bar counts), characters with spaces, or characters
/// without spaces, whichever was chosen (#2797). A click opens Word Count; a right click the choice.
fn count(app: &mut WordApp, ui: &mut Ui) {
    let unit = app.ui.status_count;
    let counts = app.cached_status_counts();
    // A selected picture/shape/text box isn't a text selection: show the total.
    let text = if app.session.sel.is_collapsed() || crate::objects::selected(app).is_some() {
        status_count_text(unit, counts.in_unit(unit), None)
    } else {
        let selected = crate::count_in_unit(&app.session.selected_text(), unit);
        status_count_text(unit, counts.in_unit(unit), Some(selected))
    };
    let out = item(ui, text, None);
    if out.clicked() {
        let _ = app.run("ui.dialog", json!({"name": "wordCount"}));
    }
    // No `open` binding: with one the framework leaves the opening to the app, without it the opener opens the menu.
    Menu::context(tl!("Status count").to_string(), &out.response).values().show(ui, |ui, _| {
        for c in crate::StatusCount::ALL {
            MenuItem::command(CommandRef::with_params("ui.statusCount", json!({"value": c.code()})))
                .label(tl!(c.label()).to_string())
                .entry_checked(unit == c)
                .show(ui);
        }
    });
}

/// The language at the caret; a click sets the proofing language (Review › Language).
fn language(app: &mut WordApp, ui: &mut Ui) {
    let lang = crate::ribbon::caret_language(app);
    let missing = crate::ribbon::caret_missing_dictionary(app);
    let unreadable = crate::ribbon::caret_unreadable_dictionary(app);
    let shown = if unreadable.is_some() {
        crate::i18n::fmt(tl!("{language} (dictionary unreadable)"), &[("language", &lang)])
    } else if missing.is_some() {
        crate::i18n::fmt(tl!("{language} (no dictionary)"), &[("language", &lang)])
    } else {
        lang.clone()
    };
    // The tooltip names the language in full: in a narrow window the item's text is shortened (#5886).
    let tip = dictionary_problem(&lang, &unreadable, missing).unwrap_or_else(|| shown.clone());
    let out = item(ui, shown, Some(tip));
    let menu =
        Menu::new(tl!("Language").to_string()).opened_by(&out.response, Trigger::Click).show(ui, |ui, _| crate::ribbon::language_menu(ui, app));
    // Dismissed, the menu hands focus back to this item; the page takes the keyboard back, so typing goes on in the
    // document (#1093). A press that closed it on a field leaves the field its focus.
    use craft::ui::runtime::overlay::CloseReason;
    if matches!(menu.closed, Some(CloseReason::Outside | CloseReason::Escape)) {
        app.canvas.focus_page_after_menu(ui.ctx(), out.response.id);
    }
}

/// Fonts the document uses that aren't installed: they're drawn in other fonts, so the text can wrap and
/// paginate unlike where the document was made.
fn missing_fonts(app: &mut WordApp, ui: &mut Ui) {
    let layout = app.session.layout();
    let missing = app.cached_missing_fonts(&layout).0;
    let count = missing.iter().filter(|f| !f.metric_compatible).count();
    if count == 0 {
        return;
    }
    // A hostile file can name thousands of fonts: list the first few.
    const SHOWN: usize = 20;
    let label = crate::i18n::tn(count as u64, "{n} font not installed", "{n} fonts not installed");
    // The warning sign before the count, as the old bar's (prod `chrome.rs:843`).
    let out = Button::new(label)
        .icon(IconId::new("fontMissing"))
        .variant(Variant::GHOST)
        .tooltip(TooltipSpec::new(tl!("Shown in substitute fonts. Click for details.").to_string()))
        .show(ui);
    Menu::new(tl!("Missing fonts").to_string()).opened_by(&out.response, Trigger::Click).show(ui, |ui, _| {
        MenuItem::heading(tl!("Not installed — shown as").to_string()).show(ui);
        for f in missing.iter().filter(|f| !f.metric_compatible).take(SHOWN) {
            MenuItem::heading(format!("{} → {}", f.font, f.shown_as.join(", "))).show(ui);
        }
        if count > SHOWN {
            MenuItem::heading(crate::i18n::tn((count - SHOWN) as u64, "…and {n} more", "…and {n} more")).show(ui);
        }
        Divider::horizontal().show(ui);
        if missing.iter().any(|f| f.download.is_some()) {
            MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "fontDownload"})))
                .label(tl!("Get the Missing Fonts…").to_string())
                .show(ui);
        }
        MenuItem::command(CommandRef::new("font.add")).label(tl!("Add Font…").to_string()).show(ui);
    });
}

/// The view, zoom and Focus controls, which stay at the reading end outside the bar's scroll; returns the rect they
/// took. In a narrow window they give way by `fit` (#5886) so the page keeps its room: at 1 the zoom slider is left
/// out (Zoom Out, Zoom In and the percentage stay), at 2 Focus shows its icon alone as well.
fn controls(app: &mut WordApp, ui: &mut Ui, fit: usize) -> Option<egui::Rect> {
    use craft::ui::contract::Outcome as _;
    let mut rects = Vec::new();
    let _ = Row::new().gap(Gap::Section).show(ui, |ui, _| {
        let focus = if fit >= 2 {
            IconButton::command(IconId::new("focus"), CommandRef::new("view.focus")).label(tl!("Focus").to_string()).show(ui).response
        } else {
            Button::command(CommandRef::new("view.focus"))
                .label(tl!("Focus").to_string())
                .icon(IconId::new("focus"))
                .variant(Variant::GHOST)
                .show(ui)
                .response
        };
        rects.push(focus.rect);
        // Icons only, Read Mode first and Web Layout last, each named in its tooltip, the current view on (prod
        // `chrome.rs:925-943`).
        let _ = Row::new().gap(Gap::Item).show(ui, |ui, _| {
            let (mode, read) = (app.session.view.mode, app.session.view.read_mode);
            for (id, icon, name, on) in [
                ("view.readMode", "readMode", tl!("Read Mode"), read),
                ("view.printLayout", "printLayout", tl!("Print Layout"), mode == wordcraft_layout::ViewMode::Print && !read),
                ("view.webLayout", "webLayout", tl!("Web Layout"), mode == wordcraft_layout::ViewMode::Web),
            ] {
                // No command carries a checked state, so the view is given at the button (S-L415).
                let out = ToggleButton::new(name.to_string(), Bind::given(on))
                    .command(CommandRef::new(id))
                    .icon(crate::icons::spec(icon))
                    .icon_only()
                    .tooltip(TooltipSpec::new(name.to_string()))
                    .show(ui);
                rects.push(out.response().rect);
            }
        });
        let reading = app.session.view.read_mode;
        // Reading zoom changes only reading presentation, never the normal view zoom.
        let current = if reading { f64::from(app.session.view.read_zoom) } else { app.canvas.zoom() };
        let percent = (current * 100.0).round();
        let step = |app: &mut WordApp, plus: bool| {
            if reading {
                let value = if plus { (percent + 10.0).min(150.0) } else { (percent - 10.0).max(75.0) };
                let _ = app.run("view.readTextSize", json!({"value": value}));
            } else {
                let _ = app.run(if plus { "view.zoomIn" } else { "view.zoomOut" }, json!({}));
            }
        };
        let _ = Row::new().show(ui, |ui, _| {
            let out = IconButton::new(IconId::new("wordcraft.minus"))
                .label(tl!("Zoom Out").to_string())
                .tooltip(TooltipSpec::new(tl!("Zoom Out").to_string()))
                .show(ui);
            rects.push(out.response.rect);
            if out.response.clicked() {
                step(app, false);
            }
            if fit < 1 {
                let mut value = current;
                let range = if reading { 0.75..=1.5 } else { 0.1..=5.0 };
                let name = if reading { tl!("Text Size") } else { tl!("Zoom") };
                let slider = Slider::new(Bind::bound(&mut value), range)
                    .size(ControlSize::Md)
                    .scale(if reading { SliderScale::Linear } else { SliderScale::Log })
                    // A plain rail, as the old zoom slider's (prod `chrome.rs:911`): no fill up to the thumb.
                    .fill(false)
                    .label(name.to_string())
                    .show(ui);
                rects.push(slider.response().rect);
                if let Some(v) = slider.value() {
                    let command = if reading { "view.readTextSize" } else { "view.zoom" };
                    let _ = app.run(command, json!({"value": (*v * 100.0).round()}));
                }
            }
            let out = IconButton::new(IconId::new("wordcraft.plus"))
                .label(tl!("Zoom In").to_string())
                .tooltip(TooltipSpec::new(tl!("Zoom In").to_string()))
                .show(ui);
            rects.push(out.response.rect);
            if out.response.clicked() {
                step(app, true);
            }
            if !reading {
                // PLAN: assumed Button::room_for (ribbon-w2, S-L418)
                let out = Button::new(format!("{percent}%")).variant(Variant::GHOST).room_for(format!("{MOST_ZOOM_PERCENT}%")).show(ui);
                rects.push(out.response.rect);
                if out.clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "zoom"}));
                }
            }
        });
    });
    super::union(rects)
}
