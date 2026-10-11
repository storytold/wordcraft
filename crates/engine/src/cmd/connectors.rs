//! Connectors: gluing a connector's ends to shapes' connection sites (`shape.connect`), and
//! re-routing glued connectors when the shapes they join move or change size ([`reroute`], run by
//! the session after object edits).
//!
//! Floating connectors in the body are routed on the page (from the layout); connectors inside a
//! group, in the group's own space. A glue to a shape that is gone (or on another page) leaves
//! that end where it is.

use std::collections::HashMap;

use serde_json::{Value, json};
use wordcraft_doc::connector::{ConnEnd, Frame, ends, nearest_site, route, site_point};
use wordcraft_doc::para::{Anchor, InlineObject, Wrap};
use wordcraft_doc::{Pos, StoryRef};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// How close (points) a dragged end must come to a connection site to glue to it.
pub const SNAP: f32 = 12.0;
/// Largest offset a routed connector gets, points.
const MAX_OFFSET: f32 = 4000.0;

fn has_connector(s: &Session) -> Option<&'static str> {
    match super::objects::selected(s) {
        Some((_, InlineObject::Shape { kind, .. })) if kind.is_connector() => None,
        _ => Some("select a connector first"),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("shape.connect", "Connect Shapes", "Shape Format › Insert Shapes", connect)
            .params(
                r#"{"start"?: end, "end"?: end}  end: {"path": [..], "off": n, "site"?: n} (glue to that body shape's connection site; nearest one when omitted) | {"x": pt, "y": pt, "page"?: n} (drag the end there: it glues to a shape's site within 12 pt, else stays free there) | null (unglue)"#,
            )
            .when(has_connector),
    ]
}

/// Commands after which glued connectors are re-routed: those that move, resize, turn, add or
/// change drawings.
pub fn reroutes_after(id: &str) -> bool {
    id.starts_with("arrange.") || id.starts_with("shape.") || id.starts_with("picture.") || id == "insert.shape" || id.starts_with("draw.")
}

/// The body's floating drawings with an id (by id) and its glued connectors.
fn scan(s: &Session) -> (HashMap<u32, Pos>, Vec<Pos>, Vec<Pos>) {
    let (mut ids, mut conns, mut groups) = (HashMap::new(), Vec::new(), Vec::new());
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(para) = s.doc.para(StoryRef::Body, &path) else { continue };
        for off in para.object_offsets() {
            let pos = Pos { story: StoryRef::Body, path: path.clone(), off };
            match para.object_at(off) {
                Some(InlineObject::Shape { kind, extra, float, .. }) => {
                    if extra.id != 0 {
                        ids.entry(extra.id).or_insert(pos.clone());
                    }
                    if kind.is_connector() && float.wrap != Wrap::Inline && (extra.start.is_some() || extra.end.is_some()) {
                        conns.push(pos);
                    }
                }
                Some(InlineObject::Group { children, .. })
                    if children.iter().any(|c| matches!(&c.obj, InlineObject::Shape { kind, extra, .. } if kind.is_connector() && (extra.start.is_some() || extra.end.is_some()))) =>
                {
                    groups.push(pos)
                }
                _ => {}
            }
        }
    }
    (ids, conns, groups)
}

/// Re-route every glued connector to the shapes it joins. Returns whether anything moved.
pub fn reroute(s: &mut Session) -> bool {
    let (ids, conns, groups) = scan(s);
    let mut moved = false;
    for pos in groups {
        if let Ok(para) = s.doc.para_mut(pos.story, &pos.path)
            && let Some(InlineObject::Group { children, .. }) = para.object_at_mut(pos.off)
            && wordcraft_doc::connector::reroute_group(children)
        {
            para.touch();
            moved = true;
        }
    }
    if conns.is_empty() || ids.is_empty() {
        return moved;
    }
    if moved {
        s.touch();
    }
    let layout = s.layout();
    let site = |e: Option<ConnEnd>, page: usize| -> Option<(f32, f32)> {
        let e = e?;
        let target = ids.get(&e.id)?;
        let hit = layout.object(target, page)?;
        let kind = match s.doc.para_at(target)?.object_at(target.off)? {
            InlineObject::Shape { kind, .. } => *kind,
            _ => return None,
        };
        (hit.page == page).then(|| site_point(kind, [hit.rect.x, hit.rect.y, hit.rect.w, hit.rect.h], hit.spin, e.site))?
    };
    let mut plans = Vec::new();
    for pos in conns {
        let Some(hit) = layout.object(&pos, s.page_hint) else { continue };
        let Some(InlineObject::Shape { extra, float, .. }) = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)) else { continue };
        let r = hit.rect;
        let (s0, e0) = ends(r.x, r.y, r.w, r.h, float.flip_h, float.flip_v);
        let to = route(site(extra.start, hit.page).unwrap_or(s0), site(extra.end, hit.page).unwrap_or(e0));
        plans.push((pos, to, (r.x, r.y), (hit.origin.x, hit.origin.y)));
    }
    for (pos, to, at, origin) in plans {
        moved |= place(s, &pos, to, at, origin);
    }
    moved
}

