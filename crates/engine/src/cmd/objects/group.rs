use super::arrange::{MAX_OFFSET, move_object, unalign};
use super::select::{FROZEN, INK_GROUP, holds_ink, min_size, movable_story, object_ref, object_selection, picked_objects, selected, shown_members};
use super::*;

/// Add objects to the selection (Shift+click): the first one is selected if no object is yet;
/// one already selected is taken out again.
pub(super) fn add_object(s: &mut Session, v: &Value) -> CmdResult {
    let mut items: Vec<Value> = match v.get("objects").and_then(Value::as_array) {
        Some(a) => a.iter().take(1000).cloned().collect(),
        None => Vec::new(),
    };
    if let Some(p) = v.get("pos") {
        items.push(p.clone());
    }
    if let Some(i) = v.get("index") {
        items.push(i.clone());
    }
    if items.is_empty() {
        return Err(CmdError::Params("`pos`, `index` or `objects` required".into()));
    }
    for it in &items {
        let pos = object_ref(s, it)?;
        match object_selection(s).map(|(p, _)| p) {
            None => {
                s.also_selected.clear();
                s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos } };
            }
            Some(first) if first == pos => {}
            Some(_) => {
                if let Some(i) = s.also_selected.iter().position(|p| *p == pos) {
                    s.also_selected.remove(i);
                } else {
                    s.also_selected.push(pos);
                }
            }
        }
    }
    Ok(json!({"selected": picked_objects(s).iter().map(crate::cmd::pos_json).collect::<Vec<_>>()}))
}

/// Group the selected objects (Layout › Arrange › Group): one floating group object, anchored
/// where the first of them was, in the box around them all; they keep their places and sizes.
/// A group among them is merged in (its members join the new group).
pub(super) fn group(s: &mut Session, _: &Value) -> CmdResult {
    use wordcraft_doc::para::GroupChild;
    let mut picks = picked_objects(s);
    if picks.len() < 2 {
        return Err(CmdError::Disabled("select two or more pictures or shapes first".into()));
    }
    picks.sort();
    let layout = s.layout();
    let mut found = Vec::new();
    let story = picks.first().cloned().map(|p| p.story).ok_or_else(|| CmdError::Failed("nothing to group".into()))?;
    if picks.iter().any(|p| p.story != story) {
        return Err(CmdError::Disabled("objects in different stories (body, header, footer) can't be grouped".into()));
    }
    for pos in &picks {
        if pos.path.depth() > 0 || !movable_story(s, pos.story) {
            return Err(CmdError::Disabled("only objects in the body text, headers and footers (not in tables) can be grouped".into()));
        }
        let obj = s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
        if matches!(obj, InlineObject::Graphic { .. }) {
            return Err(CmdError::Disabled(FROZEN.into()));
        }
        if obj.is_canvas() {
            return Err(CmdError::Disabled("a drawing canvas can't be grouped".into()));
        }
        if !obj.is_floating() {
            return Err(CmdError::Disabled("objects in line with text can't be grouped: choose a text wrapping for them first".into()));
        }
        let hit = layout.object(pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
        found.push((obj, hit));
    }
    let page = found.first().map(|(_, h)| h.page).unwrap_or(0);
    if found.iter().any(|(_, h)| h.page != page) {
        return Err(CmdError::Disabled("objects on different pages can't be grouped".into()));
    }
    // The members as they show on the page: each one's unturned frame and turn (a group among
    // them gives its members, its turn taken into theirs).
    let mut shown: Vec<([f32; 4], Spin, &InlineObject)> = Vec::new();
    for (obj, hit) in &found {
        let r = hit.rect;
        match obj {
            InlineObject::Group { .. } => shown.extend(shown_members(obj, [r.x, r.y, r.w, r.h])),
            _ => shown.push(([r.x, r.y, r.w, r.h], hit.spin, obj)),
        }
    }
    // The group's box takes in what they cover, turned.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for ([x, y, w, h], spin, _) in &shown {
        let b = spin.bounds(wordcraft_geom::Rect::new(*x, *y, *w, *h));
        (x0, y0) = (x0.min(b.x), y0.min(b.y));
        (x1, y1) = (x1.max(b.right()), y1.max(b.bottom()));
    }
    if shown.is_empty() {
        return Err(CmdError::Failed("nothing to group".into()));
    }
    // `shown` already has the members of any picked group merged in, so this counts the
    // flattened members. Readers stop at the cap and would drop the rest.
    if shown.len() > wordcraft_doc::para::MAX_GROUP_CHILDREN {
        return Err(CmdError::Disabled(format!("too many objects to group (at most {})", wordcraft_doc::para::MAX_GROUP_CHILDREN)));
    }
    let (w, h) = ((x1 - x0).clamp(1.0, MAX_OFFSET), (y1 - y0).clamp(1.0, MAX_OFFSET));
    // Each member keeps its turn: the group itself starts unturned.
    let children: Vec<GroupChild> = shown
        .into_iter()
        .map(|([x, y, cw, ch], spin, obj)| {
            let mut obj = obj.clone();
            obj.set_size(cw, ch);
            if let Some(f) = obj.float_mut() {
                // Each member keeps its own alt text.
                let alt = std::mem::take(&mut f.alt);
                *f = Float { alt, ..Float::default() };
                f.set_spin(spin);
            }
            GroupChild { x: x - x0, y: y - y0, obj }
        })
        .collect();
    // The group takes the first member's placement; its members keep their own alt text.
    let mut float = found.first().and_then(|(o, _)| o.frame()).map(|(_, _, f)| Float { alt: Default::default(), ..f.clone() }).unwrap_or_default();
    float.set_spin(Spin::default());
    let grouped = InlineObject::Group { w, h, float, ch_w: w, ch_h: h, children, canvas: None };
    // Take the members out, last first so the earlier positions hold, and put the group where
    // the first one was.
    let len = wordcraft_doc::para::OBJ.len_utf8();
    for pos in picks.iter().rev() {
        s.doc.delete_range(pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    }
    let first = picks.first().cloned().ok_or_else(|| CmdError::Failed("nothing to group".into()))?;
    let at = s.doc.clamp(&first);
    s.doc.insert_object(&at, grouped, &wordcraft_doc::CharProps::default())?;
    s.touch();
    let pos = move_object(s, at, Some(page as u64), x0, y0, (w, h))?;
    s.also_selected.clear();
    s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + len, ..pos.clone() } };
    let o = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned();
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null), "pos": crate::cmd::pos_json(&pos)}))
}

