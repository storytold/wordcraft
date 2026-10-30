use super::object::ink_name;
use super::{
    AltText, Anchor, Fill, Float, GradientFlip, GradientKind, InlineObject, PartRels, PathShape, Rgb, ShapeEffects, ShapeKind, W, Wrap, Writer, emu,
    n, rt,
};

pub(super) struct DrawingInfo<'a> {
    pub float: &'a Float,
    pub size: (f32, f32),
    pub docpr: &'a str,
    pub name: &'a str,
    pub alt: &'a str,
}

impl Writer<'_> {
    /// A picture's `pic:pic` (the image in `media/{file}`), at `off` in its group (or 0, 0).
    pub(super) fn pic(&mut self, w: &mut W, o: &InlineObject, file: &str, off: (f32, f32), rels: &mut PartRels) {
        let InlineObject::Image { media, w: iw, h: ih, alt, crop, float, shape, .. } = o else { return };
        let rid = rels.add(rt::IMAGE, &format!("media/{file}"), false);
        self.used_media.insert(media.clone());
        w.open("pic:pic", &[]);
        w.open("pic:nvPicPr", &[]);
        nv_props(w, "pic:cNvPr", "0", file, alt, &float.alt, float.hidden);
        w.open("pic:cNvPicPr", &[]);
        w.empty("a:picLocks", &[("noChangeAspect", "1"), ("noChangeArrowheads", "1")]);
        w.close("pic:cNvPicPr");
        w.close("pic:nvPicPr");
        w.open("pic:blipFill", &[]);
        w.empty("a:blip", &[("r:embed", &rid)]);
        // Negative values pad the picture (Fit): ECMA-376 §20.1.8.55 allows them.
        if crop.iter().any(|c| wordcraft_geom::finite(*c) != 0.0) {
            let c = |v: f32| n((wordcraft_geom::finite(v).clamp(wordcraft_doc::para::MIN_CROP, 1.0) * 100_000.0).round() as i64);
            w.empty("a:srcRect", &[("l", &c(crop[0])), ("t", &c(crop[1])), ("r", &c(crop[2])), ("b", &c(crop[3]))]);
        }
        w.open("a:stretch", &[]);
        w.empty("a:fillRect", &[]);
        w.close("a:stretch");
        w.close("pic:blipFill");
        w.open("pic:spPr", &[("bwMode", "auto")]);
        xfrm(w, off, *iw, *ih, float.spin());
        // Crop to Shape: the outline the picture is clipped to.
        // A preset read from a file is written back as it was, adjust values included.
        let (prst, av) = match shape {
            Some(s) if !s.prst.is_empty() => (s.prst.as_str(), s.av.as_slice()),
            Some(s) => (crate::read::shape_style::preset_name(s.kind), s.av.as_slice()),
            None => ("rect", &[][..]),
        };
        w.open("a:prstGeom", &[("prst", prst)]);
        if av.is_empty() {
            w.empty("a:avLst", &[]);
        } else {
            w.open("a:avLst", &[]);
            for (name, fmla) in av {
                w.empty("a:gd", &[("name", name), ("fmla", fmla)]);
            }
            w.close("a:avLst");
        }
        w.close("a:prstGeom");
        w.close("pic:spPr");
        w.close("pic:pic");
    }

    /// A shape's or text box's `wps:wsp`, at `off` in its group (or 0, 0). In a group it carries
    /// its own `wps:cNvPr` with drawing id `id`.
    pub(super) fn wsp(&mut self, w: &mut W, o: &InlineObject, id: Option<&str>, off: (f32, f32), rels: &mut PartRels, depth: usize) {
        let InlineObject::Shape { kind, w: sw, h: sh, fill, stroke, stroke_width, story, effects, float, freeform, extra } = o else { return };
        let geom = freeform.as_deref().filter(|_| *kind == ShapeKind::Freeform);
        w.open("wps:wsp", &[]);
        if let Some(id) = id {
            let name = shape_name(*kind, freeform.as_deref(), id);
            nv_props(w, "wps:cNvPr", id, &name, &float.alt.text, &float.alt, float.hidden);
        }
        if kind.is_connector() {
            // A connector's glued ends (ECMA-376 §20.1.2.2.8 `cNvCxnSpPr`).
            let ends = [("a:stCxn", extra.start), ("a:endCxn", extra.end)];
            if ends.iter().any(|(_, e)| e.is_some()) {
                w.open("wps:cNvCnPr", &[]);
                for (tag, e) in ends {
                    if let Some(e) = e {
                        w.empty(tag, &[("id", &e.id.to_string()), ("idx", &e.site.to_string())]);
                    }
                }
                w.close("wps:cNvCnPr");
            } else {
                w.empty("wps:cNvCnPr", &[]);
            }
        } else if *kind == ShapeKind::TextBox {
            w.empty("wps:cNvSpPr", &[("txBox", "1")]);
        } else {
            w.empty("wps:cNvSpPr", &[]);
        }
        w.open("wps:spPr", &[]);
        xfrm(w, off, *sw, *sh, float.spin());
        let prst = crate::read::shape_style::preset_name(*kind);
        match geom {
            Some(f) => cust_geom(w, f, *sw, *sh),
            None => {
                w.open("a:prstGeom", &[("prst", prst)]);
                match extra.corner.filter(|_| *kind == ShapeKind::RoundedRectangle) {
                    Some(c) => {
                        w.open("a:avLst", &[]);
                        let fmla = format!("val {}", c.min(wordcraft_doc::connector::MAX_CORNER));
                        w.empty("a:gd", &[("name", "adj"), ("fmla", &fmla)]);
                        w.close("a:avLst");
                    }
                    None => w.empty("a:avLst", &[]),
                }
                w.close("a:prstGeom");
            }
        }
        let alpha = geom.map_or(1.0, |f| f.alpha);
        let ink = geom.is_some_and(|f| f.is_ink());
        write_fill(w, fill.as_ref().filter(|_| !ink && !kind.is_open()));
        match stroke {
            Some(c) => {
                let width = emu(stroke_width.clamp(0.0, 100.0));
                let cap = if ink { "rnd" } else { extra.cap.ooxml() };
                let ln: &[(&str, &str)] = if ink || !extra.cap.is_flat() { &[("w", width.as_str()), ("cap", cap)] } else { &[("w", width.as_str())] };
                w.open("a:ln", ln);
                // An outline gradient read from a file, while the outline keeps its colour (#1500).
                let grad = extra.outline_fill.as_ref().filter(|f| !ink && f.main_color() == *c);
                if grad.is_some() {
                    write_fill(w, grad);
                } else {
                    w.open("a:solidFill", &[]);
                    if alpha < 1.0 {
                        w.open("a:srgbClr", &[("val", &c.hex())]);
                        w.empty("a:alpha", &[("val", &n((alpha.clamp(0.0, 1.0) * 100_000.0).round() as i64))]);
                        w.close("a:srgbClr");
                    } else {
                        w.empty("a:srgbClr", &[("val", &c.hex())]);
                    }
                    w.close("a:solidFill");
                }
                line_dash(w, &extra.dash);
                if ink {
                    w.empty("a:round", &[]);
                }
                arrow_ends(w, extra);
                w.close("a:ln");
            }
            None => {
                let cap = extra.cap.ooxml();
                let ln: &[(&str, &str)] = if extra.cap.is_flat() { &[] } else { &[("cap", cap)] };
                w.open("a:ln", ln);
                w.empty("a:noFill", &[]);
                line_dash(w, &extra.dash);
                w.close("a:ln");
            }
        }
        effect_list(w, effects);
        w.close("wps:spPr");
        // Its text, within the same bounds layout shows boxes inside boxes with. A box linked
        // from another (Create Link) shows the rest of that box's text: it names the chain and
        // its place in it instead.
        let part = (*story).and_then(|id| self.doc.parts.get(&id).filter(|p| p.kind == wordcraft_doc::PartKind::TextBox).map(|p| (id, p)));
        let link = part.and_then(|(id, _)| self.txbx_links.get(&id).copied());
        if let Some((n, seq)) = link.filter(|(_, seq)| *seq > 0) {
            w.empty("wps:linkedTxbx", &[("id", &n.to_string()), ("seq", &seq.to_string())]);
        } else if let Some((id, part)) = part
            && self.boxes.enter(id)
        {
            match link {
                Some((n, _)) => w.open("wps:txbx", &[("id", &n.to_string())]),
                None => w.open("wps:txbx", &[]),
            }
            w.open("w:txbxContent", &[]);
            if self.written_boxes.insert(id) {
                let blocks = part.blocks.clone();
                self.blocks(w, &blocks, rels, false, depth + 1);
            } else {
                w.empty("w:p", &[]);
            }
            w.close("w:txbxContent");
            w.close("wps:txbx");
            self.boxes.leave();
        }
        // Text direction and alignment, when not the defaults.
        let body = part.map(|(_, p)| p.body).unwrap_or_default();
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if body.vert != wordcraft_doc::props::TextVert::Horz {
            attrs.push(("vert", body.vert.ooxml()));
        }
        if body.anchor != wordcraft_doc::props::VAlign::Top {
            attrs.push(("anchor", wordcraft_doc::props::anchor_ooxml(body.anchor)));
        }
        // Internal margins, when the box has its own.
        let inset = body.inset.map(|i| i.map(|v| v.min(wordcraft_doc::MAX_TEXT_BOX_INSET).to_string()));
        if let Some(inset) = &inset {
            for (n, v) in ["lIns", "tIns", "rIns", "bIns"].into_iter().zip(inset) {
                attrs.push((n, v));
            }
        }
        let warp = extra.warp.as_ref().and_then(|t| t.sanitized());
        if warp.is_none() && !body.auto_fit {
            w.empty("wps:bodyPr", &attrs);
        } else {
            w.open("wps:bodyPr", &attrs);
            if let Some(warp) = &warp {
                text_warp(w, warp);
            }
            if body.auto_fit {
                w.empty("a:spAutoFit", &[]);
            }
            w.close("wps:bodyPr");
        }
        w.close("wps:wsp");
    }

    /// A `w:drawing` holding `graphic` (an `a:graphic`) in a frame of the object's size and place.
    pub(super) fn graphic_frame(&mut self, w: &mut W, info: DrawingInfo<'_>, graphic: &crate::xml::El) {
        let inline = info.float.wrap == Wrap::Inline;
        w.open("w:drawing", &[]);
        self.drawing_open(w, info);
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.raw(&graphic.to_xml());
        w.close(if inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
    }

    pub(super) fn drawing_open(&mut self, w: &mut W, info: DrawingInfo<'_>) {
        let DrawingInfo { float, size: (iw, ih), docpr, name, alt } = info;
        let (cx, cy) = (emu(iw.max(0.0)), emu(ih.max(0.0)));
        if float.wrap == Wrap::Inline {
            w.open("wp:inline", &[("distT", "0"), ("distB", "0"), ("distL", "0"), ("distR", "0")]);
        } else {
            let d = emu(float.dist.clamp(0.0, 1584.0));
            let (dt, db) = (emu(float.dist_top.clamp(0.0, 1584.0)), emu(float.dist_bottom.clamp(0.0, 1584.0)));
            // The stacking order read from the file is kept; objects without one stack in
            // document order above it.
            self.z = if float.z > 0 { float.z } else { self.z.saturating_add(1) };
            let behind = if float.wrap == Wrap::BehindText { "1" } else { "0" };
            w.open(
                "wp:anchor",
                &[
                    ("distT", &dt),
                    ("distB", &db),
                    ("distL", &d),
                    ("distR", &d),
                    ("simplePos", "0"),
                    ("relativeHeight", &self.z.to_string()),
                    ("behindDoc", behind),
                    ("locked", "0"),
                    ("layoutInCell", if float.outside_cell { "0" } else { "1" }),
                    ("allowOverlap", "1"),
                ],
            );
            w.empty("wp:simplePos", &[("x", "0"), ("y", "0")]);
            let rel = |a: Anchor, horiz: bool| match (a, horiz) {
                (Anchor::Margin, _) => "margin",
                (Anchor::Page, _) => "page",
                (Anchor::InsideMargin, _) => "insideMargin",
                (Anchor::OutsideMargin, _) => "outsideMargin",
                (Anchor::LeftMargin, true) => "leftMargin",
                (Anchor::RightMargin, true) => "rightMargin",
                (Anchor::Character, true) => "character",
                (Anchor::TopMargin, false) => "topMargin",
                (Anchor::BottomMargin, false) => "bottomMargin",
                (Anchor::Line, false) => "line",
                (_, true) => "column",
                (_, false) => "paragraph",
            };
            for (tag, horiz, a, rf, off) in
                [("wp:positionH", true, float.h_align, float.h_rel, float.x), ("wp:positionV", false, float.v_align, float.v_rel, float.y)]
            {
                w.open(tag, &[("relativeFrom", rel(rf, horiz))]);
                match a {
                    Some(a) => w.leaf("wp:align", &[], a.ooxml(horiz)),
                    None => w.leaf("wp:posOffset", &[], &emu(off)),
                }
                w.close(tag);
            }
        }
        w.empty("wp:extent", &[("cx", &cx), ("cy", &cy)]);
        // Word's effect extent also covers a rotated object's overhang (its rotated bounds).
        let (px, py) = float.spin_pad(iw, ih);
        let [el, et, er, eb] = float.effect_extent();
        let [el, et, er, eb] = [el + px, et + py, er + px, eb + py].map(emu);
        w.empty("wp:effectExtent", &[("l", &el), ("t", &et), ("r", &er), ("b", &eb)]);
        match float.wrap {
            Wrap::Inline => {}
            Wrap::Square => w.empty("wp:wrapSquare", &[("wrapText", float.wrap_text.ooxml())]),
            Wrap::Tight | Wrap::Through => {
                let tag = if float.wrap == Wrap::Tight { "wp:wrapTight" } else { "wp:wrapThrough" };
                w.open(tag, &[("wrapText", float.wrap_text.ooxml())]);
                // The object's own polygon, or its rectangle (the schema requires one).
                let s = wordcraft_doc::wrap::WRAP_SPACE;
                let (edited, pts) = match float.wrap_polygon.as_deref() {
                    Some(p) if p.points.len() >= 3 => (p.edited, p.points.clone()),
                    _ => (false, vec![[0, 0], [0, s], [s, s], [s, 0]]),
                };
                w.open("wp:wrapPolygon", &[("edited", if edited { "1" } else { "0" })]);
                // Closed: back to the start.
                for (i, [x, y]) in pts.iter().chain(pts.first()).enumerate() {
                    w.empty(if i == 0 { "wp:start" } else { "wp:lineTo" }, &[("x", &x.to_string()), ("y", &y.to_string())]);
                }
                w.close("wp:wrapPolygon");
                w.close(tag);
            }
            Wrap::TopAndBottom => w.empty("wp:wrapTopAndBottom", &[]),
            Wrap::BehindText | Wrap::InFrontOfText => w.empty("wp:wrapNone", &[]),
        }
        nv_props(w, "wp:docPr", docpr, name, alt, &float.alt, float.hidden);
    }
}

