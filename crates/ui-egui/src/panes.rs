//! Side panes: Navigation (headings, pages, search results), Clipboard, Styles, Style Inspector,
//! Accessibility and Alt Text (see `a11y_pane`), Restrict Editing ([`crate::dialogs_review`]), Comments,
//! and the SmartArt Text Pane.

use egui::{Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::{Path, Pos, StoryRef};

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
                job.append(
                    hit,
                    0.0,
                    egui::TextFormat { font_id: semibold(12.0), background: mark, color: ui.visuals().text_color(), ..Default::default() },
                );
                job.append(post, 0.0, fmt(ui.visuals().text_color()));
                job.wrap.max_width = ui.available_width();
                if ui.add(egui::Button::new(job).frame(false)).clicked() {
                    let _ = app.run("select.range", json!({"anchor": a, "focus": b}));
                }
                ui.separator();
            }
        }
        _ => outline(app, ui),
    });
}

/// What the outline's context menu can do. Everything but the table of contents works on a whole
/// section at a time — the heading and the paragraphs under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Promote,
    Demote,
    Remove,
    Hide,
    Above,
    Below,
    Child,
    Toc,
}

impl Edit {
    /// The menu, in the order it shows the commands: the heading's own level, then what happens to
    /// its section, then the headings added around it, and last the table of contents the outline
    /// is for. [`Edit::starts_group`] rules those four groups off from one another.
    const ALL: [Edit; 8] = [Edit::Promote, Edit::Demote, Edit::Remove, Edit::Hide, Edit::Above, Edit::Below, Edit::Child, Edit::Toc];

    /// The command that carries it out. The menu draws each command's own label, so what a search
    /// finds by name is what the menu says (and every label is translated once, in the catalog).
    fn command(self) -> &'static str {
        match self {
            Edit::Promote => "nav.promote",
            Edit::Demote => "nav.demote",
            Edit::Remove => "nav.remove",
            Edit::Hide => "nav.hideFromToc",
            Edit::Above => "nav.addAbove",
            Edit::Below => "nav.addBelow",
            Edit::Child => "nav.addSubheading",
            // The References tab's own command, which inserts a three-level table at the caret (or
            // refreshes the one already there) — the outline is what such a table is built from.
            Edit::Toc => "references.toc",
        }
    }

    /// Whether a rule is drawn above it, to set the groups apart.
    fn starts_group(self) -> bool {
        matches!(self, Edit::Remove | Edit::Above | Edit::Toc)
    }

    /// What it runs with: every command works on the heading's path except the table, which goes
    /// where the caret is like the References tab's own.
    fn params(self, path: &Path) -> Value {
        if self == Edit::Toc { json!({"levels": 3}) } else { json!({"path": path.0}) }
    }

    /// Whether the page should follow the caret the command leaves behind: adding a heading puts it
    /// in the new one so it can be typed straight away, and deleting one leaves it where the
    /// section was. Promoting, demoting and hiding leave the text where it is.
    fn moves_caret(self) -> bool {
        matches!(self, Edit::Remove | Edit::Above | Edit::Below | Edit::Child)
    }
}