/// Split the selected group back into its pictures and shapes (Layout › Arrange › Ungroup), in
/// the places and sizes the group shows them, anchored where the group was. They stay
/// selected, so Group puts them back together.
pub(super) fn ungroup(s: &mut Session, _: &Value) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("select a group first".into()))?;
    let InlineObject::Group { w, h, float, .. } = &obj else { return Err(CmdError::Disabled("select a group first".into())) };
    if holds_ink(&obj) {
        return Err(CmdError::Disabled(INK_GROUP.into()));
    }
    // Its offsets, explicit: the members are placed from the same anchor.
    let pos = if float.wrap == Wrap::Inline {
        if pos.path.depth() > 0 || !movable_story(s, pos.story) {
            return Err(CmdError::Disabled("only groups in the body text, headers and footers can be ungrouped in line with text".into()));
        }
        let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
        move_object(s, pos, Some(hit.page as u64), hit.rect.x, hit.rect.y, (*w, *h))?
    } else {
        unalign(s)?;
        pos
    };
    let obj = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    let Some((w, h, float)) = obj.frame() else { return Err(CmdError::Failed("object vanished".into())) };
    let float = float.clone();
    // Each member where the group shows it: the group's turn moves its centre about the group's
    // centre and adds to its own turn.
    let members: Vec<InlineObject> = shown_members(&obj, [0.0, 0.0, w, h])
        .into_iter()
        .map(|([x, y, cw, ch], spin, c)| {
            let mut c = c.clone();
            let (mw, mh) = (cw.max(min_size(&c)), ch.max(min_size(&c)));
            c.set_size(mw, mh);
            // Grown to its smallest size about its centre, so it stays centred where it was.
            let (x, y) = (x - (mw - cw) / 2.0, y - (mh - ch) / 2.0);
            if let Some(f) = c.float_mut() {
                // Each member keeps its own alt text; the group's placement is shared out.
                let alt = std::mem::take(&mut f.alt);
                *f = Float { x: (float.x + x).clamp(-MAX_OFFSET, MAX_OFFSET), y: (float.y + y).clamp(-MAX_OFFSET, MAX_OFFSET), alt, ..float.clone() };
                f.set_spin(spin);
            }
            c
        })
        .collect();
    if members.is_empty() {
        return Err(CmdError::Failed("the group is empty".into()));
    }
    let len = wordcraft_doc::para::OBJ.len_utf8();
    s.doc.delete_range(&pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    let mut at = s.doc.clamp(&pos);
    let mut placed = Vec::new();
    for m in members {
        placed.push(at.clone());
        at = s.doc.insert_object(&at, m, &wordcraft_doc::CharProps::default())?;
    }
    let first = placed.first().cloned().unwrap_or(pos);
    s.sel = Selection { anchor: first.clone(), focus: Pos { off: first.off + len, ..first } };
    s.also_selected = placed.into_iter().skip(1).collect();
    sel_result(s)
}
