use super::select::{edit_float, min_size, movable_story, obj_size, select_object, selected, with_obj};
use super::{CmdError, CmdResult, Float, InlineObject, Pos, Selection, Session, StoryRef, Value, Wrap, json, p};

/// Largest object offset or move, points.
pub(super) const MAX_OFFSET: f32 = 4000.0;

/// Resize and/or move the selected object (dragging its frame or handles).
pub(super) fn bounds(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let to = match (p::f32(v, "x"), p::f32(v, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        (None, None) => None,
        _ => return Err(CmdError::Params("`x` and `y` go together".into())),
    };
    let in_text = text_target(v, &obj)?;
    if in_text.is_some() && to.is_some() {
        return Err(CmdError::Params("`to` or `x`/`y`, not both".into()));
    }
    let (w0, h0) = obj_size(&obj);
    let min = min_size(&obj);
    let w = p::f32(v, "width").unwrap_or(w0).clamp(min, MAX_OFFSET);
    let h = p::f32(v, "height").unwrap_or(h0).clamp(min, MAX_OFFSET);
    if (w, h) != (w0, h0) {
        with_obj(s, |o| o.set_size(w, h))?;
    }
    let pos = match (to, in_text) {
        (Some((x, y)), _) => move_object(s, pos, p::u64(v, "page"), x, y, (w, h))?,
        (None, Some(at)) => move_in_text(s, pos, &at)?,
        (None, None) => pos,
    };
    select_object(s, &pos);
    let o = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned();
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null), "pos": crate::cmd::pos_json(&pos)}))
}

/// The text position in `v`'s `to`, for an In Line with Text `obj` (a floating one is placed by
/// `x`/`y`); None without one.
fn text_target(v: &Value, obj: &InlineObject) -> Result<Option<Pos>, CmdError> {
    let Some(to) = v.get("to").filter(|t| !t.is_null()) else { return Ok(None) };
    if obj.is_floating() {
        return Err(CmdError::Params("`to` places an In Line with Text object; move a floating one by `x` and `y`".into()));
    }
    crate::cmd::parse_pos(to).map(Some).ok_or_else(|| CmdError::Params("bad `to`".into()))
}

/// The text position `to` an In Line with Text object at `pos` can land at: in the object's own
/// story (a table cell there too), not inside a field's code or such (it's clamped to a
/// character).
fn landing(s: &Session, pos: &Pos, to: &Pos) -> Result<Pos, CmdError> {
    if pos.path.depth() > 0 || !movable_story(s, pos.story) {
        return Err(CmdError::Disabled("only objects in the body text, headers and footers can be moved".into()));
    }
    if to.story != pos.story || s.doc.para_at(to).is_none() {
        return Err(CmdError::Params("`to` must be a position in the object's own text".into()));
    }
    Ok(s.doc.clamp(to))
}