/// The Headings tab: the outline as a tree that can be worked on rather than only read. A row
/// jumps to its paragraph, the triangle (or a double-click) folds the section under a heading,
/// the context menu promotes, demotes, adds to and takes out whole sections, and a row can be
/// dragged above or below another one to move its section there. Word's pane navigates; this is
/// the editable outline of an online editor (Tencent Docs, WPS), on the same tree.
fn outline(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let caret = app.session.sel.focus.clone();
    // (paragraph, level 1–9, title) for every heading of the body, TOC entries excluded.
    let mut items: Vec<(Path, u8, String)> = Vec::new();
    let mut current: Option<usize> = None;
    for path in app.session.doc.para_paths(StoryRef::Body) {
        let Some(p) = app.session.doc.para(StoryRef::Body, &path) else { continue };
        let rp = app.session.doc.styles.resolve_para(&p.props);
        if rp.style.starts_with("TOC") {
            continue;
        }
        let Some(lv) = rp.outline_level.filter(|l| *l < 9) else { continue };
        if path <= caret.path {
            current = Some(items.len());
        }
        items.push((path, lv + 1, p.plain_text().trim().to_string()));
    }
    if items.is_empty() {
        ui.label(egui::RichText::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.")).weak());
        return;
    }
    // The rows to draw: everything not tucked away under a folded heading.
    let mut shown: Vec<usize> = Vec::new();
    let mut folded: Option<u8> = None;
    for (i, (path, level, _)) in items.iter().enumerate() {
        if let Some(open) = folded {
            if *level > open {
                continue;
            }
            folded = None;
        }
        shown.push(i);
        if has_children(&items, i) && app.ui.nav_collapsed.contains(&path.0) {
            folded = Some(*level);
        }
    }
    ui.spacing_mut().item_spacing.y = 1.0;
    // The menu's wording, read off the commands themselves so the pane and a command search never
    // drift apart.
    let menu: Vec<&'static str> = Edit::ALL.iter().map(|e| app.session.registry.get(e.command()).map_or("", |c| c.label)).collect();
    let mut rows: Vec<(usize, Rect)> = Vec::new();
    let mut edits: Vec<(Edit, Path)> = Vec::new();
    let mut dropped = false;
    for &i in &shown {
        let path = items[i].0.clone();
        let level = items[i].1;
        let text = items[i].2.clone();
        let kids = has_children(&items, i);
        let empty = text.is_empty();
        // An empty heading still gets a row — it is what Add Heading Above/Below/Subheading makes —
        // so it reads as its style instead of a blank line.
        let title = if empty { crate::i18n::fmt(tl!("Heading {n}"), &[("n", &level.to_string())]) } else { text };
        let open = !app.ui.nav_collapsed.contains(&path.0);
        let resp = outline_row(
            ui,
            &t,
            Row {
                level,
                title: &title,
                dim: empty,
                selected: current == Some(i),
                dragging: app.ui.nav_drag.as_deref() == Some(path.0.as_slice()),
                kids,
                open,
            },
        );
        rows.push((i, resp.rect));
        // The triangle folds; anywhere else the row goes to the heading, as the pane always did.
        let on_triangle = resp.clicked()
            && ui.input(|i| i.pointer.interact_pos().is_some_and(|p| p.x < resp.rect.min.x + 4.0 + level.saturating_sub(1) as f32 * 14.0 + 12.0));
        if kids && (on_triangle || resp.double_clicked()) {
            if open {
                app.ui.nav_collapsed.push(path.0.clone());
            } else {
                app.ui.nav_collapsed.retain(|k| *k != path.0);
            }
        } else if resp.clicked() {
            let _ = app.run("caret.set", json!({"pos": Pos { story: StoryRef::Body, path: path.clone(), off: 0 }}));
            // Align the chosen heading's page immediately against this pane, rather
            // than the indeterminate minimum movement from scroll_to_rect.
            app.canvas.nav_to_caret = true;
            app.canvas.scroll_to_caret = false;
            app.canvas.want_focus = true;
        }
        resp.context_menu(|ui| {
            for (n, edit) in Edit::ALL.iter().enumerate() {
                if n > 0 && edit.starts_group() {
                    ui.separator();
                }
                if ui.button(tl!(menu[n])).clicked() {
                    edits.push((*edit, path.clone()));
                    ui.close();
                }
            }
        });
        if resp.drag_started() {
            app.ui.nav_drag = Some(path.0.clone());
        }
        if resp.drag_stopped() {
            dropped = true;
        }
    }
    // Where the dragged section would land, drawn as a line while it is still in the air.
    let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
    let landing = app.ui.nav_drag.as_ref().and(pointer).and_then(|p| landing(&items, &rows, p));
    if let Some((_, y)) = &landing
        && let (Some((_, r0)), Some((_, r1))) = (rows.first(), rows.last())
    {
        ui.painter().hline(r0.min.x..=r1.max.x, *y, Stroke::new(2.0, t.accent));
    }
    if dropped && let (Some(from), Some((at, _))) = (app.ui.nav_drag.take(), landing) {
        let _ = app.run("nav.move", json!({"path": from, "before": at}));
    }
    for (edit, path) in edits {
        let _ = app.run(edit.command(), edit.params(&path));
        if edit.moves_caret() {
            app.canvas.nav_to_caret = true;
            app.canvas.scroll_to_caret = false;
            app.canvas.want_focus = true;
        }
    }
}

/// The height of one outline row.
const ROW_H: f32 = 20.0;

/// One row of the outline, as the pane draws it.
struct Row<'a> {
    /// Heading level, 1–9: the title is indented 14 pt under its parent per level.
    level: u8,
    /// What the row reads: the heading's text, or its style's name while it is still empty.
    title: &'a str,
    /// An empty heading is drawn dimmed, so it reads as a place to type rather than a blank line.
    dim: bool,
    /// The caret is in this heading's section.
    selected: bool,
    /// This is the row being dragged.
    dragging: bool,
    /// A section follows it, so the row gets a fold triangle.
    kids: bool,
    /// That section is open (the triangle points down).
    open: bool,
}