/// Put the floating connector at `pos` (now with its frame's top-left at page point `at`, its
/// anchor's origin at `origin`) in frame `to`. Returns whether it changed.
fn place(s: &mut Session, pos: &Pos, to: Frame, at: (f32, f32), origin: (f32, f32)) -> bool {
    let Ok(para) = s.doc.para_mut(pos.story, &pos.path) else { return false };
    let Some(InlineObject::Shape { w, h, float, .. }) = para.object_at_mut(pos.off) else { return false };
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    if near(to.x, at.0)
        && near(to.y, at.1)
        && near(to.w, *w)
        && near(to.h, *h)
        && to.flip_h == float.flip_h
        && to.flip_v == float.flip_v
        && float.rot == 0.0
    {
        return false;
    }
    let clamp = |v: f32| wordcraft_geom::finite(v).clamp(-MAX_OFFSET, MAX_OFFSET);
    // An aligned position becomes an offset from the column / paragraph (where `origin` is).
    if float.h_align.take().is_some() {
        float.h_rel = Anchor::Column;
        float.x = clamp(to.x - origin.0);
    } else {
        float.x = clamp(float.x + to.x - at.0);
    }
    if float.v_align.take().is_some() {
        float.v_rel = Anchor::Paragraph;
        float.y = clamp(to.y - origin.1);
    } else {
        float.y = clamp(float.y + to.y - at.1);
    }
    *w = to.w.clamp(0.0, MAX_OFFSET);
    *h = to.h.clamp(0.0, MAX_OFFSET);
    float.rot = 0.0;
    float.flip_h = to.flip_h;
    float.flip_v = to.flip_v;
    para.touch();
    true
}

/// A body position from `{"path": [..], "off": n}` (or any form `parse_pos` reads).
fn body_pos(e: &Value) -> Option<Pos> {
    let path: Option<Vec<u32>> =
        e.get("path").and_then(Value::as_array).map(|a| a.iter().take(64).filter_map(Value::as_u64).filter_map(|x| u32::try_from(x).ok()).collect());
    match (path, p::u64(e, "off").and_then(|o| usize::try_from(o).ok())) {
        (Some(path), Some(off)) if !path.is_empty() => Some(Pos { story: StoryRef::Body, path: wordcraft_doc::Path(path), off }),
        _ => super::parse_pos(e).filter(|t| t.story == StoryRef::Body),
    }
}

/// Give the drawing at `pos` a drawing id if it has none; returns it.
fn ensure_id(s: &mut Session, pos: &Pos) -> Result<u32, CmdError> {
    let next = s.doc.next_shape_id();
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    let (id, fresh) = match para.object_at_mut(pos.off) {
        Some(InlineObject::Shape { extra, kind, .. }) if !kind.is_connector() => {
            let fresh = extra.id == 0;
            if fresh {
                extra.id = next;
            }
            (extra.id, fresh)
        }
        _ => return Err(CmdError::Params("connectors glue to shapes and text boxes".into())),
    };
    if fresh {
        para.touch();
    }
    Ok(id)
}