/// Move the In Line with Text object at `pos` to the text position `to`, where it stays In Line
/// with Text, keeping its character formatting: Word's drag of an inline picture, which lands
/// where its dashed insertion cursor was (#2233, #2234). Dropped on itself, nothing changes.
/// Returns the object's position afterwards.
fn move_in_text(s: &mut Session, pos: Pos, to: &Pos) -> Result<Pos, CmdError> {
    let to = landing(s, &pos, to)?;
    let len = wordcraft_doc::para::OBJ.len_utf8();
    let same_para = to.path == pos.path;
    if same_para && (to.off == pos.off || to.off == pos.off + len) {
        return Ok(pos);
    }
    let para = s.doc.para_at(&pos).ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    let obj = para.object_at(pos.off).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    let props = para.props_of_char(pos.off).clone();
    s.doc.delete_range(&pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    let mut at = to;
    if same_para && at.off > pos.off {
        at.off = at.off.saturating_sub(len);
    }
    let at = s.doc.clamp(&at);
    s.doc.insert_object(&at, obj, &props)?;
    Ok(at)
}

/// Ctrl+drag: copy the selected object (a text box gets its own story) and move the copy to
/// (x, y), or for an In Line with Text object put it at the text position `to`, leaving the
/// original in place and the clipboard alone. The copy is selected.
pub(super) fn copy_to(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let len = wordcraft_doc::para::OBJ.len_utf8();
    let end = Pos { off: pos.off + len, ..pos.clone() };
    if let Some(to) = text_target(v, &obj)? {
        if p::f32(v, "x").is_some() || p::f32(v, "y").is_some() {
            return Err(CmdError::Params("`to` or `x`/`y`, not both".into()));
        }
        let at = landing(s, &pos, &to)?;
        let frag = s.doc.copy_range(&pos, &end);
        s.sel = Selection::caret(at.clone());
        crate::cmd::paste::insert(s, frag)?;
        // The copy starts where it was pasted.
        let copy = s.doc.clamp(&at);
        select_object(s, &copy);
        let o = s.doc.para_at(&copy).and_then(|p| p.object_at(copy.off)).cloned();
        return Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null), "pos": crate::cmd::pos_json(&copy)}));
    }
    if p::f32(v, "x").is_none() || p::f32(v, "y").is_none() {
        return Err(CmdError::Params("`x` and `y` are required".into()));
    }
    let frag = s.doc.copy_range(&pos, &end);
    s.sel = Selection::caret(end.clone());
    crate::cmd::paste::insert(s, frag)?;
    s.touch(); // lay out the copy before moving it
    // The copy sits right after the original; select it and move it like a drag would.
    s.sel = Selection { anchor: end.clone(), focus: Pos { off: end.off + len, ..end } };
    bounds(s, v)
}

