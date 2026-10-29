use super::balloons::over_balloon;
use super::links::{field_showing_code, is_followable_link, is_generated_bookmark_link, link_under};
use super::navigation::page_at;
use super::story::{enters_header_footer, in_text_box, part_kind, straight_box, text_box_host};
use super::{CanvasState, DocLayout, PartKind, Pos, Pos2, Rect, StoryRef, Ui, WordApp, json};

/// Drag-select auto-scroll (#688): the pointer within this many points of the viewport's edge
/// starts scrolling, like Word.
pub(super) const EDGE_MARGIN: f32 = 32.0;

/// Auto-scroll speed (points/s) at the margin's inner boundary and at (or past) the edge.
const EDGE_SPEED_MIN: f32 = 45.0;

pub(super) const EDGE_SPEED_MAX: f32 = 720.0;

/// Longest frame time trusted for auto-scroll (a stall shouldn't jump the page).
pub(super) const EDGE_MAX_DT: f32 = 0.1;

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

/// Clicks in a row for a press at `at` (time `now`) after `last`: within the double-click
/// delay and distance it counts up to 3 (a triple-click), otherwise it starts again at 1.
pub(crate) fn press_count(last: Option<(f64, Pos2, u8)>, now: f64, at: Pos2, delay: f64, dist: f32) -> u8 {
    match last {
        Some((t, p, n)) if now - t >= 0.0 && now - t < delay && p.distance(at) <= dist => n.saturating_add(1).min(3),
        _ => 1,
    }
}

impl CanvasState {
    /// True while a mouse drag-select is in progress (the mini toolbar waits for mouse-up).
    pub fn drag_selecting(&self) -> bool {
        self.dragging
    }
}

/// Whether wheel input counts as being over the canvas. Another widget's drag (a scrollbar, say)
/// owns the wheel, but a drag-select on the canvas itself keeps scrolling so a long passage can be selected.
pub(super) fn wheel_hovered(over_canvas: bool, any_drag: bool, drag_selecting: bool) -> bool {
    over_canvas && (!any_drag || drag_selecting)
}

/// The drag auto-scroll along one axis for a pointer at `p` inside the viewport band `lo..hi`, in
/// the wheel's convention (positive = content moves down the screen, toward the document's start):
/// nothing while the pointer is [`EDGE_MARGIN`] or more inside, then ramping from
/// [`EDGE_SPEED_MIN`] at the margin to [`EDGE_SPEED_MAX`] at the edge (and capping beyond it),
/// applied over `dt` seconds. Junk numbers scroll nothing.
pub(super) fn edge_scroll(p: f32, lo: f32, hi: f32, dt: f32) -> f32 {
    if !(p.is_finite() && lo.is_finite() && hi.is_finite() && dt.is_finite()) || hi <= lo {
        return 0.0;
    }
    let dt = dt.clamp(0.0, EDGE_MAX_DT);
    let speed = |into: f32| egui::emath::lerp(EDGE_SPEED_MIN..=EDGE_SPEED_MAX, (into / EDGE_MARGIN).clamp(0.0, 1.0));
    if p < lo + EDGE_MARGIN {
        speed(lo + EDGE_MARGIN - p) * dt
    } else if p > hi - EDGE_MARGIN {
        -speed(p - (hi - EDGE_MARGIN)) * dt
    } else {
        0.0
    }
}

/// The scroll offset after `delta` (the wheel's convention) reaches `before`: clamped to the
/// scrollable range, so a document shorter than the viewport never moves.
pub(super) fn clamped_scroll(before: egui::Vec2, delta: egui::Vec2, max: egui::Vec2) -> egui::Vec2 {
    (before - delta).clamp(egui::Vec2::ZERO, max)
}

