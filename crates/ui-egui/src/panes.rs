//! Side panes: Navigation (headings, pages, search results), Clipboard, Styles, Style Inspector,
//! Accessibility and Alt Text (see `a11y_pane`), Format Shape (see `format_shape`), Restrict Editing ([`crate::dialogs_review`]), Comments,
//! and the SmartArt Text Pane.

use std::collections::BTreeMap;

use egui::{Stroke, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_doc::numbering::{Counters, LevelSuffix};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Document, Path, Pos, StoryRef};

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // SmartArt Design › Text Pane, while a SmartArt graphic WordCraft can edit is selected.
    if app.session.view.smart_art_pane && wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_some() {
        egui::Panel::left("smart_art_pane")
            .default_size(250.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                if header(ui, "Text Pane") {
                    let _ = app.run("smartArt.textPane", json!({"value": false}));
                    return;
                }
                crate::dialogs_smart_art::text_pane(app, ui);
            });
    }
    if app.session.view.nav_pane {
        egui::Panel::left("nav_pane")
            .default_size(260.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| nav(app, ui));
    }
    if app.session.view.clipboard_pane {
        egui::Panel::left("clipboard_pane")
            .default_size(250.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| clipboard(app, ui));
    }
    if app.session.view.styles_pane {
        egui::Panel::right("styles_pane")
            .default_size(250.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| styles(app, ui));
    }
    if app.session.view.style_inspector {
        egui::Panel::right("style_inspector")
            .default_size(240.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| inspector(app, ui));
    }
    if app.session.view.accessibility_pane {
        egui::Panel::right("accessibility_pane")
            .default_size(280.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| crate::a11y_pane::accessibility(app, ui));
    }
    if app.session.view.alt_text_pane {
        egui::Panel::right("alt_text_pane")
            .default_size(260.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| crate::a11y_pane::alt_text(app, ui));
    }
    if !app.session.view.format_shape_pane {
        crate::format_shape::forget_drag(ui.ctx());
    } else {
        egui::Panel::right("format_shape_pane")
            .default_size(280.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| crate::format_shape::pane(app, ui)));
    }
    if app.restrict.open {
        egui::Panel::right("restrict_pane")
            .default_size(270.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                if header(ui, "Restrict Editing") {
                    app.restrict.open = false;
                    return;
                }
                crate::dialogs_review::restrict_pane(app, ui);
            });
    }
    if app.session.view.comments_pane {
        egui::Panel::right("comments_pane")
            .default_size(290.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| comments(app, ui));
    }
}

pub(crate) fn header(ui: &mut Ui, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!(title)).font(semibold(15.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Drawn, not the "✕" character: the interface fonts have no glyph for it.
            if crate::widgets::icon_button(ui, "close", tl!("Close")).clicked() {
                close = true;
            }
        });
    });
    ui.add_space(4.0);
    close
}