/// A shape's `cNvPr`/`docPr` name: ink is "Ink <pen> <id>" (and "#<order>" when its drawing
/// order is known), which reading looks for.
pub(super) fn shape_name(kind: ShapeKind, freeform: Option<&wordcraft_doc::freeform::Freeform>, id: &str) -> String {
    let f = freeform.filter(|_| kind == ShapeKind::Freeform);
    match (f.and_then(|f| f.ink), f.and_then(|f| f.order)) {
        // The drawing order rides along for Ink Replay: `Ink Pen 3 #12`.
        (Some(tool), Some(n)) => format!("Ink {} {id} #{n}", ink_name(tool)),
        (Some(tool), None) => format!("Ink {} {id}", ink_name(tool)),
        (None, _) => format!("Shape {id}"),
    }
}

/// A freeform's geometry as DrawingML custom geometry (`a:custGeom`): each path in EMUs of its
/// own coordinate space (the shape's size when it has none), from `a:moveTo` along `a:lnTo`.
fn cust_geom(w: &mut W, f: &wordcraft_doc::freeform::Freeform, sw: f32, sh: f32) {
    let (pw, ph) = (if f.w > 0.0 { f.w } else { sw.max(0.0) }, if f.h > 0.0 { f.h } else { sh.max(0.0) });
    w.open("a:custGeom", &[]);
    w.empty("a:avLst", &[]);
    w.empty("a:gdLst", &[]);
    w.empty("a:ahLst", &[]);
    w.empty("a:cxnLst", &[]);
    w.empty("a:rect", &[("l", "0"), ("t", "0"), ("r", "r"), ("b", "b")]);
    w.open("a:pathLst", &[]);
    let pt = |w: &mut W, tag: &str, [x, y]: [f32; 2]| {
        w.open(tag, &[]);
        w.empty("a:pt", &[("x", &emu(x)), ("y", &emu(y))]);
        w.close(tag);
    };
    for p in f.paths.iter().take(wordcraft_doc::freeform::MAX_PATHS) {
        let mut attrs = vec![("w", emu(pw)), ("h", emu(ph))];
        if !p.closed || f.is_ink() {
            attrs.push(("fill", "none".to_string()));
        }
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open("a:path", &attrs);
        for (i, xy) in p.pts.iter().take(wordcraft_doc::freeform::MAX_POINTS).enumerate() {
            pt(w, if i == 0 { "a:moveTo" } else { "a:lnTo" }, *xy);
        }
        // A tap: a zero-length line, so it shows as a dot.
        if let (1, Some(xy)) = (p.pts.len(), p.pts.first()) {
            pt(w, "a:lnTo", *xy);
        }
        if p.closed {
            w.empty("a:close", &[]);
        }
        w.close("a:path");
    }
    w.close("a:pathLst");
    w.close("a:custGeom");
}