/// Routes pointer input: pen or eraser ink, object and crop drags, table border, selection and move
/// handles, then page hit testing, outlines, headers and footers, equations, selection and caret
/// interaction.
pub(super) fn mouse(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let Some(pointer) = route_pointer_targets(app, ui, resp, rects, layout, scale) else {
        return;
    };
    // While drag-selecting, the pointer may sit past the viewport's edge (or the window's):
    // hit-test where it would be inside the view, so as the edge auto-scroll moves the content the
    // selection keeps growing under a stationary pointer (#688).
    let p = if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && let Some(vp) = app.canvas.viewport
    {
        vp.clamp(pointer)
    } else {
        pointer
    };
    let Some((page, x, y)) = page_at(rects, layout, scale, p) else { return };
    let story = app.session.sel.focus.story;
    handle_page_pointer(app, PagePointer { ui, resp, layout, p, page, x, y, story });
}

fn route_pointer_targets(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> Option<Pos2> {
    // A pen or the eraser (Draw tab) owns the pointer.
    if crate::ink::pointer(app, ui, resp, rects, layout, scale) {
        return None;
    }
    let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
    // An object (or crop handle) drag follows the pointer anywhere until it's released.
    let dragging = app.canvas.obj_drag.is_some() || app.canvas.crop_drag.is_some();
    let object_pointer = pointer.or_else(|| dragging.then(|| ui.input(|i| i.pointer.latest_pos())).flatten());
    if let Some(at) = object_pointer
        && crate::objects::pointer(app, ui, resp, rects, layout, scale, at)
    {
        return None;
    }
    // A press on a table border (or the table's resize handle) drags it instead of the caret.
    if crate::table_resize::pointer(app, ui, resp, rects, layout, scale) {
        return None;
    }
    // A press on a table's selection area, insert button or move handle.
    if crate::table_mouse::pointer(app, ui, resp, rects, layout, scale) {
        return None;
    }
    pointer
}

#[derive(Clone, Copy)]
struct PagePointer<'a> {
    ui: &'a Ui,
    resp: &'a egui::Response,
    layout: &'a DocLayout,
    p: Pos2,
    page: usize,
    x: f32,
    y: f32,
    story: StoryRef,
}

fn handle_page_pointer(app: &mut WordApp, pointer: PagePointer<'_>) {
    let PagePointer { ui, resp, layout, p, page, x, y, story, .. } = pointer;
    let mods = ui.input(|i| i.modifiers);
    if skip_equation_or_outline(app, &pointer) {
        return;
    }
    let on_frame = app.canvas.eq_options.is_some_and(|f| f.strip.contains(p) || f.grip.contains(p));
    if handle_completed_click(app, &pointer) {
        return;
    }
    let hf_zone = if app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode {
        layout.header_footer_zone(page, y)
    } else {
        None
    };
    let hf_enter = enters_header_footer(app, story, hf_zone);
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() && !over_balloon(app, p);
    // The equation frame's grip and options strip stay in the equation (the strip's click opens
    // its menu, in `show`).
    if pressed && on_frame {
        return;
    }
    if pressed {
        handle_primary_press(app, &pointer, mods, hf_zone, hf_enter);
    } else if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && let Some((from, last)) = app.canvas.column_drag
    {
        if last != (page, x, y) {
            app.canvas.column_drag = Some((from, (page, x, y)));
            let story = app.session.sel.focus.story;
            let _ = app.run(
                "select.column",
                json!({"from": {"page": from.0, "x": from.1, "y": from.2}, "to": {"page": page, "x": x, "y": y}, "story": story}),
            );
        }
    } else if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && let Some(d) = app.canvas.unit_drag.as_mut()
    {
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
            app.canvas.caret_visible_since = crate::now_ms();
        }
    } else if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && app.canvas.turned_press.is_some_and(|p0| p0.distance(p) < ui.ctx().options(|o| o.input_options.max_click_dist))
    {
        // Pressed into a turned text box (now drawn straight) and not yet dragged: nothing to extend.
    } else if app.canvas.dragging && ui.input(|i| i.pointer.primary_down()) {
        app.canvas.turned_press = None;
        // Through the command, so from one table cell into another the drag takes whole cells
        // (Word); straight to the session, as the app's `run` would scroll on every frame.
        if let Some(pos) = layout.hit(page, x, y, story)
            && pos != app.session.sel_ends().focus
        {
            let pj = serde_json::to_value(&pos).unwrap_or_default();
            let _ = app.session.run("caret.set", &json!({"pos": pj, "extend": true}));
            app.session.page_hint = page;
            app.canvas.caret_visible_since = crate::now_ms();
        }
        // The pointer was clamped into the viewport above, so when the edge auto-scroll moves the
        // content under it, this hit-test keeps the selection growing (#688).
    } else if app.canvas.dragging && ui.input(|i| i.pointer.primary_released()) {
        app.canvas.dragging = false;
        app.canvas.turned_press = None;
        app.canvas.column_drag = None;
        app.canvas.unit_drag = None;
        if app.session.painter.is_some() && !app.session.sel.is_collapsed() {
            let _ = app.run("edit.pasteFormat", json!({}));
        }
    }
    show_hover_link(app, &pointer);
}