fn nav(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Navigation") {
        let _ = app.run("view.navigationPane", json!({"value": false}));
        return;
    }
    let qid = egui::Id::new("nav_query");
    let mut q = ui.data(|d| d.get_temp::<String>(qid)).unwrap_or_default();
    // Ctrl+F with text selected searches for it, as Word does (a selection within one line).
    let focus = std::mem::take(&mut app.nav_search_focus);
    let picked = focus.then(|| app.session.selected_text()).filter(|t| !t.is_empty() && !t.contains(['\n', '\r']) && t.chars().count() <= 255);
    if let Some(t) = &picked {
        q = t.clone();
    }
    let mut out = egui::TextEdit::singleline(&mut q).hint_text(tl!("Search document")).desired_width(f32::INFINITY).show(ui);
    let r = out.response.clone();
    if focus {
        // Focused with its text selected, so typing replaces it.
        r.request_focus();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(q.chars().count()));
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), r.id);
    }
    if r.changed() || picked.is_some() {
        app.ui.nav_tab = "results".into();
        ui.data_mut(|d| d.insert_temp(qid, q.clone()));
        if !q.is_empty() {
            let _ = app.session.run("edit.find", &json!({"text": q}));
        } else {
            app.session.find.results.clear();
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (k, l) in [("headings", "Headings"), ("pages", "Pages"), ("results", "Results")] {
            if ui.selectable_label(app.ui.nav_tab == k, tl!(l)).clicked() {
                app.ui.nav_tab = k.into();
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| match app.ui.nav_tab.as_str() {
        "pages" => {
            let l = app.session.layout();
            for (i, p) in l.pages.iter().enumerate() {
                let label = crate::i18n::fmt(tl!("Page {page}"), &[("page", &p.number.to_string())]);
                if ui.selectable_label(app.session.page_hint == i, label).clicked() {
                    let _ = app.run("edit.goto", json!({"page": i + 1}));
                }
            }
        }
        "results" => {
            let results = app.session.find.results.clone();
            ui.label(egui::RichText::new(format!("{} results", results.len())).small().weak());
            for (a, b) in results.iter().take(500) {
                let Some(p) = app.session.doc.para_at(a) else { continue };
                let from = p.clamp(a.off.saturating_sub(30));
                let to = p.clamp((b.off + 40).min(p.len()));
                let pre = p.text.get(from..a.off).unwrap_or("");
                let hit = p.text.get(a.off..b.off).unwrap_or("");
                let post = p.text.get(b.off..to).unwrap_or("");
                let mut job = egui::text::LayoutJob::default();
                let fmt = |c| egui::TextFormat { font_id: regular(12.0), color: c, ..Default::default() };
                job.append(pre, 0.0, fmt(ui.visuals().text_color()));
                // The match is marked the way Find marks it on the page, per theme (#656).
                let mark = Tokens::get(ui.ctx()).find_highlight;
                job.append(hit, 0.0, egui::TextFormat { font_id: semibold(12.0), background: mark, color: ui.visuals().text_color(), ..Default::default() });
                job.append(post, 0.0, fmt(ui.visuals().text_color()));
                job.wrap.max_width = ui.available_width();
                if ui.add(egui::Button::new(job).frame(false)).clicked() {
                    let _ = app.run("select.range", json!({"anchor": a, "focus": b}));
                }
                ui.separator();
            }
        }
        _ => {
            let caret = app.session.sel.focus.clone();
            let items = nav_headings(&app.session.doc);
            let current = items.iter().rposition(|(path, ..)| *path <= caret.path);
            if items.is_empty() {
                ui.label(egui::RichText::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.")).weak());
            }
            // A heading has children when the next heading is a deeper level (#1997).
            let has_children: Vec<bool> = items.iter().enumerate().map(|(i, (_, lv, _))| items.get(i + 1).is_some_and(|n| n.1 > *lv)).collect();
            let collapsed_flags: Vec<bool> = items.iter().enumerate().map(|(i, (p, _, _))| has_children[i] && app.ui.nav_collapsed.contains(p)).collect();
            let levels: Vec<u8> = items.iter().map(|(_, lv, _)| *lv).collect();
            let visible = visible_headings(&levels, &collapsed_flags);
            for (i, (path, lv, txt)) in items.into_iter().enumerate() {
                if !visible[i] {
                    continue;
                }
                let collapsed = collapsed_flags[i];
                let (toggle, r) = nav_heading(ui, lv, &txt, current == Some(i), has_children[i].then_some(collapsed));
                if toggle {
                    if !app.ui.nav_collapsed.remove(&path) {
                        app.ui.nav_collapsed.insert(path);
                    }
                } else if r.clicked() {
                    let _ = app.run("caret.set", json!({"pos": Pos { story: StoryRef::Body, path, off: 0 }}));
                    // Align the chosen heading's page immediately against this pane, rather
                    // than the indeterminate minimum movement from scroll_to_rect.
                    app.canvas.nav_to_caret = true;
                    app.canvas.scroll_to_caret = false;
                    app.canvas.want_focus = true;
                }
            }
        }
    });
}

/// Which headings the pane shows: those under a collapsed heading (every following heading
/// of a deeper level) are hidden (#1997).
fn visible_headings(levels: &[u8], collapsed: &[bool]) -> Vec<bool> {
    let mut hide_below: Option<u8> = None;
    levels
        .iter()
        .zip(collapsed)
        .map(|(&lv, &c)| {
            if hide_below.is_some_and(|h| lv > h) {
                return false;
            }
            hide_below = c.then_some(lv);
            true
        })
        .collect()
}

/// The body's headings (paragraphs with an outline level) in document order, as (path, level,
/// text). A numbered heading starts with the number it shows (`2.1 Method`), as in Word.
fn nav_headings(doc: &Document) -> Vec<(Path, u8, String)> {
    // Every numbered paragraph advances the list counters, headings or not.
    let mut counters = Counters::default();
    let mut items = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        let rp = doc.styles.resolve_para(&p.props);
        let number = p
            .props
            .numbering
            .or(rp.numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None));
        if let Some(lv) = rp.outline_level {
            let txt = p.plain_text().trim().to_string();
            if txt.is_empty() || rp.style.starts_with("TOC") {
                continue;
            }
            let txt = match number {
                Some((label, def)) if def.suffix == LevelSuffix::Nothing => format!("{label}{txt}"),
                Some((label, _)) => format!("{label} {txt}"),
                None => txt,
            };
            items.push((path, lv, txt));
        }
    }
    items
}

/// Constrain each heading to the available pane width so long document text cannot
/// enlarge the panel. A heading too long for the width wraps onto further lines — a word
/// wider than the pane is broken too — and the row grows with it, so the label reads whole
/// instead of ending in an ellipsis (#3015).
fn nav_heading(ui: &mut Ui, level: u8, text: &str, selected: bool, collapsed: Option<bool>) -> (bool, egui::Response) {
    ui.horizontal(|ui| {
        ui.add_space(level as f32 * 14.0);
        // The triangle is drawn in code; headings without children keep an empty slot so text aligns.
        let mut toggled = false;
        let (rect, tri) = ui.allocate_exact_size(vec2(14.0, ui.spacing().interact_size.y), egui::Sense::click());
        if let Some(c) = collapsed {
            let ctr = rect.center();
            let col = ui.visuals().text_color();
            let pts = if c {
                vec![ctr + vec2(-2.5, -4.0), ctr + vec2(-2.5, 4.0), ctr + vec2(3.0, 0.0)]
            } else {
                vec![ctr + vec2(-4.0, -2.5), ctr + vec2(4.0, -2.5), ctr + vec2(0.0, 3.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            toggled = tri.clicked();
        }
        let label = egui::RichText::new(text).font(if level == 0 { semibold(12.5) } else { regular(12.5) });
        let width = ui.available_width();
        let r = ui.add(
            // The trailing grow atom takes the spare width, so the text starts flush left
            // under its parent instead of centred, as in Word (#1199). A minimum height
            // keeps short headings on one row; a wrapped one becomes as tall as it needs.
            egui::Button::selectable(selected, (label, egui::Atom::grow())).wrap().min_size(vec2(width, ui.spacing().interact_size.y)),
        );
        (toggled, r)
    })
    .inner
}

/// Items collected by Copy and Cut, newest first; clicking one pastes it at the caret.
fn clipboard(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Clipboard") {
        let _ = app.run("edit.clipboardPane", json!({"value": false}));
        return;
    }
    let items: Vec<String> = app.session.clip_history.items().iter().map(|i| i.preview()).collect();
    ui.horizontal(|ui| {
        if ui.add_enabled(!items.is_empty(), egui::Button::new(tl!("Paste All"))).clicked() {
            let _ = app.run("edit.pasteAllClipboard", json!({}));
            app.canvas.want_focus = true;
        }
        if ui.add_enabled(!items.is_empty(), egui::Button::new(tl!("Clear All"))).clicked() {
            let _ = app.run("edit.clearClipboard", json!({}));
            app.canvas.want_focus = true;
        }
    });
    ui.add_space(4.0);
    let hint = if items.is_empty() { "Nothing collected yet. Items you copy or cut appear here." } else { "Click an item to paste it." };
    ui.label(egui::RichText::new(tl!(hint)).small().weak());
    ui.separator();
    let t = Tokens::get(ui.ctx());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, preview) in items.iter().enumerate() {
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let text = if preview.is_empty() { tl!("(picture or object)").to_string() } else { preview.clone() };
                let mut job =
                    egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: regular(12.5), color: t.text, ..Default::default() });
                job.wrap.max_width = ui.available_width();
                job.wrap.max_rows = 3;
                if ui.add(egui::Button::new(job).frame(false)).on_hover_text(tl!("Paste")).clicked() {
                    let _ = app.run("edit.pasteClipboardItem", json!({"index": i}));
                    app.canvas.want_focus = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.small_button(tl!("Delete")).clicked() {
                        let _ = app.run("edit.deleteClipboardItem", json!({"index": i}));
                        app.canvas.want_focus = true;
                    }
                });
            });
            ui.add_space(6.0);
        }
    });
}