/// Explicit DrawingML outline dashes, before joins and arrowheads.
fn line_dash(w: &mut W, dash: &wordcraft_doc::line_dash::LineDash) {
    use wordcraft_doc::line_dash::{LineDash, MAX_DASH_STOPS};
    if dash.is_solid() {
        return;
    }
    if let Some(preset) = dash.ooxml() {
        w.empty("a:prstDash", &[("val", preset)]);
        return;
    }
    if let LineDash::Custom(stops) = dash {
        if stops.is_empty() {
            return;
        }
        w.open("a:custDash", &[]);
        for (d, sp) in stops.iter().take(MAX_DASH_STOPS) {
            w.empty("a:ds", &[("d", &d.to_string()), ("sp", &sp.to_string())]);
        }
        w.close("a:custDash");
    }
}

/// A shape's `a:effectLst` (ECMA-376 Part 1 §20.1.8.26), children in schema order.
/// A line's arrowheads (`a:headEnd`, `a:tailEnd`, ECMA-376 §20.1.8.38 / §20.1.8.57).
fn arrow_ends(w: &mut W, extra: &wordcraft_doc::connector::ShapeExtra) {
    if extra.arrow_start {
        w.empty("a:headEnd", &[("type", "triangle")]);
    }
    if extra.arrow_end {
        w.empty("a:tailEnd", &[("type", "triangle")]);
    }
}