/// Put the object at `pos` with its top-left at (x, y) on `page` (default: where it is), as a
/// drag does in Word: it floats (an inline object gets Square wrapping) and anchors to the
/// paragraph under its top edge, positioned relative to that paragraph's column and top. So the
/// text it lands on flows around it, and it moves with that text. On a page where no paragraph
/// starts, it anchors to the nearest line and is positioned on the page. Returns the object's
/// position afterwards.
///
/// A drop across pages can be thrown back: while the object sits on its old page it wraps text
/// away from the paragraph it is anchored to, so once it has moved, that paragraph can move back
/// onto the old page itself — and the object goes with it. Dropping near the top of the next
/// page does it every time (the anchor is that page's first paragraph, which fits on the old page
/// again once the object's wrap is gone). So a cross-page drop is anchored again until the object
/// really stays on the page asked for: the later passes see the pagination as it now is. A
/// same-page move (a resize, a nudge) places once, and a drop that still won't stay keeps its
/// last place rather than failing.
pub(super) fn move_object(s: &mut Session, pos: Pos, page: Option<u64>, x: f32, y: f32, (w, h): (f32, f32)) -> Result<Pos, CmdError> {
    let cur = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?.page;
    let target = page.map_or(cur, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let tries = if target == cur { 1 } else { 3 };
    let mut at = pos;
    for _ in 0..tries {
        at = move_object_once(s, &at, target, x, y, (w, h))?;
        s.touch();
        if s.layout().object(&at, target).is_some_and(|hit| hit.page == target) {
            break;
        }
    }
    Ok(at)
}

/// [`move_object`]'s one placement: move the object's character to the anchor under (x, y) on
/// `page` and offset it from there.
fn move_object_once(s: &mut Session, pos: &Pos, page: usize, x: f32, y: f32, (w, h): (f32, f32)) -> Result<Pos, CmdError> {
    use wordcraft_doc::para::Anchor;
    if pos.path.depth() > 0 || !movable_story(s, pos.story) {
        return Err(CmdError::Disabled("only objects in the body text, headers and footers can be moved".into()));
    }
    let story = pos.story;
    let layout = s.layout();
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    // Keep a corner of it on the page.
    let x = x.max(12.0 - w).min(pg.w - 12.0);
    let y = y.max(12.0 - h).min(pg.h.min(MAX_OFFSET) - 12.0);
    // The paragraph under the middle of its top edge: in the column it mostly lands in.
    let (anchor, on_page) = match anchor_paragraph(&s.doc, &layout, page, (x + w / 2.0, y), story) {
        Some(path) => (Pos { story, path, off: 0 }, false),
        None => (nearest_line(&layout, page, y, story).ok_or_else(|| CmdError::Params("no text on that page to anchor to".into()))?, true),
    };
    // Move its character to the anchor (unless it's already in that paragraph).
    let at = if anchor.story == pos.story && anchor.path == pos.path && !on_page {
        pos.clone()
    } else {
        let obj = s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
        let len = wordcraft_doc::para::OBJ.len_utf8();
        s.doc.delete_range(pos, &Pos { off: pos.off + len, ..pos.clone() })?;
        let mut at = anchor;
        if at.story == pos.story && at.path == pos.path && at.off > pos.off {
            at.off = at.off.saturating_sub(len);
        }
        let at = s.doc.clamp(&at);
        s.doc.insert_object(&at, obj, &wordcraft_doc::CharProps::default())?;
        at
    };
    // First on the page where it was dropped, so the text wraps around it as it will in the end.
    edit_float(s, &at, |f| {
        if f.wrap == Wrap::Inline {
            f.wrap = Wrap::Square;
        }
        f.h_rel = Anchor::Page;
        f.v_rel = Anchor::Page;
        f.h_align = None;
        f.v_align = None;
        f.x = x.clamp(-MAX_OFFSET, MAX_OFFSET);
        f.y = y.clamp(-MAX_OFFSET, MAX_OFFSET);
    })?;
    if on_page {
        return Ok(at);
    }
    // Then offsets from its paragraph, as laid out with the object there.
    s.touch();
    let origin = s.layout().object(&at, page).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?.origin;
    edit_float(s, &at, |f| {
        f.h_rel = Anchor::Column;
        f.v_rel = Anchor::Paragraph;
        f.x = (x - origin.x).clamp(-MAX_OFFSET, MAX_OFFSET);
        f.y = (y - origin.y).clamp(-MAX_OFFSET, MAX_OFFSET);
    })?;
    Ok(at)
}

/// Turn the selected floating object's alignment (left, centred, …) into an offset from its
/// column / paragraph that keeps it where it is, so it can be moved by an offset.
pub(super) fn unalign(s: &mut Session) -> Result<(), CmdError> {
    use wordcraft_doc::para::Anchor;
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let Some((_, _, float)) = obj.frame() else { return Ok(()) };
    if float.h_align.is_none() && float.v_align.is_none() {
        return Ok(());
    }
    let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
    let (ox, oy) = (hit.rect.x - hit.origin.x, hit.rect.y - hit.origin.y);
    edit_float(s, &pos, |f| {
        if f.h_align.is_some() {
            f.h_align = None;
            f.h_rel = Anchor::Column;
            f.x = ox.clamp(-MAX_OFFSET, MAX_OFFSET);
        }
        if f.v_align.is_some() {
            f.v_align = None;
            f.v_rel = Anchor::Paragraph;
            f.y = oy.clamp(-MAX_OFFSET, MAX_OFFSET);
        }
    })?;
    Ok(())
}

/// Whether a floating object moves with its text (Word's "Move with Text"): its vertical position
/// follows its paragraph (as layout places any anchor but the page and its margins), rather than
/// being fixed on the page ("Fix Position on Page").
pub fn moves_with_text(f: &Float) -> bool {
    use wordcraft_doc::para::Anchor;
    !matches!(f.v_rel, Anchor::Page | Anchor::Margin | Anchor::TopMargin | Anchor::BottomMargin | Anchor::InsideMargin | Anchor::OutsideMargin)
}

/// Arrange › Wrap Text › Move with Text / Fix Position on Page: anchor the selected floating
/// object's vertical position to its paragraph or to the page, keeping it where it is now.
pub(super) fn move_with_text(s: &mut Session, with_text: bool) -> CmdResult {
    use wordcraft_doc::para::Anchor;
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let Some((_, _, float)) = obj.frame() else { return Err(CmdError::Disabled("select a floating picture or shape first".into())) };
    if moves_with_text(float) == with_text {
        return Ok(json!({"object": serde_json::to_value(&obj).unwrap_or(Value::Null)}));
    }
    let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
    // Paragraph offsets start at the paragraph's top (the hit's origin), page offsets at the page's.
    let y = if with_text { hit.rect.y - hit.origin.y } else { hit.rect.y };
    let o = edit_float(s, &pos, |f| {
        f.v_rel = if with_text { Anchor::Paragraph } else { Anchor::Page };
        f.v_align = None;
        f.y = y.clamp(-MAX_OFFSET, MAX_OFFSET);
    })?;
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null)}))
}