fn styles(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Styles") {
        let _ = app.run("view.stylesPane", json!({"value": false}));
        return;
    }
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    let cur = st.get("style").and_then(Value::as_str).unwrap_or("Normal").to_string();
    let current_name = app
        .session
        .doc
        .styles
        .get(&cur)
        .map(crate::i18n::style_name)
        .unwrap_or_else(|| st.get("styleName").and_then(Value::as_str).unwrap_or("").to_string());
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Current style: {style}"), &[("style", &current_name)])).small());
    ui.horizontal(|ui| {
        if ui.button(tl!("New Style…")).clicked() {
            app.dialog = crate::dialogs::Dialog::open("newStyle", app);
        }
        if ui.button(tl!("Update to Match")).clicked() {
            let _ = app.run("styles.updateToMatch", json!({}));
        }
        if ui.button(tl!("Clear")).clicked() {
            let _ = app.run("format.clear", json!({}));
        }
    });
    if ui.button(tl!("Manage Styles…")).clicked() {
        let _ = app.run("styles.manage", json!({}));
    }
    if ui.selectable_label(app.session.view.style_inspector, tl!("Style Inspector")).clicked() {
        let _ = app.run("styles.inspector", json!({}));
    }
    ui.separator();
    let mut list: Vec<(String, String, bool, bool)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| !s.hidden && s.kind != wordcraft_doc::StyleKind::Table)
        .map(|s| (s.id.clone(), crate::i18n::style_name(s), s.kind == wordcraft_doc::StyleKind::Character, s.builtin))
        .collect();
    list.sort_by_key(|s| s.1.to_lowercase());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, is_char, builtin) in list {
            ui.horizontal(|ui| {
                let r = ui.selectable_label(id == cur, &name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(if is_char { "a" } else { "¶" }).weak());
                });
                if r.clicked() {
                    let _ = app.run("para.style", json!({"style": id}));
                    app.canvas.want_focus = true;
                }
                r.context_menu(|ui| {
                    if ui.button(tl!("Modify…")).clicked() {
                        app.dialog = crate::dialogs::Dialog::modify_style(app, &id);
                        ui.close();
                    }
                    if ui.button(tl!("Update to Match Selection")).clicked() {
                        let _ = app.run("styles.updateToMatch", json!({"style": id}));
                        app.canvas.want_focus = true;
                        ui.close();
                    }
                    if ui.add_enabled(!builtin, egui::Button::new(tl!("Delete"))).clicked() {
                        let _ = app.run("styles.delete", json!({"style": id}));
                        app.canvas.want_focus = true;
                        ui.close();
                    }
                });
            });
        }
    });
}