fn handle_primary_press(
    app: &mut WordApp,
    pointer: &PagePointer<'_>,
    mods: egui::Modifiers,
    hf_zone: Option<(Option<StoryRef>, bool)>,
    hf_enter: bool,
) {
    let PagePointer { ui, layout, p, page, x, y, story, .. } = *pointer;
    let now = ui.input(|i| i.time);
    let (delay, dist) = ui.ctx().options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_dist));
    let clicks = press_count(app.canvas.last_press, now, p, delay, dist);
    app.canvas.last_press = Some((now, p, clicks));
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
        return;
    }
    // A press in the header/footer area from the body leaves the caret where it is, as in
    // Word (#2103): moving it to the nearest body text scrolled the view there, away from the
    // footer the double-click was aiming at.
    if hf_switch && !mods.shift && hf_zone.is_some() && story == StoryRef::Body && app.session.sel.focus.story == StoryRef::Body {
        app.canvas.column_drag = None;
        return;
    }
    if !mods.shift
        && let Some((at, inner)) = layout.equation_hit(page, x, y, story)
    {
        app.session.sel = wordcraft_engine::Selection::caret(at);
        let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
        app.session.page_hint = page;
        return;
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
        return;
    }
    // A turned text box not being edited is drawn turned: the click is taken into its frame.
    let straight = straight_box(app, layout);
    let Some(pos) = layout.hit_shown(page, x, y, story, straight) else { return };
    // Ctrl/⌘+click follows a hyperlink.
    if mods.command
        && let shown = layout.shown_point(page, x, y, story, straight)
        && let Some(link) = link_under(app, layout, page, shown.x, shown.y, &pos)
    {
        if let Some(name) = link.strip_prefix('#') {
            // The caret goes to the bookmark's start, as in Word (Go To selects its text).
            if app.run("edit.goto", json!({"bookmark": name})).is_ok() {
                let _ = app.run("select.collapse", json!({}));
            }
        } else if is_followable_link(&link) {
            app.canvas.open_url = Some(link);
        } else {
            app.status(tl!("Only web (http, https) and email (mailto) links can be followed"));
        }
        return;
    }
    let pj = serde_json::to_value(&pos).unwrap_or_default();
    // Alt+drag selects a column (block) of text.
    if mods.alt && !mods.shift && !mods.command {
        let _ = app.run("caret.set", json!({"pos": pj}));
        app.session.page_hint = page;
        app.canvas.column_drag = Some(((page, x, y), (page, x, y)));
        app.canvas.dragging = true;
        return;
    }
    app.canvas.column_drag = None;
    let _ = app.run("caret.set", json!({"pos": pj, "extend": mods.shift}));
    // Clicking into a turned text box draws it straight under the pointer: holding still
    // mustn't then select from where the pointer is on the straight box.
    let now = app.session.layout();
    app.canvas.turned_press = (straight_box(app, &now) != straight).then_some(p);
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
}

