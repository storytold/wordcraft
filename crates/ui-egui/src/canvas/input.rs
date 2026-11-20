use std::sync::Arc;

use craft::behavior::CursorIcon;
use craft::behavior::cursor::{Cursor, PaintedCursor};
use craft::behavior::value::CancelReason;
use craft::host::target::{Os, Runtime, ShellInfo};
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use super::links::{field_showing_code, follow_link, is_generated_bookmark_link, link_under};
use super::story::{enters_header_footer, in_text_box, part_kind, text_box_host};
use super::{CanvasState, DocLayout, PartKind, Pos, Pos2, StoryRef, WordApp, json};
use crate::objects::ObjectGesture;

/// See [`CanvasState::unit_drag`].
pub(super) struct UnitDrag {
    pub(super) unit: &'static str,
    /// Clicks in a row that made it (2 or 3; 1 for Ctrl/⌘+click).
    clicks: u8,
    start: Pos,
    end: Pos,
    last: Pos,
}

/// After a whole-document selection from a press (not a line, so it can't grow): holds the press
/// so its release isn't a caret click.
pub(crate) fn hold_document_selection(app: &mut WordApp, clicks: u8) {
    let (start, end) = app.session.sel.ordered();
    app.canvas.unit_drag = Some(UnitDrag { unit: "document", clicks, start, last: end.clone(), end });
    app.canvas.dragging = true;
}

/// The release of a press that [`hold_document_selection`] held: the hold ends. A table's selection part takes
/// that press and its release (the page's text tool sees neither), so without this the hold would stay and the next
/// click would be read as part of it.
pub(crate) fn release_document_selection(app: &mut WordApp) {
    if app.canvas.unit_drag.as_ref().is_some_and(|d| d.unit == "document") {
        app.canvas.unit_drag = None;
        app.canvas.dragging = false;
    }
}

impl CanvasState {
    /// True while a mouse drag-select is in progress (the mini toolbar waits for mouse-up).
    pub fn drag_selecting(&self) -> bool {
        self.dragging
    }
}

/// Paint the selection with the Format Painter's formatting, when it is armed and something is
/// selected.
fn apply_format_painter(app: &mut WordApp) {
    if app.session.painter.is_some() {
        let _ = app.run("edit.pasteFormat", json!({}));
    }
}

/// The page's text tool: selection, caret placement and the page's editor input of one pass. Built
/// inside the page-view body and dropped there, so nothing outlives the pass; `body` is this
/// pass's placement, never the last pass's record.
pub(crate) struct PageTextTool<'a> {
    app: &'a mut WordApp,
    layout: Arc<DocLayout>,
    body: &'a PageBody<'a>,
    /// The Windows shell draws its own I-beam.
    draws_ibeam: bool,
    /// The modifier a link tip names for following the link.
    link_key: &'static str,
}

/// What the page tool needs of a press, resolved once.
#[derive(Clone, Copy)]
struct PagePointer<'a> {
    layout: &'a DocLayout,
    page: usize,
    x: f32,
    y: f32,
    story: StoryRef,
    clicks: u8,
}

/// The page under `p` and the point on it. With Hide White Space the pages' rects overlap in the margins it hides: a
/// point there belongs to the page whose shown band is nearest, so a click just past one page's text is still on it.
fn page_of(pages: &[PlacedPage], p: Pos2) -> Option<(usize, f32, f32)> {
    let away = |pl: &PlacedPage| {
        let y = p.y - pl.rect.min.y;
        (pl.band.0 - y).max(0.0).max(y - pl.band.1)
    };
    let placed = pages.iter().filter(|pl| pl.rect.contains(p)).min_by(|a, b| away(a).total_cmp(&away(b)))?;
    let at = placed.to_page(p);
    Some((placed.page, at.x as f32, at.y as f32))
}