fn comments(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Comments") {
        let _ = app.run("view.commentsPane", json!({"value": false}));
        return;
    }
    if crate::widgets::icon_text_button(ui, "newComment", tl!("New Comment"), regular(12.5)).clicked() {
        let _ = app.run("review.newComment", json!({"text": ""}));
    }
    ui.separator();
    let list = app.session.run("review.comments", &json!({})).unwrap_or_default();
    let t = Tokens::get(ui.ctx());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for c in list.as_array().cloned().unwrap_or_default() {
            let id = c.get("id").and_then(Value::as_u64).unwrap_or(0);
            let reply = c.get("parent").is_some_and(|p| !p.is_null());
            let when = u32::try_from(id).ok().and_then(|i| app.session.doc.comments.get(&i)).map(crate::local_time::comment_time).unwrap_or_default();
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if reply {
                    ui.add_space(0.0);
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c.get("author").and_then(Value::as_str).unwrap_or("")).font(semibold(12.0)));
                    ui.label(egui::RichText::new(when).small().weak());
                    if c.get("resolved").and_then(Value::as_bool).unwrap_or(false) {
                        ui.label(egui::RichText::new(tl!("Resolved")).small().color(t.green));
                    }
                });
                // Editable comment text (one undo step per edit, like the balloons).
                comment_editor(app, ui, id as u32, None, false);
                let open_key = egui::Id::new(("comment_reply_open", id));
                let mut replying = ui.data(|d| d.get_temp::<bool>(open_key)).unwrap_or(false);
                ui.horizontal(|ui| {
                    if ui.small_button(tl!("Go to")).clicked()
                        && let Some(a) = c.get("anchor").filter(|a| !a.is_null())
                    {
                        let _ = app.run("caret.set", json!({"pos": a}));
                    }
                    if ui.small_button(tl!("Reply")).clicked() {
                        replying = true;
                        ui.data_mut(|d| d.insert_temp(open_key, true));
                        ui.data_mut(|d| d.insert_temp(egui::Id::new(("comment_reply_focus", id)), true));
                    }
                    let resolved = c.get("resolved").and_then(Value::as_bool).unwrap_or(false);
                    if ui.small_button(if resolved { tl!("Reopen") } else { tl!("Resolve") }).clicked() {
                        let _ = app.run("review.resolveComment", json!({"id": id}));
                    }
                    if ui.small_button(tl!("Delete")).clicked() {
                        let _ = app.run("review.deleteComment", json!({"id": id}));
                    }
                });
                if replying {
                    let focus = ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(("comment_reply_focus", id)))).unwrap_or(false);
                    if reply_editor(app, ui, id as u32, None, focus) {
                        ui.data_mut(|d| d.remove::<bool>(open_key));
                    }
                }
            });
            ui.add_space(6.0);
        }
    });
}

/// What has been typed in comment and reply fields but not yet written to the document (#1348).
/// Drafts belong to one document ([`Session::document_id`](wordcraft_engine::Session::document_id)):
/// when another document replaces it they are dropped, so a comment there with the same id never
/// shows them. A draft is removed only once its command succeeds; a refused edit keeps it.
#[derive(Default)]
pub(crate) struct CommentDrafts {
    document: u64,
    edits: BTreeMap<u32, String>,
    replies: BTreeMap<u32, String>,
}

impl CommentDrafts {
    /// The drafts for `document`, dropping any left from another one.
    fn of(&mut self, document: u64) -> &mut Self {
        if self.document != document {
            self.edits.clear();
            self.replies.clear();
            self.document = document;
        }
        self
    }
}

/// The pending draft of comment `id`'s text in the open document, if any.
#[cfg(test)]
pub(crate) fn comment_draft(app: &WordApp, id: u32) -> Option<String> {
    let d = &app.comment_drafts;
    if d.document == app.session.document_id() { d.edits.get(&id).cloned() } else { None }
}

/// Whether the open document has a comment or reply draft not yet written to it (typed, or
/// kept after the document refused it): closing the page would lose it.
pub(crate) fn has_comment_drafts(app: &WordApp) -> bool {
    let d = &app.comment_drafts;
    d.document == app.session.document_id() && !(d.edits.is_empty() && d.replies.is_empty())
}

fn drafts(app: &mut WordApp) -> &mut CommentDrafts {
    let document = app.session.document_id();
    app.comment_drafts.of(document)
}

/// A comment's text, editable in place (the Comments pane and the balloons). Typing stays in a
/// draft until the field loses focus; then [`commit_comment`] writes it back with one
/// `review.editComment`, so each editing session is one undo step.
pub(crate) fn comment_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) {
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    let draft = drafts(app).edits.get(&id).cloned();
    let mut text = draft.unwrap_or_else(|| app.session.doc.plain_text(StoryRef::Part(part)));
    // Read-only where comments can't change (a protected document), so nothing is typed in vain.
    let editable = crate::widgets::enabled(app, "review.editComment");
    let mut edit = egui::TextEdit::multiline(&mut text)
        .id(field_id(app, ui, "comment_edit", id))
        .interactive(editable)
        .desired_rows(1)
        .desired_width(f32::INFINITY)
        .hint_text(tl!("Add a comment"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        drafts(app).edits.insert(id, text);
    }
    if r.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Escape throws the draft away: the field shows the comment's text again.
            drafts(app).edits.remove(&id);
        } else {
            // A refusal is on the status line, and the draft stays in the field.
            let _ = commit_comment(app, id);
        }
    }
}