/// A preset text warp (`a:prstTxWarp`, ECMA-376 §20.1.9.19) with its adjust values.
fn text_warp(w: &mut W, warp: &wordcraft_doc::wordart::TextWarp) {
    w.open("a:prstTxWarp", &[("prst", &warp.preset)]);
    if warp.adj.is_empty() {
        w.empty("a:avLst", &[]);
    } else {
        w.open("a:avLst", &[]);
        for (name, v) in &warp.adj {
            w.empty("a:gd", &[("name", name), ("fmla", &format!("val {v}"))]);
        }
        w.close("a:avLst");
    }
    w.close("a:prstTxWarp");
}

fn effect_list(w: &mut W, effects: &ShapeEffects) {
    let fx = effects.sanitized();
    if fx.is_empty() {
        return;
    }
    let color = |w: &mut W, c: Rgb, transparency: f32| {
        let alpha = n(((100.0 - transparency.clamp(0.0, 100.0)) * 1000.0).round() as i64);
        w.open("a:srgbClr", &[("val", &c.hex())]);
        w.empty("a:alpha", &[("val", &alpha)]);
        w.close("a:srgbClr");
    };
    w.open("a:effectLst", &[]);
    if let Some(g) = fx.glow {
        w.open("a:glow", &[("rad", &emu(g.size))]);
        color(w, g.color, g.transparency);
        w.close("a:glow");
    }
    if let Some(s) = fx.shadow {
        let dir = n((s.angle * 60_000.0).round() as i64);
        // Anchor scaling at the corner the shadow falls away from (no scaling is written, so
        // this only matters to editors that resize it).
        let (dx, dy) = s.offset();
        let algn = match (dy > 0.01, dy < -0.01, dx > 0.01, dx < -0.01) {
            (true, _, true, _) => "tl",
            (true, _, _, true) => "tr",
            (true, _, _, _) => "t",
            (_, true, true, _) => "bl",
            (_, true, _, true) => "br",
            (_, true, _, _) => "b",
            (_, _, true, _) => "l",
            (_, _, _, true) => "r",
            _ => "ctr",
        };
        w.open(
            "a:outerShdw",
            &[
                ("blurRad", &emu(s.blur)),
                ("dist", &emu(s.distance)),
                ("dir", &dir),
                ("algn", algn),
                ("rotWithShape", if s.rot_with_shape { "1" } else { "0" }),
            ],
        );
        color(w, s.color, s.transparency);
        w.close("a:outerShdw");
    }
    if let Some(r) = fx.soft_edge {
        w.empty("a:softEdge", &[("rad", &emu(r))]);
    }
    w.close("a:effectLst");
}