impl<'a> PageTextTool<'a> {
    /// The tool for this pass over `app`'s document. The shell facts say whether the I-beam is
    /// drawn (Windows, native: on the web the host OS is the browser's) and which key follows a link.
    // PLAN: the sheet's `pass` and `ctx` parameters are not taken: the page's keys run from `run_page_tool` (S-L240) and the tip goes through `ToolContext::tip`; canvas-w9 drops both from its call.
    pub(crate) fn new(app: &'a mut WordApp, layout: Arc<DocLayout>, body: &'a PageBody<'a>, shell: &ShellInfo) -> Self {
        let draws_ibeam = shell.os == Os::Windows && shell.runtime == Runtime::Native;
        let link_key = if shell.os == Os::Mac { "⌘" } else { "Ctrl" };
        PageTextTool { app, layout, body, draws_ibeam, link_key }
    }

    fn pointer_at(&self, pos: Pos2, clicks: u32) -> Option<(usize, f32, f32, u8)> {
        let (page, x, y) = page_of(self.body.pages, pos)?;
        Some((page, x, y, clicks.min(3) as u8))
    }

    fn hf_zone(&self, page: usize, y: f32) -> Option<(Option<StoryRef>, bool)> {
        if self.app.session.view.mode == wordcraft_layout::ViewMode::Print && !self.app.session.view.read_mode {
            self.layout.header_footer_zone(page, y)
        } else {
            None
        }
    }

    /// Grows the selection to where the dragging pointer is.
    fn drag_select(&mut self, p: Pos2) {
        let layout = self.layout.clone();
        let Some((page, x, y)) = page_of(self.body.pages, p) else { return };
        let story = self.app.session.sel.focus.story;
        let app = &mut *self.app;
        if let Some((from, last)) = app.canvas.column_drag {
            if last != (page, x, y) {
                app.canvas.column_drag = Some((from, (page, x, y)));
                let _ = app.run(
                    "select.column",
                    json!({"from": {"page": from.0, "x": from.1, "y": from.2}, "to": {"page": page, "x": x, "y": y}, "story": story}),
                );
            }
        } else if let Some(d) = app.canvas.unit_drag.as_mut() {
            // Lines grow line by line wherever the pointer is across; the whole document can't grow.
            let target = match d.unit {
                "line" => layout.body_line_at(page, y),
                "document" => None,
                _ => layout.hit(page, x, y, story),
            };
            if let Some(pos) = target
                && pos != d.last
            {
                d.last = pos.clone();
                let params = json!({"unit": d.unit, "start": d.start, "end": d.end, "pos": pos});
                let _ = app.run("select.dragUnit", params);
                app.session.page_hint = page;
                app.canvas.glide.restart(craft::platform::clock::monotonic_ms());
            }
        } else if let Some(pos) = layout.hit(page, x, y, story)
            && pos != app.session.sel_ends().focus
        {
            // Through the command, so from one table cell into another the drag takes whole cells
            // (Word); straight to the session, as the app's `run` would scroll on every frame.
            let pj = serde_json::to_value(&pos).unwrap_or_default();
            let _ = app.session.run("caret.set", &json!({"pos": pj, "extend": true}));
            app.session.page_hint = page;
            app.canvas.glide.restart(craft::platform::clock::monotonic_ms());
        }
    }

    /// The tip for the link under `pos`: where it goes and how to follow it.
    fn link_tip(&self, pos: Pos2) -> Option<String> {
        let app = &*self.app;
        let (page, x, y) = page_of(self.body.pages, pos)?;
        let layout = &*self.layout;
        // A linked picture or shape under the pointer comes first: it lies over the text (#5267).
        let story = app.session.sel.focus.story;
        let link = crate::objects::link_at(app, layout, self.body, pos)
            .or_else(|| layout.hit(page, x, y, story).and_then(|at| link_under(app, layout, page, x, y, &at)))?;
        let target = if is_generated_bookmark_link(&link) { tl!("Current Document").to_string() } else { link };
        Some(format!("{target}\n{}", crate::i18n::fmt(tl!("{key}+Click to follow link"), &[("key", self.link_key)])))
    }
}