/// A comment field's widget id, per document: egui's own state for the field (its cursor and
/// undo history) never carries over to another document's comment with the same id.
fn field_id(app: &WordApp, ui: &Ui, kind: &str, id: u32) -> egui::Id {
    ui.id().with((kind, app.session.document_id(), id))
}

/// Write a comment's draft text back, when it has one that differs from the comment. The draft
/// is dropped once written (or when there is nothing to write); a refused edit keeps it.
pub(crate) fn commit_comment(app: &mut WordApp, id: u32) -> Result<(), String> {
    let Some(text) = drafts(app).edits.get(&id).cloned() else { return Ok(()) };
    if let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part)
        && text != app.session.doc.plain_text(StoryRef::Part(part))
    {
        app.run("review.editComment", json!({"id": id, "text": text}))?;
    }
    drafts(app).edits.remove(&id);
    Ok(())
}

/// The comment a reply to `id` answers: replies to a reply join its thread.
fn thread_root(app: &WordApp, id: u32) -> u32 {
    match app.session.doc.comments.get(&id).and_then(|c| c.parent) {
        Some(p) if app.session.doc.comments.contains_key(&p) => p,
        _ => id,
    }
}

/// A field for replying to a comment. When it loses focus a non-empty reply is posted with
/// `review.reply`; returns true when the field is done (posted or left empty). A refused reply
/// keeps the field open with its text.
pub(crate) fn reply_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) -> bool {
    let mut text = drafts(app).replies.get(&id).cloned().unwrap_or_default();
    let editable = crate::widgets::enabled(app, "review.reply");
    let mut edit = egui::TextEdit::multiline(&mut text)
        .id(field_id(app, ui, "comment_reply", id))
        .interactive(editable)
        .desired_rows(1)
        .desired_width(f32::INFINITY)
        .hint_text(tl!("Write a reply"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        drafts(app).replies.insert(id, text);
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        // Escape throws the reply away and closes the field.
        drafts(app).replies.remove(&id);
        return true;
    }
    r.lost_focus() && commit_reply(app, id).is_ok()
}

/// Post a pending reply draft to comment `id` (nothing when there is none or it is blank). The
/// draft is dropped once posted; a refused reply keeps it.
pub(crate) fn commit_reply(app: &mut WordApp, id: u32) -> Result<(), String> {
    let Some(text) = drafts(app).replies.get(&id).cloned() else { return Ok(()) };
    if !text.trim().is_empty() && app.session.doc.comments.contains_key(&id) {
        let root = thread_root(app, id);
        app.run("review.reply", json!({"id": root, "text": text.trim_end()}))?;
    }
    drafts(app).replies.remove(&id);
    Ok(())
}

/// Write every pending comment and reply draft to the document, so a save has them and a close,
/// New or Open sees them as unsaved changes (#1348). Every draft is tried; the first refusal is
/// returned (and is on the status line), and refused drafts stay.
pub(crate) fn flush_comment_drafts(app: &mut WordApp) -> Result<(), String> {
    let (edits, replies): (Vec<u32>, Vec<u32>) = {
        let d = drafts(app);
        (d.edits.keys().copied().collect(), d.replies.keys().copied().collect())
    };
    let mut first = Ok(());
    for id in edits {
        let r = commit_comment(app, id);
        if first.is_ok() {
            first = r;
        }
    }
    for id in replies {
        let r = commit_reply(app, id);
        if first.is_ok() {
            first = r;
        }
    }
    first
}

/// Style Inspector: the paragraph and character levels at the caret, each with its style and the
/// direct formatting on top, and a button to clear each level.
fn inspector(app: &mut WordApp, ui: &mut Ui) {
    if header(ui, "Style Inspector") {
        let _ = app.run("styles.inspector", json!({"value": false}));
        return;
    }
    // Read-only, so call it directly: running the command every frame would pay for the command
    // machinery and consume per-command state (a pending `join_next_undo`) just to draw the pane.
    let r = wordcraft_engine::cmd::inspector::inspect(&app.session);
    let t = Tokens::get(ui.ctx());
    let none = tl!("None").to_string();
    let para = &r["paragraph"];
    let chr = &r["character"];
    let pstyle = para["styleName"].as_str().unwrap_or("Normal").to_string();
    let cstyle = chr["styleName"].as_str().map(str::to_string);
    // Built-in style names show in the interface language; the document keeps the English ones.
    let shown = |id: &Value, name: &str| {
        id.as_str().and_then(|id| app.session.doc.styles.get(id)).map(crate::i18n::style_name).unwrap_or_else(|| name.to_string())
    };
    let pstyle_shown = shown(&para["style"], &pstyle);
    let cstyle_shown = cstyle.as_deref().map(|n| shown(&chr["style"], n));
    let pdirect = direct_list(&para["direct"]);
    let cdirect = direct_list(&chr["direct"]);
    let mut clear: Option<&str> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        section(ui, &t, "Paragraph", |ui| {
            if level_row(ui, &t, "Style", &pstyle_shown, false, "Reset to Normal", pstyle == "Normal" && para["style"] == "Normal") {
                clear = Some("paragraphStyle");
            }
            let (txt, empty) = if pdirect.is_empty() { (none.clone(), true) } else { (pdirect.join(", "), false) };
            if level_row(ui, &t, "Direct formatting", &txt, empty, "Clear Paragraph Formatting", empty) {
                clear = Some("paragraphFormatting");
            }
        });
        ui.add_space(8.0);
        section(ui, &t, "Characters", |ui| {
            let (txt, empty) = match &cstyle_shown {
                Some(n) => (n.clone(), false),
                None => (none.clone(), true),
            };
            if level_row(ui, &t, "Style", &txt, empty, "Clear Character Style", empty) {
                clear = Some("characterStyle");
            }
            let (txt, empty) = if cdirect.is_empty() { (none.clone(), true) } else { (cdirect.join(", "), false) };
            if level_row(ui, &t, "Direct formatting", &txt, empty, "Clear Character Formatting", empty) {
                clear = Some("characterFormatting");
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Clear All")).on_hover_text(tl!("Clear All Formatting")).clicked() {
                let _ = app.run("format.clear", json!({}));
                app.canvas.want_focus = true;
            }
            if ui.button(tl!("Styles Pane")).clicked() {
                let _ = app.run("view.stylesPane", json!({"value": true}));
            }
        });
    });
    if let Some(level) = clear {
        let _ = app.run("styles.inspectorClear", json!({"level": level}));
        app.canvas.want_focus = true;
    }
}