/// Draw one heading row: the fold triangle, the title indented under its parent, and the
/// background marking the section the caret is in. Long titles elide inside the pane instead of
/// stretching it (#1199), and the whole title is on hover. What a click, a drag or a right-click
/// does with the row belongs to the pane, which knows the document.
fn outline_row(ui: &mut Ui, t: &Tokens, row: Row<'_>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), egui::Sense::click_and_drag());
    if row.selected || row.dragging || resp.hovered() {
        let fill = if row.selected { t.selection } else { t.hover };
        ui.painter().rect_filled(rect, 3.0, fill);
    }
    let indent = 4.0 + row.level.saturating_sub(1) as f32 * 14.0;
    if row.kids {
        let c = pos2(rect.min.x + indent + 4.0, rect.center().y);
        let d = 3.6;
        let points = if row.open {
            vec![pos2(c.x - d, c.y - d * 0.6), pos2(c.x + d, c.y - d * 0.6), pos2(c.x, c.y + d * 0.8)]
        } else {
            vec![pos2(c.x - d * 0.6, c.y - d), pos2(c.x - d * 0.6, c.y + d), pos2(c.x + d * 0.8, c.y)]
        };
        ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, Stroke::NONE));
    }
    let color = if row.dim { t.text_disabled } else { t.text };
    let text_x = rect.min.x + indent + 13.0;
    let mut job = egui::text::LayoutJob::single_section(
        row.title.to_string(),
        egui::TextFormat { font_id: if row.level == 1 { semibold(12.5) } else { regular(12.5) }, color, ..Default::default() },
    );
    // One line, elided: a long heading stays inside the pane (#1199).
    job.wrap.max_width = (rect.max.x - text_x - 2.0).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(text_x, rect.center().y - galley.size().y / 2.0), galley, color);
    resp.clone().on_hover_text(row.title)
}

/// Whether a heading opens a section: another heading deeper than it follows before one at its
/// level or above.
fn has_children(items: &[(Path, u8, String)], i: usize) -> bool {
    items.get(i + 1).is_some_and(|(_, l, _)| *l > items[i].1)
}

/// Where a drop at `pos` would put the dragged section: the paragraph it goes in front of, as the
/// path `nav.move` wants, and the height the drop line is drawn at. Dropping on a row's upper half
/// lands before that heading; the lower half lands after its whole section, in front of the next
/// heading at its level or above. `None` (no line) = the end of the container it is in.
fn landing(items: &[(Path, u8, String)], rows: &[(usize, Rect)], pos: Pos2) -> Option<(serde_json::Value, f32)> {
    let (top, bottom) = (rows.first()?.1, rows.last()?.1);
    if pos.y < top.min.y {
        return rows.first().map(|(i, r)| (json!(items[*i].0.0), r.min.y));
    }
    if pos.y > bottom.max.y {
        return None;
    }
    let (i, rect) = *rows.iter().find(|(_, r)| r.contains(pos))?;
    if pos.y <= rect.center().y {
        return Some((json!(items[i].0.0), rect.min.y));
    }
    // Past this heading's section: the first heading that is no longer under it, and the line at
    // that row — or, when it is the last section, at the bottom of the last row it covers.
    let level = items[i].1;
    let next = items.iter().enumerate().skip(i + 1).find(|(_, (_, l, _))| *l <= level);
    match next {
        Some((j, (p, _, _))) if p.parent() == items[i].0.parent() => {
            let y = rows
                .iter()
                .find(|(k, _)| *k == j)
                .map_or_else(|| rows.iter().filter(|(k, _)| *k >= i).map(|(_, r)| r.max.y).next_back().unwrap_or(rect.max.y), |(_, r)| r.min.y);
            Some((json!(p.0), y))
        }
        _ => None,
    }
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
    let mut list: Vec<(String, String, bool)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| !s.hidden && s.kind != wordcraft_doc::StyleKind::Table)
        .map(|s| (s.id.clone(), crate::i18n::style_name(s), s.kind == wordcraft_doc::StyleKind::Character))
        .collect();
    list.sort_by_key(|s| s.1.to_lowercase());
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, is_char) in list {
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
                        ui.close();
                    }
                    if ui.button(tl!("Delete")).clicked() {
                        let _ = app.run("styles.delete", json!({"style": id}));
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
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if reply {
                    ui.add_space(0.0);
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c.get("author").and_then(Value::as_str).unwrap_or("")).font(semibold(12.0)));
                    ui.label(egui::RichText::new(comment_stamp(c.get("date").and_then(Value::as_str).unwrap_or(""))).small().weak());
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