impl CanvasTool for PageTextTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let layout = self.layout.clone();
        let object = ObjectGesture::press(self.app, &layout, self.body, pointer, cx);
        // A press on a Behind Text object is also a press on the text under it until it becomes a drag.
        // PLAN: assumed `ObjectGesture::pending_behind(&self) -> bool` (canvas-w11).
        let over_text = object.as_ref().is_some_and(ObjectGesture::pending_behind);
        if let Some(g) = object {
            self.app.canvas.object_gesture = Some(g);
            if !over_text {
                return ToolAction::Claimed;
            }
        }
        let ended = || if over_text { ToolAction::Claimed } else { ToolAction::Done };
        let Some((page, x, y, clicks)) = self.pointer_at(pointer.pos, pointer.clicks) else {
            return if over_text { ToolAction::Claimed } else { ToolAction::Ignored };
        };
        let story = self.app.session.sel.focus.story;
        let pp = PagePointer { layout: &layout, page, x, y, story, clicks };
        let m = pointer.mods;
        let mods = egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: false, command: m.cmd };
        if skip_equation_or_outline(self.app, &pp) {
            return ended();
        }
        let hf_zone = self.hf_zone(page, y);
        let hf_enter = enters_header_footer(self.app, story, hf_zone);
        handle_primary_press(self.app, &pp, mods, hf_zone, hf_enter);
        // A press in the header or footer band keeps the gesture: its release decides a double-click there
        // (`handle_completed_click` enters the header or footer), and a tool that is done gets no release.
        if over_text || self.app.canvas.dragging || hf_zone.is_some() { ToolAction::Claimed } else { ToolAction::Done }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let layout = self.layout.clone();
        if let Some(mut g) = self.app.canvas.object_gesture.take() {
            g.drag(self.app, &layout, self.body, pointer, cx);
            self.app.canvas.object_gesture = Some(g);
            return;
        }
        if !self.app.canvas.dragging {
            return;
        }
        // Past the viewport's edge the pointer hits where it would be inside the view, so the host's
        // edge autoscroll keeps growing the selection under a stationary pointer (#688).
        let p = cx.rect.clamp(pointer.pos);
        self.drag_select(p);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let layout = self.layout.clone();
        if let Some(g) = self.app.canvas.object_gesture.take() {
            // `false`: a click on the text under a Behind Text object, finished as a text click below.
            if g.release(self.app, &layout, self.body, pointer, cx) {
                self.app.canvas.dragging = false;
                self.app.canvas.column_drag = None;
                self.app.canvas.unit_drag = None;
                return;
            }
        }
        if let Some((page, x, y, clicks)) = self.pointer_at(pointer.pos, pointer.clicks) {
            let story = self.app.session.sel.focus.story;
            let pp = PagePointer { layout: &layout, page, x, y, story, clicks };
            if handle_completed_click(self.app, &pp) {
                return;
            }
        }
        self.app.canvas.dragging = false;
        self.app.canvas.column_drag = None;
        self.app.canvas.unit_drag = None;
        apply_format_painter(self.app);
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // PLAN: assumed `ToolContext::tip(&mut self, tip: impl Into<Tooltip>)` (canvas-w1, S-L238).
        if let Some(tip) = self.link_tip(pointer.pos) {
            cx.tip(tip);
        }
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if let Some(g) = self.app.canvas.object_gesture.take() {
            g.cancel(self.app);
        }
        self.app.canvas.dragging = false;
        self.app.canvas.column_drag = None;
        self.app.canvas.unit_drag = None;
    }

    fn paint(&self, draw: &Draw<'_>, _cx: &ToolContext<'_>) {
        if let Some(g) = self.app.canvas.object_gesture() {
            g.paint(&*self.app, draw, self.body);
        }
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let text = || if self.draws_ibeam { Cursor::Painted(PaintedCursor::IBeam) } else { Cursor::System(CursorIcon::Text) };
        let Some(pointer) = cx.pointer else { return text() };
        if let Some(c) = ObjectGesture::cursor(&*self.app, &self.layout, self.body, pointer.pos, pointer.mods) {
            return c;
        }
        let Some((page, x, y)) = page_of(self.body.pages, pointer.pos) else { return Cursor::System(CursorIcon::Default) };
        // The selection bar keeps its arrow.
        if self.app.session.sel.focus.story == StoryRef::Body && !pointer.mods.alt && self.layout.selection_bar_at(page, x, y).is_some() {
            return Cursor::System(CursorIcon::Default);
        }
        if self.app.session.painter.is_some() {
            return Cursor::System(CursorIcon::Copy);
        }
        text()
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the caret.set command")
    }
}