/// A titled, bordered group in the inspector.
fn section(ui: &mut Ui, t: &Tokens, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(egui::RichText::new(tl!(title)).font(semibold(12.5)).color(t.text));
    ui.add_space(2.0);
    egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(4).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
}

/// One level: a caption, its value, and a clear button (disabled when there's nothing to clear).
/// Returns true when the button was clicked.
fn level_row(ui: &mut Ui, t: &Tokens, caption: &str, value: &str, dim: bool, tip: &str, disabled: bool) -> bool {
    ui.label(egui::RichText::new(tl!(caption)).font(regular(11.0)).color(t.text_dim));
    let mut clicked = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 26.0).max(40.0);
        ui.allocate_ui(vec2(w, 18.0), |ui| {
            ui.set_width(w);
            let text = egui::RichText::new(value).font(regular(12.5)).color(if dim { t.text_disabled } else { t.text });
            ui.add(egui::Label::new(text).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), egui::Sense::click());
            let enabled = !disabled;
            if enabled && resp.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            let c = if enabled { t.icon } else { t.text_disabled };
            crate::icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                "clear",
                c,
                if enabled { t.accent } else { c },
            );
            let resp = resp.on_hover_text(tl!(tip));
            clicked = enabled && resp.clicked();
        });
    });
    ui.add_space(4.0);
    clicked
}

/// Readable descriptions of the inspector's `direct` entries (`[{"prop", "value"}]`).
fn direct_list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().take(64).map(|d| describe(d["prop"].as_str().unwrap_or(""), &d["value"])).collect()).unwrap_or_default()
}

/// Each `{"prop", "value"}` of a list (the Style Inspector's, or a formatting change's) as text.
pub(crate) fn describe_props(list: &[Value]) -> Vec<String> {
    list.iter().take(64).map(|d| describe(d["prop"].as_str().unwrap_or(""), &d["value"])).collect()
}