/// `shape.connect`: glue, move or unglue the selected connector's ends.
fn connect(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, obj) = super::objects::selected(s).ok_or_else(|| CmdError::Disabled("select a connector first".into()))?;
    let InlineObject::Shape { kind, float, .. } = &obj else { return Err(CmdError::Disabled("select a connector first".into())) };
    if !kind.is_connector() {
        return Err(CmdError::Disabled("select a connector first".into()));
    }
    if pos.story != StoryRef::Body || float.wrap == Wrap::Inline {
        return Err(CmdError::Disabled("only floating connectors in the body text can be glued".into()));
    }
    let (fh, fv) = (float.flip_h, float.flip_v);
    let layout = s.layout();
    let hit = layout.object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the connector isn't laid out".into()))?;
    let r = hit.rect;
    let (mut p0, mut p1) = ends(r.x, r.y, r.w, r.h, fh, fv);
    let mut glue: [Option<Option<ConnEnd>>; 2] = [None, None];
    for (i, key) in ["start", "end"].into_iter().enumerate() {
        let Some(e) = v.get(key) else { continue };
        if e.is_null() {
            glue[i] = Some(None);
            continue;
        }
        let here = if i == 0 { p0 } else { p1 };
        let (target, site) = if e.get("path").is_some() {
            let at = body_pos(e).ok_or_else(|| CmdError::Params(format!("`{key}`: no shape there")))?;
            let th = layout
                .object(&at, hit.page)
                .filter(|t| t.page == hit.page)
                .ok_or_else(|| CmdError::Params(format!("`{key}`: no shape there on this page")))?;
            let tkind = match s.doc.para_at(&at).and_then(|p| p.object_at(at.off)) {
                Some(InlineObject::Shape { kind, .. }) if !kind.is_connector() => *kind,
                _ => return Err(CmdError::Params(format!("`{key}`: connectors glue to shapes and text boxes"))),
            };
            let rect = [th.rect.x, th.rect.y, th.rect.w, th.rect.h];
            let site = match p::u64(e, "site") {
                Some(n) => u32::try_from(n)
                    .ok()
                    .filter(|n| site_point(tkind, rect, th.spin, *n).is_some())
                    .ok_or_else(|| CmdError::Params(format!("`{key}`: no such connection site")))?,
                None => nearest_site(tkind, rect, th.spin, here).map(|(n, ..)| n).unwrap_or(0),
            };
            (Some(at), site)
        } else {
            let (x, y) = (p::f32(e, "x").ok_or_else(|| CmdError::Params(format!("`{key}`: a shape or a point")))?, p::f32(e, "y").unwrap_or(here.1));
            let page = p::u64(e, "page").map_or(hit.page, |n| usize::try_from(n).unwrap_or(usize::MAX));
            if page != hit.page {
                return Err(CmdError::Params("a connector's ends stay on its page".into()));
            }
            // The nearest site of a shape (not a connector) within reach.
            let mut best: Option<(Pos, u32, f32)> = None;
            if let Some(pg) = layout.pages.get(page) {
                for it in &pg.items {
                    let wordcraft_layout::Placed::Object { rect, spin, story: StoryRef::Body, path, off, .. } = it else { continue };
                    let at = Pos { story: StoryRef::Body, path: path.clone(), off: *off };
                    if at == pos {
                        continue;
                    }
                    let Some(InlineObject::Shape { kind, .. }) = s.doc.para_at(&at).and_then(|p| p.object_at(at.off)) else { continue };
                    if kind.is_connector() {
                        continue;
                    }
                    if let Some((n, _, d)) = nearest_site(*kind, [rect.x, rect.y, rect.w, rect.h], *spin, (x, y))
                        && d <= SNAP
                        && best.as_ref().is_none_or(|b| d < b.2)
                    {
                        best = Some((at, n, d));
                    }
                }
            }
            match best {
                Some((at, n, _)) => (Some(at), n),
                None => {
                    // A free end, moved there.
                    if i == 0 {
                        p0 = (x, y);
                    } else {
                        p1 = (x, y);
                    }
                    glue[i] = Some(None);
                    continue;
                }
            }
        };
        if let Some(at) = target {
            let id = ensure_id(s, &at)?;
            glue[i] = Some(Some(ConnEnd { id, site }));
        }
    }
    // Free ends first, then glued ones are routed.
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    if let Some(InlineObject::Shape { extra, .. }) = para.object_at_mut(pos.off) {
        if let Some(g) = glue[0] {
            extra.start = g;
        }
        if let Some(g) = glue[1] {
            extra.end = g;
        }
        para.touch();
    }
    place(s, &pos, route(p0, p1), (r.x, r.y), (hit.origin.x, hit.origin.y));
    s.touch();
    reroute(s);
    let o = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned();
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null)}))
}