/// A right click on the page, before its menu opens, so the menu acts on what was clicked: a table cell selection it
/// misses drops; in an equation the caret goes there and the equation is edited (off it, an equation being edited is
/// left); a picture, shape or text box under it is selected; otherwise a selection it lands in stays and the caret
/// goes there.
pub(crate) fn handle_secondary_selection(app: &mut WordApp, body: &PageBody<'_>, layout: &DocLayout, at: Pos2) {
    let Some((page, x, y)) = page_of(body.pages, at) else { return };
    let point = wordcraft_geom::Point::new(x, y);
    // Selected table cells: a right-click off them drops them, even between the selection's ends
    // (beside a selected column), so the menu acts on the cell clicked.
    if let Some(range) = app.session.cell_range() {
        let on_cells =
            wordcraft_engine::cmd::table_mouse::cell_rects(layout, &app.session.doc, range).iter().any(|(pi, r)| *pi == page && r.contains(point));
        // On a selected cell the cells stay selected for the menu.
        if on_cells {
            return;
        }
        if let Some(pos) = layout.hit(page, x, y, app.session.sel.focus.story) {
            app.session.sel = wordcraft_engine::Selection::caret(pos);
            app.session.page_hint = page;
        }
    }
    match layout.equation_hit(page, x, y, app.session.sel.focus.story) {
        Some((pos, inner)) => {
            app.session.sel = wordcraft_engine::Selection::caret(pos);
            let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
            return;
        }
        None if app.session.math.is_some() => {
            let _ = app.run("equation.exit", json!({}));
        }
        None => {}
    }
    if app.session.math.is_some() {
        return;
    }
    if crate::objects::select_at(app, layout, body, at) {
        app.canvas.context_issue = None;
        app.canvas.context_synonyms = None;
        return;
    }
    if !app.session.sel.is_collapsed() {
        let (from, to) = app.session.sel.ordered();
        if layout.selection_rects(&app.session.doc, &from, &to, page).into_iter().any(|(pi, r)| pi == page && r.contains(point)) {
            return;
        }
    }
    if let Some(pos) = layout.hit(page, x, y, app.session.sel.focus.story) {
        app.session.sel = wordcraft_engine::Selection::caret(pos);
        app.session.page_hint = page;
    }
}