/// "Bold", "Not Italic", "Font: Arial", "Size: 14 pt"…
fn describe(prop: &str, v: &Value) -> String {
    let label = match prop {
        "font" => "Font",
        "size" => "Size",
        "bold" => "Bold",
        "italic" => "Italic",
        "underline" => "Underline",
        "underlineColor" => "Underline color",
        "strike" => "Strikethrough",
        "doubleStrike" => "Double strikethrough",
        "color" => "Font color",
        "highlight" => "Highlight",
        "shading" => "Shading",
        "vertAlign" => "Position",
        "caps" => "All caps",
        "smallCaps" => "Small caps",
        "hidden" => "Hidden",
        "spacing" => "Character spacing",
        "scale" => "Scale",
        "position" => "Raised/lowered",
        "outline" => "Outline",
        "shadow" => "Shadow",
        "emboss" => "Emboss",
        "engrave" => "Engrave",
        "align" => "Alignment",
        "indentLeft" => "Left indent",
        "indentRight" => "Right indent",
        "indentFirst" => "First line",
        "spaceBefore" => "Space before",
        "spaceAfter" => "Space after",
        "lineSpacing" => "Line spacing",
        "contextualSpacing" => "Don't add space between paragraphs of the same style",
        "mirrorIndents" => "Mirror indents",
        "keepNext" => "Keep with next",
        "keepLines" => "Keep lines together",
        "pageBreakBefore" => "Page break before",
        "widowControl" => "Widow/Orphan control",
        "outlineLevel" => "Outline level",
        "tabs" => "Tabs",
        "borders" => "Borders",
        "suppressHyphens" => "Don't hyphenate",
        "suppressLineNumbers" => "Suppress line numbers",
        "bidi" => "Right-to-left",
        "dropCap" => "Drop cap",
        "dropCapMargin" => "In margin",
        "dropCapDistance" => "Distance from text",
        "dropCapFont" => "Font",
        "style" => "Style",
        "numbering" => "Numbering",
        "lang" => "Language",
        other => other,
    };
    let label = tl!(label);
    let pt = |x: f64| crate::i18n::fmt(tl!("{n} pt"), &[("n", &trim_num(x))]);
    let hex = |c: &Value| -> Option<String> {
        let a = c.as_array()?;
        let ch = |i: usize| a.get(i).and_then(Value::as_u64).unwrap_or(0).min(255);
        Some(format!("#{:02X}{:02X}{:02X}", ch(0), ch(1), ch(2)))
    };
    let value = match (prop, v) {
        (_, Value::Bool(true)) => return label.to_string(),
        (_, Value::Bool(false)) => return crate::i18n::fmt(tl!("Not {name}"), &[("name", label)]),
        ("scale", Value::Number(n)) => format!("{}%", trim_num(n.as_f64().unwrap_or(100.0))),
        ("outlineLevel", Value::Number(n)) => match n.as_u64().unwrap_or(9) {
            l @ 0..=8 => crate::i18n::fmt(tl!("Level {n}"), &[("n", &(l + 1).to_string())]),
            _ => tl!("Body Text").to_string(),
        },
        ("dropCapFont", Value::String(s)) => s.clone(),
        ("dropCap", Value::Number(n)) => crate::i18n::fmt(tl!("{n} lines"), &[("n", &n.to_string())]),
        (_, Value::Number(n)) => pt(n.as_f64().unwrap_or(0.0)),
        ("align", Value::String(s)) => tl!(match s.as_str() {
            "center" => "Centered",
            "right" => "Right",
            "justify" => "Justified",
            "distribute" => "Distributed",
            _ => "Left",
        })
        .to_string(),
        ("color", Value::String(_)) => tl!("Automatic").to_string(),
        ("color", Value::Object(o)) => o.get("Rgb").and_then(hex).unwrap_or_default(),
        (_, Value::Array(_)) if prop != "tabs" => hex(v).unwrap_or_default(),
        ("lineSpacing", Value::Object(o)) => {
            let x = o.get("value").and_then(Value::as_f64).unwrap_or(1.0);
            match o.get("rule").and_then(Value::as_str) {
                Some("atLeast") => crate::i18n::fmt(tl!("At least {n}"), &[("n", &pt(x))]),
                Some("exactly") => crate::i18n::fmt(tl!("Exactly {n}"), &[("n", &pt(x))]),
                _ => crate::i18n::fmt(tl!("{n} lines"), &[("n", &trim_num(x))]),
            }
        }
        (_, Value::String(s)) => tl!(&camel_words(s)).to_string(),
        _ => return label.to_string(),
    };
    format!("{label}: {value}")
}