fn xfrm(w: &mut W, (x, y): (f32, f32), cw: f32, ch: f32, spin: wordcraft_geom::Spin) {
    w.open("a:xfrm", &spin_attrs(spin).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
    w.empty("a:off", &[("x", &emu(x)), ("y", &emu(y))]);
    w.empty("a:ext", &[("cx", &emu(cw.max(0.0))), ("cy", &emu(ch.max(0.0)))]);
    w.close("a:xfrm");
}

/// An `a:xfrm` holding only a rotation and flips.
pub(super) fn xfrm_spin_only(w: &mut W, spin: wordcraft_geom::Spin) {
    w.empty("a:xfrm", &spin_attrs(spin).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
}

/// The `a:xfrm` attributes of a rotation and flips (ECMA-376 §20.1.7.6: `rot` in 60000ths of a
/// degree, clockwise), none for an unturned object.
pub(in super::super) fn spin_attrs(spin: wordcraft_geom::Spin) -> Vec<(&'static str, String)> {
    let mut a = Vec::new();
    let rot = (wordcraft_geom::normalize_degrees(spin.deg) as f64 * 60_000.0).round() as i64 % 21_600_000;
    if rot != 0 {
        a.push(("rot", rot.to_string()));
    }
    if spin.flip_h {
        a.push(("flipH", "1".to_string()));
    }
    if spin.flip_v {
        a.push(("flipV", "1".to_string()));
    }
    a
}

/// A drawing's `wp:docPr` (or a group member's `pic:cNvPr`/`wps:cNvPr`, ECMA-376
/// `CT_NonVisualDrawingProps`) with its alt text `descr`, its title, and the decorative mark as
/// the `adec:decorative` extension ([MS-ODRAWXML]). A description that is only the title (read
/// from a file with no description) is not written twice.
pub(in super::super) fn nv_props(w: &mut W, tag: &str, id: &str, name: &str, descr: &str, alt: &AltText, hidden: bool) {
    let mut attrs = vec![("id", id), ("name", name)];
    if descr != alt.title || descr.is_empty() {
        attrs.push(("descr", descr));
    }
    if !alt.title.is_empty() {
        attrs.push(("title", alt.title.as_str()));
    }
    if hidden {
        attrs.push(("hidden", "1"));
    }
    if !alt.decorative {
        w.empty(tag, &attrs);
        return;
    }
    w.open(tag, &attrs);
    w.open("a:extLst", &[]);
    w.open("a:ext", &[("uri", crate::xml::DECORATIVE_EXT)]);
    w.empty("adec:decorative", &[("xmlns:adec", crate::xml::DECORATIVE_NS), ("val", "1")]);
    w.close("a:ext");
    w.close("a:extLst");
    w.close(tag);
}

/// A shape's fill: `a:solidFill` (with `a:alpha` when transparent), `a:gradFill` with everything
/// read from the file, or `a:noFill`.
pub(super) fn write_fill(w: &mut W, f: Option<&Fill>) {
    let pct = |v: f32| n((v * 1000.0).round() as i64);
    let color = |w: &mut W, c: Rgb, transparency: f32| {
        if transparency > 0.0 {
            w.open("a:srgbClr", &[("val", &c.hex())]);
            w.empty("a:alpha", &[("val", &pct(100.0 - transparency))]);
            w.close("a:srgbClr");
        } else {
            w.empty("a:srgbClr", &[("val", &c.hex())]);
        }
    };
    let rect = |r: [f32; 4]| [("l", pct(r[0])), ("t", pct(r[1])), ("r", pct(r[2])), ("b", pct(r[3]))];
    match f.and_then(Fill::sanitized) {
        None => w.empty("a:noFill", &[]),
        Some(Fill::Solid { color: c, transparency }) => {
            w.open("a:solidFill", &[]);
            color(w, c, transparency);
            w.close("a:solidFill");
        }
        Some(Fill::Gradient(g)) => {
            // `rotWithShape` and `scaled` are written only when set: an absent one reads back off.
            let mut attrs = if g.rot_with_shape { vec![("rotWithShape", "1")] } else { vec![] };
            match g.flip {
                GradientFlip::None => {}
                GradientFlip::X => attrs.insert(0, ("flip", "x")),
                GradientFlip::Y => attrs.insert(0, ("flip", "y")),
                GradientFlip::Xy => attrs.insert(0, ("flip", "xy")),
            }
            w.open("a:gradFill", &attrs);
            w.open("a:gsLst", &[]);
            for s in &g.stops {
                w.open("a:gs", &[("pos", &pct(s.pos))]);
                color(w, s.color, s.transparency);
                w.close("a:gs");
            }
            w.close("a:gsLst");
            match g.kind {
                GradientKind::Linear { angle, scaled } => {
                    let ang = n((angle * 60_000.0).round() as i64);
                    let attrs: &[(&str, &str)] = if scaled { &[("ang", &ang), ("scaled", "1")] } else { &[("ang", &ang)] };
                    w.empty("a:lin", attrs)
                }
                GradientKind::Path { shape, focus } => {
                    let path = match shape {
                        PathShape::Circle => "circle",
                        PathShape::Rect => "rect",
                        PathShape::Shape => "shape",
                    };
                    w.open("a:path", &[("path", path)]);
                    let r = rect(focus);
                    w.empty("a:fillToRect", &r.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
                    w.close("a:path");
                }
            }
            if let Some(t) = g.tile {
                let r = rect(t);
                w.empty("a:tileRect", &r.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
            }
            w.close("a:gradFill");
        }
    }
}