/// A press. `true` when it was handled and the pointer is done.
fn handle_primary_press(
    app: &mut WordApp,
    pointer: &PagePointer<'_>,
    mods: egui::Modifiers,
    hf_zone: Option<(Option<StoryRef>, bool)>,
    hf_enter: bool,
) -> bool {
    let PagePointer { layout, page, x, y, story, clicks, .. } = *pointer;
    app.canvas.unit_drag = None;
    if clicks <= 1 {
        app.canvas.hf_press_switched = false;
    }
    // A double-click that enters or leaves a header/footer is handled on release.
    let hf_switch = hf_enter || app.canvas.hf_press_switched || (matches!(story, StoryRef::Part(_)) && !in_text_box(app) && hf_zone.is_none());
    // Clicking into a footnote/endnote or a text box edits it; clicking outside a text box goes
    // back to where it is. While editing a header/footer, only its own text boxes are in reach.
    let story = pointer_story(app, layout, page, x, y, story);
    // While editing a header/footer, a press in another one (the footer while editing the
    // header, or another page's own) edits that one, with the caret at the press. Hit-tested
    // against the story being edited, the press put the caret back in it, scrolling the view
    // away from the band that was clicked.
    if !mods.shift
        && matches!(part_kind(app, story), Some(PartKind::Header | PartKind::Footer))
        && let Some((_, header)) = hf_zone
        && enters_header_footer(app, story, hf_zone)
    {
        app.canvas.column_drag = None;
        app.canvas.hf_press_switched = true;
        let id = if header { "insert.editHeader" } else { "insert.editFooter" };
        if app.run(id, json!({ "page": page })).is_ok() {
            let to = app.session.sel.focus.story;
            if let Some(pos) = app.session.layout().hit(page, x, y, to) {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            app.session.page_hint = page;
        }
        return true;
    }
    // A press in the header/footer area from the body leaves the caret where it is, as in
    // Word (#2103): moving it to the nearest body text scrolled the view there, away from the
    // footer the double-click was aiming at.
    if hf_switch && !mods.shift && hf_zone.is_some() && story == StoryRef::Body && app.session.sel.focus.story == StoryRef::Body {
        app.canvas.column_drag = None;
        return true;
    }
    if !mods.shift
        && let Some((at, inner)) = layout.equation_hit(page, x, y, story)
    {
        app.session.sel = wordcraft_engine::Selection::caret(at);
        let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
        app.session.page_hint = page;
        return true;
    }
    if app.session.math.is_some() {
        let _ = app.run("equation.exit", json!({}));
    }
    // The selection bar (the left margin beside body text, #856): a click selects the line,
    // a double-click the paragraph, a triple-click or Ctrl/⌘+click the document; Shift+click
    // extends by lines; dragging grows the selection by the clicked unit.
    if story == StoryRef::Body
        && !mods.alt
        && let Some(line) = layout.selection_bar_at(page, x, y)
    {
        app.canvas.column_drag = None;
        app.session.page_hint = page;
        let lj = serde_json::to_value(&line).unwrap_or_default();
        let (unit, start, end) = if mods.shift {
            let anchor = app.session.sel.anchor.clone();
            let aj = serde_json::to_value(&anchor).unwrap_or_default();
            let _ = app.run("select.dragUnit", json!({"unit": "line", "start": aj, "end": aj, "pos": lj}));
            ("line", anchor.clone(), anchor)
        } else {
            let unit = match clicks {
                _ if mods.command => "document",
                3.. => "document",
                2 => "paragraph",
                _ => "line",
            };
            let _ = app.run("caret.set", json!({"pos": lj}));
            let id = match unit {
                "document" => "select.all",
                "paragraph" => "select.paragraph",
                _ => "select.line",
            };
            let params = if id == "select.all" { json!({"whole": true}) } else { json!({}) };
            let _ = app.run(id, params);
            let (start, end) = app.session.sel.ordered();
            (unit, start, end)
        };
        app.canvas.unit_drag = Some(UnitDrag { unit, clicks, start, end, last: line });
        app.canvas.dragging = true;
        return true;
    }
    let Some(pos) = layout.hit(page, x, y, story) else { return true };
    // Ctrl/⌘+click follows a hyperlink.
    if mods.command
        && let Some(link) = link_under(app, layout, page, x, y, &pos)
    {
        follow_link(app, link);
        return true;
    }
    let pj = serde_json::to_value(&pos).unwrap_or_default();
    // Alt+drag selects a column (block) of text.
    if mods.alt && !mods.shift && !mods.command {
        let _ = app.run("caret.set", json!({"pos": pj}));
        app.session.page_hint = page;
        app.canvas.column_drag = Some(((page, x, y), (page, x, y)));
        app.canvas.dragging = true;
        return true;
    }
    app.canvas.column_drag = None;
    let _ = app.run("caret.set", json!({"pos": pj, "extend": mods.shift}));
    // Double-click selects a word, triple-click a paragraph, Ctrl/⌘+click a sentence; holding
    // the button and dragging then extends by whole units (Word, #549).
    let unit = match clicks {
        _ if mods.shift => None,
        3 if !hf_switch => Some("paragraph"),
        2 if !hf_switch => Some("word"),
        1 if mods.command => Some("sentence"),
        _ => None,
    };
    if let Some(unit) = unit {
        let _ = app.run(&format!("select.{unit}"), json!({}));
        let (start, end) = app.session.sel.ordered();
        app.canvas.unit_drag = Some(UnitDrag { unit, clicks, start, end, last: pos });
    }
    app.session.page_hint = page;
    app.canvas.dragging = true;
    // Format painter applies on mouse up
    false
}

fn pointer_story(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32, story: StoryRef) -> StoryRef {
    let is_note = |s| matches!(part_kind(app, s), Some(PartKind::Footnote | PartKind::Endnote));
    let is_box = |s| part_kind(app, s) == Some(PartKind::TextBox);
    let host = if is_box(story) { text_box_host(app, layout).unwrap_or(StoryRef::Body) } else { story };
    let in_hf = matches!(part_kind(app, host), Some(PartKind::Header | PartKind::Footer));
    let hf_box = if in_hf { layout.header_footer_text_box_at(page, x, y).map(StoryRef::Part) } else { None };
    match (hf_box, layout.story_at(page, x, y)) {
        (Some(b), _) => b,
        (None, Some(s)) if is_note(s) => s,
        (None, Some(s)) if is_box(s) && !in_hf => s,
        (None, Some(StoryRef::Body)) if is_note(story) => StoryRef::Body,
        _ if is_box(story) => host,
        _ => story,
    }
}

/// A release that completes a multi-click. The press already selected the word or paragraph (and a
/// drag may have grown it). `false` for a plain click or drag.
fn handle_completed_click(app: &mut WordApp, pointer: &PagePointer<'_>) -> bool {
    let PagePointer { layout, page, x, y, story, clicks, .. } = *pointer;
    let released_clicks = if clicks >= 2 { clicks } else { 0 };
    if released_clicks > 0 && app.canvas.unit_drag.as_ref().is_some_and(|d| d.clicks == released_clicks) {
        app.canvas.unit_drag = None;
        app.canvas.dragging = false;
        // Double-clicking a word (or triple-clicking a paragraph) with the Format Painter armed
        // paints it, as dragging across it does (#4410).
        apply_format_painter(app);
        return true;
    }
    // Double-click in the header/footer area edits it (creating it when the page has none, #960);
    // double-click in the body leaves it. Print Layout only: the other views show no margins.
    let hf_zone = if app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode {
        layout.header_footer_zone(page, y)
    } else {
        None
    };
    let hf_enter = enters_header_footer(app, story, hf_zone);
    if clicks == 2 {
        // Its first press already moved into the header/footer and placed the caret.
        if app.canvas.hf_press_switched {
            return true;
        }
        if let Some((_, header)) = hf_zone
            && hf_enter
        {
            let id = if header { "insert.editHeader" } else { "insert.editFooter" };
            if app.run(id, json!({ "page": page })).is_ok()
                && let Some(pos) = app.session.layout().hit(page, x, y, app.session.sel.focus.story)
            {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            return true;
        }
        if matches!(story, StoryRef::Part(_)) && !in_text_box(app) && hf_zone.is_none() {
            let _ = app.run("insert.closeHeader", json!({}));
            if let Some(pos) = layout.hit(page, x, y, StoryRef::Body) {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            return true;
        }
        // Double-click a field showing its code: edit the code.
        if let Some(pos) = layout.hit(page, x, y, story)
            && let Some(field) = field_showing_code(app, &pos)
        {
            app.session.sel = wordcraft_engine::Selection::caret(field);
            let _ = app.run("fields.setCode", json!({}));
            return true;
        }
        let _ = app.run("select.word", json!({}));
        return true;
    }
    if clicks >= 3 {
        let _ = app.run("select.paragraph", json!({"cell": true}));
        return true;
    }
    false
}

/// A press the equation or the outline takes whole.
fn skip_equation_or_outline(app: &mut WordApp, pointer: &PagePointer<'_>) -> bool {
    let PagePointer { layout, page, x, y, story, clicks } = *pointer;
    // Inside an equation, clicks place the caret in it (and double-clicks don't select words).
    if clicks >= 2 && layout.equation_hit(page, x, y, story).is_some() {
        return true;
    }
    // Outline view: a paragraph's symbol selects it with what is under it; a double-click
    // expands or collapses the heading.
    if app.session.view.mode == wordcraft_layout::ViewMode::Outline
        && let Some(block) = layout.outline_symbol_at(page, x, y)
    {
        if clicks >= 2 {
            let _ = app.run("outline.toggle", json!({ "block": block }));
        } else {
            let _ = app.run("outline.selectSubtree", json!({ "block": block }));
            app.session.page_hint = page;
        }
        return true;
    }
    false
}