/// `1.50` → `1.5`, `12.0` → `12`.
fn trim_num(x: f64) -> String {
    let s = format!("{:.2}", if x.is_finite() { x } else { 0.0 });
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `dotDash` → `Dot dash` (enum values from the document model).
fn camel_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().take(64).enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// #1902: Clipboard Delete returns focus to the document like paste does.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn clipboard_delete_returns_focus_to_the_document() {
        use egui_kittest::kittest::Queryable;

        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
        app.run("select.all", serde_json::json!({})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        assert_eq!(app.session.clip_history.items().len(), 1);
        app.run("edit.clipboardPane", serde_json::json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        // Focus the button for real: a bare accesskit click leaves egui focus empty, and an
        // unrelated chrome-click helper would hand focus to the canvas on its own.
        h.get_by_label("Delete").focus();
        h.run_steps(2);
        h.state_mut().canvas.want_focus = false;
        h.get_by_label("Delete").click();
        h.run_steps(4);
        assert!(h.state().canvas.focused, "Delete returns focus to the document");
        assert!(h.state().session.clip_history.items().is_empty(), "the item was deleted");
    }

    /// #3032: Delete is greyed out for built-in styles in the Styles pane (the engine refuses
    /// them — `crates/engine/src/cmd/para.rs`), while custom styles keep it enabled.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn built_in_styles_grey_out_delete_in_the_styles_pane() {
        use egui_kittest::kittest::{NodeT, Queryable};

        /// Draw only the pane, so each style name labels exactly one node — the ribbon's Styles
        /// gallery shows the same names — right-click the row, and report whether its Delete is
        /// greyed out.
        fn delete_is_disabled(style: &str) -> bool {
            let mut app =
                crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());
            app.run("styles.create", serde_json::json!({"name": "Callout"})).unwrap();
            // The pane writes in the "semibold" family, which the app installs with the rest of the
            // interface fonts (`chrome::title_bar`). Fonts take effect the frame after they are set,
            // so the first frame only installs them and the pane is drawn from the second on.
            let mut fonts = false;
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
                |ui, app: &mut crate::WordApp| {
                    if !fonts {
                        crate::theme::install_fonts(ui.ctx());
                        fonts = true;
                        return;
                    }
                    super::styles(app, ui);
                },
                app,
            );
            h.run_steps(4);
            h.get_by_label(style).click_secondary();
            h.run_steps(3);
            h.get_by_label("Delete").accesskit_node().is_disabled()
        }
        assert!(delete_is_disabled("Normal"), "the built-in style's Delete is greyed out");
        assert!(!delete_is_disabled("Callout"), "the custom style's Delete stays enabled");
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    /// Numbered headings are listed with the number they show, and numbered body paragraphs in
    /// the same list still count (#3418).
    #[test]
    fn headings_show_their_list_numbers() {
        use wordcraft_doc::props::NumRef;
        use wordcraft_doc::{CharProps, ListKind, Paragraph, para_block};
        let mut doc = Document::new();
        let num = doc.numbering.add_list(ListKind::Legal);
        let para = |text: &str, style: &str, level: Option<u8>| {
            let mut p = Paragraph::with_text(text, CharProps::default()).styled(style);
            p.props.numbering = level.map(|level| NumRef { num, level });
            para_block(p)
        };
        doc.body = vec![
            para("Intro", "Heading1", Some(0)),
            para("Scope", "Heading2", Some(1)),
            para("A numbered body paragraph", "Normal", Some(1)),
            para("Terms", "Heading2", Some(1)),
            para("Method", "Heading1", Some(0)),
            para("Appendix", "Heading1", None),
        ];
        let texts: Vec<String> = nav_headings(&doc).into_iter().map(|(_, _, t)| t).collect();
        assert_eq!(texts, ["1. Intro", "1.1. Scope", "1.3. Terms", "2. Method", "Appendix"]);
    }

    #[test]
    fn long_headings_wrap_without_stretching_navigation() {
        // A heading that runs well past the width, and a single word wider than the pane.
        let headings = ["An extremely long heading with words that must stay inside the navigation pane ".repeat(20), "x".repeat(400)];
        // Draw one heading in a pane of `width`: the pane rect, the heading row's rect, the label's
        // line count and whether it was elided.
        let render = |width: f32, level: u8, text: &str| -> (egui::Rect, egui::Rect, usize, bool) {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_rect = egui::Rect::NOTHING;
            let mut heading_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test")
                    .default_size(width)
                    .resizable(true)
                    .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| nav_heading(ui, level, text, true, None).1).inner);
                panel_rect = result.response.rect;
                heading_rect = result.inner.rect;
            });
            let (rows, elided) = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some((t.galley.rows.len(), t.galley.elided)),
                    _ => None,
                })
                .unwrap_or((0, false));
            output.drop_without_applying_deltas();
            (panel_rect, heading_rect, rows, elided)
        };
        for (width, level) in [(260.0, 0), (180.0, 5)] {
            let (_, one_line, _, _) = render(width, level, "Kilns");
            for text in &headings {
                let (panel_rect, heading_rect, rows, elided) = render(width, level, text);
                assert!(panel_rect.width() < width + 20.0, "long heading expanded the {width}px pane to {panel_rect:?}");
                assert!(heading_rect.max.x <= panel_rect.max.x, "heading escaped pane: {heading_rect:?} / {panel_rect:?}");
                // The heading wraps onto further lines instead of ending in an ellipsis (#3015).
                assert!(rows > 1, "long heading must wrap onto further lines at {width}px, got {rows}");
                assert!(!elided, "long heading must not end in an ellipsis at {width}px");
                // ... and its row grows with the text, where a short heading keeps one row.
                assert!(heading_rect.height() > one_line.height(), "a wrapped heading row is taller at {width}px: {heading_rect:?} vs {one_line:?}");
            }
        }
    }

    /// Word lists headings flush left, each level indented under its parent (#1199).
    #[test]
    fn headings_align_left_and_indent_by_level() {
        let text_left = |level: u8| {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_left = 0.0;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test").default_size(260.0).show(ui, |ui| nav_heading(ui, level, "Kilns", false, None));
                panel_left = result.response.rect.min.x;
            });
            let x = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Kilns" => Some(t.pos.x + t.galley.rect.min.x),
                    _ => None,
                })
                .unwrap();
            output.drop_without_applying_deltas();
            x - panel_left
        };
        let (top, second) = (text_left(0), text_left(1));
        assert!(top < 40.0, "a top-level heading starts at the pane's left edge, not centred: {top}px in");
        assert!((second - top - 14.0).abs() < 0.5, "a level-2 heading is indented 14px under its parent: {top} / {second}");
    }

    /// Collapsing a heading hides everything nested under it, and only that (#1997).
    #[test]
    fn collapsed_heading_hides_its_descendants_only() {
        // 1 / 1.1 / 1.1.1 / 1.2 / 1.2.1 / 2 / 2.1
        let levels = [0, 1, 2, 1, 2, 0, 1];
        let none = [false; 7];
        assert_eq!(visible_headings(&levels, &none), vec![true; 7]);
        let mut c = none;
        c[3] = true; // collapse 1.2
        assert_eq!(visible_headings(&levels, &c), vec![true, true, true, true, false, true, true]);
        c[3] = false;
        c[0] = true; // collapse 1
        assert_eq!(visible_headings(&levels, &c), vec![true, false, false, false, false, true, true]);
        // A collapsed child inside a collapsed parent stays hidden and does not reopen siblings.
        c[1] = true;
        assert_eq!(visible_headings(&levels, &c), vec![true, false, false, false, false, true, true]);
    }
}