fn edit_key(id: u32) -> egui::Id {
    egui::Id::new(("comment_edit", id))
}

fn reply_key(id: u32) -> egui::Id {
    egui::Id::new(("comment_reply", id))
}

/// A comment's text, editable in place (the Comments pane and the balloons). Typing stays in a
/// draft until the field loses focus; then [`commit_comment`] writes it back with one
/// `review.editComment`, so each editing session is one undo step.
pub(crate) fn comment_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) {
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    let key = edit_key(id);
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| app.session.doc.plain_text(StoryRef::Part(part)));
    let mut edit = egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY).hint_text(tl!("Add a comment"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    if r.lost_focus() {
        commit_comment(app, ui.ctx(), id);
    }
}

/// Write a comment's draft text back, when it has one that differs from the comment.
pub(crate) fn commit_comment(app: &mut WordApp, ctx: &egui::Context, id: u32) {
    let Some(text) = ctx.data_mut(|d| d.remove_temp::<String>(edit_key(id))) else { return };
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    if text != app.session.doc.plain_text(StoryRef::Part(part)) {
        let _ = app.run("review.editComment", json!({"id": id, "text": text}));
    }
}

/// The comment a reply to `id` answers: replies to a reply join its thread.
fn thread_root(app: &WordApp, id: u32) -> u32 {
    match app.session.doc.comments.get(&id).and_then(|c| c.parent) {
        Some(p) if app.session.doc.comments.contains_key(&p) => p,
        _ => id,
    }
}

/// A field for replying to a comment. When it loses focus a non-empty reply is posted with
/// `review.reply`; returns true when the field is done (posted or left empty).
pub(crate) fn reply_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) -> bool {
    let key = reply_key(id);
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_default();
    let mut edit = egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY).hint_text(tl!("Write a reply"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    if r.lost_focus() {
        commit_reply(app, ui.ctx(), id);
        return true;
    }
    false
}

/// Post a pending reply draft to comment `id` (nothing when there is none or it is blank).
pub(crate) fn commit_reply(app: &mut WordApp, ctx: &egui::Context, id: u32) {
    let Some(text) = ctx.data_mut(|d| d.remove_temp::<String>(reply_key(id))) else { return };
    if !text.trim().is_empty() {
        let root = thread_root(app, id);
        let _ = app.run("review.reply", json!({"id": root, "text": text.trim_end()}));
    }
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

/// `2026-10-12T04:44:00Z` → `2026-10-12 04:44` (date and time, no seconds) for comment cards.
fn comment_stamp(iso: &str) -> String {
    iso.chars().take(16).map(|c| if c == 'T' { ' ' } else { c }).collect()
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
    use super::comment_stamp;

    #[test]
    fn comment_stamp_keeps_date_and_time() {
        assert_eq!(comment_stamp("2026-10-12T04:44:31Z"), "2026-10-12 04:44");
        assert_eq!(comment_stamp("2026-10-12"), "2026-10-12");
        assert_eq!(comment_stamp(""), "");
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn long_headings_truncate_without_stretching_navigation() {
        let text = "An extremely long heading with words that must stay inside the navigation pane ".repeat(20);
        // Level 1 and a deep level: indentation must not eat the width either way.
        for (width, level) in [(260.0, 1), (180.0, 6)] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_rect = egui::Rect::NOTHING;
            let mut heading_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test").default_size(width).resizable(true).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| {
                            let t = Tokens::get(ui.ctx());
                            outline_row(ui, &t, Row { level, title: &text, dim: false, selected: true, dragging: false, kids: false, open: true })
                        })
                        .inner
                });
                panel_rect = result.response.rect;
                heading_rect = result.inner.rect;
            });

            assert!(panel_rect.width() < width + 20.0, "long heading expanded the {width}px pane to {panel_rect:?}");
            assert!(heading_rect.max.x <= panel_rect.max.x, "heading escaped pane: {heading_rect:?} / {panel_rect:?}");
            assert!(
                output.shapes.iter().any(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        t.galley.elided && t.galley.rows.last().is_some_and(|r| r.row.glyphs.last().is_some_and(|g| g.chr == '…'))
                    }
                    _ => false,
                }),
                "long heading must render a trailing ellipsis at {width}px"
            );
            output.drop_without_applying_deltas();
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
                let result = egui::Panel::left("navigation_test").default_size(260.0).show(ui, |ui| {
                    let t = Tokens::get(ui.ctx());
                    outline_row(ui, &t, Row { level, title: "Kilns", dim: false, selected: false, dragging: false, kids: false, open: true })
                });
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
        let (top, second) = (text_left(1), text_left(2));
        assert!(top < 30.0, "a top-level heading starts at the pane's left edge, not centred: {top}px in");
        assert!((second - top - 14.0).abs() < 0.5, "a level-2 heading is indented 14px under its parent: {top} / {second}");
    }

    /// The rows of the tree below, one per heading, stacked as the pane lays them out.
    fn tree() -> (Vec<(Path, u8, String)>, Vec<(usize, Rect)>) {
        let items: Vec<(Path, u8, String)> =
            [(0, 1u8, "One"), (1, 2, "One A"), (2, 1, "Two")].into_iter().map(|(i, l, t)| (Path::top(i), l, t.to_string())).collect();
        let rows = (0..3).map(|i| (i, Rect::from_min_size(pos2(0.0, i as f32 * ROW_H), vec2(200.0, ROW_H)))).collect();
        (items, rows)
    }

    /// A section is what moves, so a row that opens one has children and a row that closes one does
    /// not (#1199's tree).
    #[test]
    fn a_row_has_children_when_a_deeper_heading_follows() {
        let (items, _) = tree();
        assert!(has_children(&items, 0), "One opens a section");
        assert!(!has_children(&items, 1), "One A is a leaf");
        assert!(!has_children(&items, 2), "Two is the last heading");
    }

    /// A drop lands where the pointer says: the upper half of a row is in front of its heading, and
    /// the lower half is past its whole section. The line drawn is the one the section appears at.
    #[test]
    fn a_drop_lands_before_a_heading_or_after_its_section() {
        let (items, rows) = tree();
        let at_row = |i: usize, frac: f32| rows[i].1.min.y + ROW_H * frac;
        let path = |v: &Value| v.as_array().map(|a| a.iter().filter_map(Value::as_u64).collect::<Vec<_>>());
        let land = |y: f32| landing(&items, &rows, pos2(10.0, y)).map(|(at, line)| (path(&at), line));
        // Above the first row: the very top of the outline.
        assert_eq!(land(-5.0), Some((Some(vec![0]), rows[0].1.min.y)));
        // The upper half of a row is in front of it; the lower half is past it.
        assert_eq!(land(at_row(1, 0.25)), Some((Some(vec![1]), rows[1].1.min.y)));
        assert_eq!(land(at_row(0, 0.75)), Some((Some(vec![2]), rows[2].1.min.y)), "past One and the subheading under it");
        assert_eq!(land(at_row(1, 0.75)), Some((Some(vec![2]), rows[2].1.min.y)), "past One A's section");
        // Past the last section there is nothing left to go in front of.
        assert_eq!(land(at_row(2, 0.75)), None);
        assert_eq!(land(rows[2].1.max.y + 5.0), None);
    }

    /// An empty outline has nowhere to drop, and the pane must not index into nothing.
    #[test]
    fn a_drop_into_an_empty_outline_lands_nowhere() {
        assert!(landing(&[], &[], pos2(0.0, 0.0)).is_none());
    }
}