/// The top-level paragraph of `story` under `(x, y)` on `page`: in the column under `x` (from
/// the section's column geometry), the last one starting at or above `y` (else the first
/// starting in that column; else, with no paragraph starting in that column, the last starting
/// at or above `y` anywhere on the page, else the first). Body paragraphs come from the page's
/// items; header/footer ones from its header and footer. The column matters (#884): layout
/// places a floating object when it lays out its anchor, so text laid out before that (an
/// earlier column) can't wrap around it. (Nor do lines continuing a paragraph from the column
/// before: layout breaks them once, in the column the paragraph starts in.)
pub(crate) fn anchor_paragraph(
    doc: &wordcraft_doc::Document,
    layout: &wordcraft_layout::DocLayout,
    page: usize,
    (x, y): (f32, f32),
    story: StoryRef,
) -> Option<wordcraft_doc::Path> {
    let pg = layout.pages.get(page)?;
    let items: Box<dyn Iterator<Item = &wordcraft_layout::Placed> + '_> = match story {
        StoryRef::Body => Box::new(pg.items.iter()),
        StoryRef::Part(_) => Box::new(pg.header.iter().chain(pg.footer.iter())),
    };
    // Top-level text on the page: (path, left, top, starts the paragraph).
    let pieces: Vec<(&wordcraft_doc::Path, f32, f32, bool)> = items
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Lines { story: s, path, l0, x, y: top, .. } if *s == story && path.depth() == 0 => {
                Some((path, *x, *top, *l0 == 0))
            }
            _ => None,
        })
        .collect();
    let starts: Vec<(&wordcraft_doc::Path, f32)> = pieces.iter().filter(|p| p.3).map(|p| (p.0, p.2)).collect();
    // The page's columns (left, right), from its section's column geometry (headers and footers
    // span the text area); the column under `x` is the rightmost one starting at or left of it.
    let boxes = match story {
        StoryRef::Body => doc.laid_out_sections().get(pg.section).map(|(_, sp)| sp.column_boxes()).unwrap_or_default(),
        StoryRef::Part(_) => Vec::new(),
    };
    let mut cols: Vec<(f32, f32)> =
        boxes.iter().map(|(cx, w)| (pg.body.x + cx, pg.body.x + cx + w)).filter(|c| c.0.is_finite() && c.1.is_finite()).collect();
    cols.sort_by(|a, b| a.0.total_cmp(&b.0));
    let col_of = |px: f32| cols.iter().rposition(|c| c.0 <= px + 0.5).unwrap_or(0);
    let col = col_of(x);
    let in_col: Vec<(&wordcraft_doc::Path, f32)> = pieces.iter().filter(|p| p.3 && col_of(p.1) == col).map(|p| (p.0, p.2)).collect();
    let pick = |v: &[(&wordcraft_doc::Path, f32)]| v.iter().rev().find(|(_, top)| *top <= y + 0.5).or(v.first()).map(|(p, _)| (*p).clone());
    pick(&in_col).or_else(|| pick(&starts))
}

/// The start of the top-level line of `story` on `page` nearest to `y`.
pub(crate) fn nearest_line(layout: &wordcraft_layout::DocLayout, page: usize, y: f32, story: StoryRef) -> Option<Pos> {
    let lines = wordcraft_layout::hit::page_lines(layout.pages.get(page)?, story);
    let best = lines.iter().filter(|l| l.path.depth() == 0).min_by(|a, b| {
        let d = |l: &wordcraft_layout::hit::LineHit| if y < l.top { l.top - y } else { (y - l.bottom).max(0.0) };
        d(a).total_cmp(&d(b))
    })?;
    let off = best.para.lines.get(best.li)?.start;
    Some(Pos { story, path: best.path.clone(), off })
}