fn pointer_story(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32, story: StoryRef) -> StoryRef {
    let is_note = |s| matches!(part_kind(app, s), Some(PartKind::Footnote | PartKind::Endnote));
    let is_box = |s| part_kind(app, s) == Some(PartKind::TextBox);
    let host = if is_box(story) { text_box_host(app, layout).unwrap_or(StoryRef::Body) } else { story };
    let in_hf = matches!(part_kind(app, host), Some(PartKind::Header | PartKind::Footer));
    let hf_box = if in_hf { layout.header_footer_text_box_at(page, x, y).map(StoryRef::Part) } else { None };
    match (hf_box, layout.story_at_shown(page, x, y, straight_box(app, layout))) {
        (Some(b), _) => b,
        (None, Some(s)) if is_note(s) => s,
        (None, Some(s)) if is_box(s) && !in_hf => s,
        (None, Some(StoryRef::Body)) if is_note(story) => StoryRef::Body,
        _ if is_box(story) => host,
        _ => story,
    }
}

fn handle_completed_click(app: &mut WordApp, pointer: &PagePointer<'_>) -> bool {
    let PagePointer { resp, layout, page, x, y, story, .. } = *pointer;
    // The press already selected the word or paragraph (and a drag may have grown it).
    let released_clicks = if resp.triple_clicked() {
        3
    } else if resp.double_clicked() {
        2
    } else {
        0
    };
    if released_clicks > 0 && app.canvas.unit_drag.as_ref().is_some_and(|d| d.clicks == released_clicks) {
        app.canvas.unit_drag = None;
        app.canvas.dragging = false;
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
    if resp.double_clicked() {
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
    if resp.triple_clicked() {
        let _ = app.run("select.paragraph", json!({"cell": true}));
        return true;
    }
    false
}

fn skip_equation_or_outline(app: &mut WordApp, pointer: &PagePointer<'_>) -> bool {
    let PagePointer { ui, resp, layout, p, page, x, y, story, .. } = pointer;
    // Inside an equation, clicks place the caret in it (and double-clicks don't select words);
    // so do clicks on the grip and options strip of its frame.
    let on_frame = app.canvas.eq_options.is_some_and(|f| f.strip.contains(*p) || f.grip.contains(*p));
    if (resp.double_clicked() || resp.triple_clicked()) && (on_frame || layout.equation_hit(*page, *x, *y, *story).is_some()) {
        return true;
    }
    // Outline view: a paragraph's symbol selects it with what is under it; a double-click
    // expands or collapses the heading.
    if app.session.view.mode == wordcraft_layout::ViewMode::Outline
        && let Some(block) = layout.outline_symbol_at(*page, *x, *y)
    {
        if resp.double_clicked() {
            let _ = app.run("outline.toggle", json!({ "block": block }));
        } else if ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() {
            let _ = app.run("outline.selectSubtree", json!({ "block": block }));
            app.session.page_hint = *page;
        }
        return true;
    }
    false
}

fn show_hover_link(app: &WordApp, pointer: &PagePointer<'_>) {
    let PagePointer { ui, resp, layout, page, x, y, story, .. } = pointer;
    // Hover a link: show its target, unless the context menu is opening or open: a tooltip draws
    // above the menu and would cover its first items (#3368).
    if resp.hovered()
        && !app.canvas.dragging
        && !app.canvas.context_menu_open
        && !resp.secondary_clicked()
        && !ui.input(|i| i.pointer.secondary_down())
        && let straight = straight_box(app, layout)
        && let Some(pos) = layout.hit_shown(*page, *x, *y, *story, straight)
        && let shown = layout.shown_point(*page, *x, *y, *story, straight)
        && let Some(link) = link_under(app, layout, *page, shown.x, shown.y, &pos)
    {
        let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
        let target = if is_generated_bookmark_link(&link) { tl!("Current Document").to_string() } else { link };
        let tip = format!("{target}\n{}", crate::i18n::fmt(tl!("{key}+Click to follow link"), &[("key", key)]));
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("link_tip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(tip);
        });
    }
}
