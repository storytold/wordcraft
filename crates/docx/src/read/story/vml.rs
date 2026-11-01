use super::{
    Anchor, Arc, CharProps, El, Float, FloatAlign, InlineObject, MAX_STORY_DEPTH, MAX_VML_GROUP_DEPTH, Paragraph, PartKind, Reader, Rels, Rgb,
    ShapeExtra, ShapeKind, StoryCtx, TextColor, TextWarp, Wrap, int, measure, para_block, vml_color, vml_fill, vml_fraction, vml_preset_kind,
};

#[derive(Clone, Copy)]
struct VmlGroupSpace {
    origin: (f32, f32),
    scale: (f32, f32),
    depth: usize,
}

impl Reader<'_> {
    /// Every legacy VML drawing in one `w:pict`, in document order: a `w:pict` may hold any
    /// number of shapes (ECMA-376 Part 4, VML), and each one is its own object. A `v:group`
    /// becomes an [`InlineObject::Group`].
    pub(super) fn read_vml_all(&mut self, sc: &mut StoryCtx, pict: &El, rels: &Rels) -> Vec<InlineObject> {
        let mut out = Vec::new();
        for e in pict.els().filter(|e| e.name.starts_with("v:") && e.local() != "shapetype") {
            if out.len() >= wordcraft_doc::para::MAX_GROUP_CHILDREN {
                break;
            }
            if sc.in_hf
                && let Some(wm) = vml_watermark(e)
            {
                // The document's watermark (the first one found); drawn on every page, not as
                // a shape, so it is not one of the objects this picture contributes.
                self.doc.settings.watermark.get_or_insert(wm);
                continue;
            }
            let o = if e.local() == "group" { self.read_vml_group(sc, pict, e, rels) } else { self.read_vml_shape(sc, pict, e, rels, None) };
            out.extend(o);
        }
        out
    }

    /// The first VML drawing in `pict`, for the places that show a single one: an OLE object's
    /// picture and a `mc:Fallback` preview.
    pub(super) fn read_vml(&mut self, sc: &mut StoryCtx, pict: &El, rels: &Rels) -> Option<InlineObject> {
        self.read_vml_all(sc, pict, rels).into_iter().next()
    }

    /// A `v:group`: its own box from its `style`, its members in the space `coordsize` and
    /// `coordorigin` give it, nested groups flattened into it as [`Self::group_members`] does for
    /// DrawingML. `None` when it has no member we can show. `pict` is the picture the group sits
    /// in, where a member's `v:shapetype` is written.
    fn read_vml_group(&mut self, sc: &mut StoryCtx, pict: &El, grp: &El, rels: &Rels) -> Option<InlineObject> {
        let style = grp.attr("style").unwrap_or("");
        let len = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
        let (w, h) = (len("width"), len("height"));
        let float = vml_float(grp, style, sc.in_hf);
        // The members' space; without a usable `coordsize` it is the group's own box.
        let (ch_w, ch_h) = vml_coordpair(grp, "coordsize").filter(|(cw, ch)| *cw > 0.0 && *ch > 0.0).unwrap_or((w.max(1.0), h.max(1.0)));
        let (cx, cy) = vml_coordpair(grp, "coordorigin").unwrap_or((0.0, 0.0));
        let mut children = Vec::new();
        self.vml_group_members(sc, pict, grp, rels, VmlGroupSpace { origin: (-cx, -cy), scale: (1.0, 1.0), depth: 0 }, &mut children);
        if children.is_empty() {
            return None;
        }
        Some(InlineObject::Group { w, h, float, ch_w, ch_h, children, canvas: None })
    }

    /// The members of VML group `g` into `out`, mapped into the outermost group's space by `t`
    /// (x0, y0, sx, sy: outer = x0 + inner × sx). A member is placed by `left`/`top` in that
    /// space, except a `v:line`, which carries its own offset in `from`.
    fn vml_group_members(
        &mut self,
        sc: &mut StoryCtx,
        pict: &El,
        g: &El,
        rels: &Rels,
        space: VmlGroupSpace,
        out: &mut Vec<wordcraft_doc::para::GroupChild>,
    ) {
        let (x0, y0) = space.origin;
        let (sx, sy) = space.scale;
        let max = crate::units::MAX_LEN_PT;
        let fit = |v: f32, lo: f32| wordcraft_geom::finite(v).clamp(lo, max);
        for e in g.els() {
            if out.len() >= wordcraft_doc::para::MAX_GROUP_CHILDREN {
                return;
            }
            if !e.name.starts_with("v:") || e.local() == "shapetype" {
                continue;
            }
            let style = e.attr("style").unwrap_or("");
            let num = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).map_or(0.0, wordcraft_geom::finite);
            let ((ox, oy), (ew, eh)) = ((num("left"), num("top")), (num("width"), num("height")));
            if e.local() == "group" {
                if space.depth >= MAX_VML_GROUP_DEPTH {
                    continue;
                }
                // Its members' space maps onto its own box.
                let (cw, ch) = vml_coordpair(e, "coordsize").filter(|(cw, ch)| *cw > 0.0 && *ch > 0.0).unwrap_or((ew.max(1.0), eh.max(1.0)));
                let (cox, coy) = vml_coordpair(e, "coordorigin").unwrap_or((0.0, 0.0));
                let (kx, ky) = (ew / cw, eh / ch);
                let nt = (x0 + (ox - cox * kx) * sx, y0 + (oy - coy * ky) * sy, sx * kx, sy * ky);
                self.vml_group_members(sc, pict, e, rels, VmlGroupSpace { origin: (nt.0, nt.1), scale: (nt.2, nt.3), depth: space.depth + 1 }, out);
                continue;
            }
            let Some(mut obj) = self.read_vml_shape(sc, pict, e, rels, Some((sx, sy))) else { continue };
            // A member is placed by `x`/`y`, so a line's own offset comes off the member itself.
            let (dx, dy) = match obj.float_mut() {
                Some(f) => {
                    let d = (f.x, f.y);
                    (f.x, f.y) = (0.0, 0.0);
                    d
                }
                None => (0.0, 0.0),
            };
            out.push(wordcraft_doc::para::GroupChild { x: fit(x0 + (ox + dx) * sx, -max), y: fit(y0 + (oy + dy) * sy, -max), obj });
        }
    }

    /// One legacy VML picture, text box, line, rectangle, oval or preset shape (best effort).
    /// `pict` is the picture it sits in, where its `v:shapetype` is written -- for a group's
    /// member too, since Word writes each shapetype once, beside the group rather than inside
    /// it. `grp` scales a member from its group's space, and leaves placing it to the caller.
    fn read_vml_shape(&mut self, sc: &mut StoryCtx, pict: &El, shape: &El, rels: &Rels, grp: Option<(f32, f32)>) -> Option<InlineObject> {
        let style = shape.attr("style").unwrap_or("");
        let len = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
        let (sx, sy) = grp.unwrap_or((1.0, 1.0));
        let (mut w, mut h) = ((len("width") * sx).min(crate::units::MAX_LEN_PT), (len("height") * sy).min(crate::units::MAX_LEN_PT));
        // A member of a group never floats: the group does, and places it.
        let mut float = match grp {
            Some(_) => {
                let mut f = Float { hidden: vml_style(style, "visibility").is_some_and(|v| v.eq_ignore_ascii_case("hidden")), ..Float::default() };
                f.set_spin(vml_spin(style));
                f
            }
            None => vml_float(shape, style, sc.in_hf),
        };
        if let Some(img) = shape.find("v:imagedata") {
            let id = img.attr("r:id").or_else(|| img.attr("r:pict"))?;
            let media = self.media_for(rels, id)?;
            let alt = shape.attr("alt").or_else(|| img.attr("o:title")).unwrap_or("").to_string();
            return Some(InlineObject::Image { media, w, h, alt, float, crop: [0.0; 4], ole: None });
        }
        // The shapetype the shape is drawn from (its `type`, ECMA-376 Part 4 VML), when written in
        // the same picture: what the shape leaves out, it takes from there.
        let type_id = shape.attr("type").map(|t| t.trim().trim_start_matches('#'));
        let shapetype = type_id.and_then(|id| pict.els().find(|e| e.name == "v:shapetype" && e.attr("id") == Some(id)));
        let attr = |name: &str| shape.attr(name).or_else(|| shapetype.and_then(|t| t.attr(name)));
        let on = |name: &str| {
            attr(name).is_none_or(|v| {
                let t = v.trim();
                !t.eq_ignore_ascii_case("f") && !t.eq_ignore_ascii_case("false") && t != "0"
            })
        };
        let stroke = on("stroked").then(|| attr("strokecolor").map_or(Some(Rgb::BLACK), vml_color)).flatten();
        let stroke_width = attr("strokeweight").and_then(|v| measure(v, 1.0)).unwrap_or(0.75).clamp(0.0, 100.0);
        let fill = vml_fill(shape, shapetype);
        let endcap = shape
            .child("v:stroke")
            .and_then(|s| s.attr("endcap"))
            .or_else(|| shapetype.and_then(|t| t.child("v:stroke")).and_then(|s| s.attr("endcap")));
        let cap = match endcap {
            Some("square") => wordcraft_doc::connector::LineCap::Square,
            Some("round") => wordcraft_doc::connector::LineCap::Round,
            _ => wordcraft_doc::connector::LineCap::Flat,
        };
        let kind = match shape.local() {
            "line" => Some(ShapeKind::Line),
            "rect" => Some(ShapeKind::Rectangle),
            "roundrect" => Some(ShapeKind::RoundedRectangle),
            "oval" => Some(ShapeKind::Ellipse),
            // A preset, unless the shape's own path replaces its shapetype's geometry.
            "shape" if shape.attr("path").is_none() && shape.child("v:path").and_then(|p| p.attr("v")).is_none() => {
                vml_spt(shape, shapetype, type_id).and_then(vml_preset_kind)
            }
            _ => None,
        };
        // Word 2007 WordArt: a `v:shape` whose `v:textpath` draws its `string` (the header watermark
        // is taken before this). Shown as a text box holding that text, in the textpath's font and the
        // shape's `fillcolor`; the box itself has neither fill nor outline, as the letters are the art.
        if shape.local() == "shape"
            && let Some(tp) = shape.child("v:textpath")
            && !tp.attr("on").is_some_and(|v| matches!(v, "f" | "false" | "0"))
            && let Some(text) = tp.attr("string").map(|t| t.chars().take(200).collect::<String>()).filter(|t| !t.trim().is_empty())
            && sc.story_depth < MAX_STORY_DEPTH
        {
            let font = tp
                .attr("style")
                .and_then(|s| vml_style(s, "font-family"))
                .map(|f| css_string(f).chars().take(100).collect::<String>())
                .filter(|f| !f.is_empty());
            // The shapetype's WordArt shape (`o:spt` 136-175) bends the text like DrawingML's warp.
            let warp = vml_spt(shape, shapetype, type_id)
                .and_then(|spt| spt.checked_sub(136))
                .and_then(|i| VML_TEXT_WARPS.get(i as usize))
                .copied()
                .and_then(TextWarp::new);
            // The letters fill the box (`fitshape`): sized to its height.
            let props = CharProps {
                font,
                size: Some((h * 0.75).clamp(8.0, 200.0)),
                color: attr("fillcolor").and_then(vml_color).map(TextColor::Rgb),
                ..CharProps::default()
            };
            let id = self.doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&text, props))]);
            if let Some(part) = self.doc.parts.get_mut(&id) {
                part.body.anchor = wordcraft_doc::props::VAlign::Center;
                part.body.inset = Some([0; 4]);
            }
            return Some(InlineObject::Shape {
                kind: ShapeKind::TextBox,
                w,
                h,
                fill: None,
                stroke: None,
                stroke_width,
                float,
                story: Some(id),
                freeform: None,
                effects: Default::default(),
                extra: ShapeExtra { warp, ..ShapeExtra::default() },
            });
        }
        if let Some((tb, t)) = vml_textbox(shape, 0)
            && sc.story_depth < MAX_STORY_DEPTH
        {
            let id = self.read_textbox(sc, t, rels);
            if let Some(part) = self.doc.parts.get_mut(&id) {
                // Internal margins `inset="l,t,r,b"` (ECMA-376 Part 4 §19.1.2.22), each one left
                // out keeping Word's default; a bare number is in EMUs, VML's base unit.
                // `o:insetmode="auto"` leaves them to the application.
                let mut inset = wordcraft_doc::TEXT_BOX_INSET;
                let custom = tb.attr("o:insetmode").is_none_or(|m| !m.eq_ignore_ascii_case("auto"));
                for (slot, v) in inset.iter_mut().zip(tb.attr("inset").filter(|_| custom).unwrap_or("").split(',')) {
                    if let Some(pt) = measure(v, 12_700.0) {
                        *slot = (pt.max(0.0) * 12_700.0).round().min(wordcraft_doc::MAX_TEXT_BOX_INSET as f32) as u32;
                    }
                }
                part.body.inset = (inset != wordcraft_doc::TEXT_BOX_INSET).then_some(inset);
                // Resize shape to fit text (DrawingML's `a:spAutoFit`).
                let fit = vml_style(tb.attr("style").unwrap_or(""), "mso-fit-shape-to-text").or_else(|| vml_style(style, "mso-fit-shape-to-text"));
                part.body.auto_fit = fit.is_some_and(|v| v.eq_ignore_ascii_case("t") || v.eq_ignore_ascii_case("true"));
                // Where the text sits between the box's top and bottom (DrawingML's `anchor`).
                if let Some(a) = vml_style(tb.attr("style").unwrap_or(""), "v-text-anchor").or_else(|| vml_style(style, "v-text-anchor")) {
                    let al = a.trim().to_ascii_lowercase();
                    part.body.anchor = if al.starts_with("middle") || al == "center" {
                        wordcraft_doc::props::VAlign::Center
                    } else if al.starts_with("bottom") {
                        wordcraft_doc::props::VAlign::Bottom
                    } else {
                        wordcraft_doc::props::VAlign::Top
                    };
                }
            }
            let story = Some(id);
            // Text in a preset shape keeps the shape around it; otherwise it is a text box.
            let kind = kind.filter(|k| *k != ShapeKind::Rectangle && !k.is_open()).unwrap_or(ShapeKind::TextBox);
            return Some(InlineObject::Shape {
                kind,
                w,
                h,
                fill,
                stroke,
                stroke_width,
                float,
                story,
                freeform: None,
                effects: Default::default(),
                extra: ShapeExtra { cap, ..Default::default() },
            });
        }
        // A freeform: its `path`, in its `coordsize` space; a `v:fill` with no opacity
        // or `nf` in the path leaves it unfilled. Not WordArt (`v:textpath`).
        if shape.local() == "shape"
            && shape.child("v:textpath").is_none()
            && let Some(path) = shape.attr("path")
            && let Some(vp) = crate::read::freeform::vml_path(path, shape.attr("coordsize"), shape.attr("coordorigin"), w, h)
        {
            let opacity = |el: &str| shape.child(el).and_then(|e| e.attr("opacity")).and_then(vml_opacity).unwrap_or(1.0);
            let fill = fill.filter(|_| !vp.no_fill && opacity("v:fill") > 0.0);
            let stroke = stroke.filter(|_| !vp.no_stroke);
            if stroke.is_none() && fill.is_none() {
                return None;
            }
            let mut f = vp.freeform;
            f.alpha = opacity("v:stroke");
            return Some(InlineObject::Shape {
                kind: ShapeKind::Freeform,
                w,
                h,
                fill,
                stroke,
                stroke_width,
                float,
                story: None,
                freeform: Some(Arc::new(f)),
                effects: Default::default(),
                extra: ShapeExtra { cap, ..Default::default() },
            });
        }
        let kind = kind?;
        let mut extra = ShapeExtra { cap, ..Default::default() };
        if kind == ShapeKind::Line {
            // Arrowheads (`v:stroke` startarrow / endarrow): any but `none` is drawn.
            let arrow = |name: &str| shape.child("v:stroke").and_then(|s| s.attr(name)).is_some_and(|v| v != "none");
            extra.arrow_start = arrow("startarrow");
            extra.arrow_end = arrow("endarrow");
        }
        if shape.local() == "line" {
            // A line runs `from` → `to` (points, offsets from where its style places it). A preset
            // line (`v:shape`) runs corner to corner of the box its style gives it.
            let pt = |name: &str| -> (f32, f32) {
                let v = shape.attr(name).unwrap_or("0,0");
                let (x, y) = v.split_once(',').unwrap_or((v, "0"));
                let m = |s: &str| measure(s, 1.0).map_or(0.0, wordcraft_geom::finite).clamp(-crate::units::MAX_LEN_PT, crate::units::MAX_LEN_PT);
                (m(x), m(y))
            };
            let ((x0, y0), (x1, y1)) = (pt("from"), pt("to"));
            let max = crate::units::MAX_LEN_PT;
            (w, h) = (((x1 - x0).abs() * sx).min(max), ((y1 - y0).abs() * sy).min(max));
            float.x = (float.x + x0.min(x1)).clamp(-max, max);
            float.y = (float.y + y0.min(y1)).clamp(-max, max);
            // Drawn top-left to bottom-right; flipped for the other directions.
            float.flip_h ^= x1 < x0;
            float.flip_v ^= y1 < y0;
        }
        if stroke.is_none() && (fill.is_none() || kind == ShapeKind::Line) {
            return None;
        }
        Some(InlineObject::Shape {
            kind,
            w,
            h,
            fill: fill.filter(|_| kind != ShapeKind::Line),
            stroke,
            stroke_width,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra,
        })
    }

    /// An OLE object (`w:object`, ECMA-376 §17.3.3.19): shown as its picture (VML, or a DrawingML
    /// picture in `w:drawing`), and kept whole so saving writes the object back.
    pub(super) fn read_object(&mut self, sc: &mut StoryCtx, obj: &El, rels: &Rels) -> Option<InlineObject> {
        let mut o = match self.read_vml(sc, obj, rels) {
            Some(o) => o,
            None => {
                let d = obj.child("w:drawing")?;
                self.read_drawing(sc, d, rels)?
            }
        };
        if let InlineObject::Image { ole, .. } = &mut o {
            *ole = self.keep_embedded(obj, rels, &[]);
        }
        Some(o)
    }

    /// A Word signature line (Insert › Signature Line): a `w:pict` whose one VML picture carries
    /// an `o:signatureline` (the suggested signer, title, e-mail and instructions). Shown as its
    /// picture, and kept whole like an OLE object so saving writes the signature line back,
    /// still signable in Word, not a plain picture. `None` for any other `w:pict`.
    pub(super) fn read_signature_line(&mut self, sc: &mut StoryCtx, pict: &El, rels: &Rels) -> Option<InlineObject> {
        let mut drawings = pict.els().filter(|e| e.name.starts_with("v:") && e.local() != "shapetype");
        let shape = drawings.next().filter(|s| s.name == "v:shape")?;
        if drawings.next().is_some() || shape.child("o:signatureline").is_none() {
            return None;
        }
        let mut o = self.read_vml_shape(sc, pict, shape, rels, None)?;
        if let InlineObject::Image { ole, .. } = &mut o {
            *ole = self.keep_embedded(pict, rels, &[]);
        }
        Some(o)
    }
}

/// The DrawingML text warps of VML's WordArt shapes, `o:spt` 136 (plain text) to 175 (can down), in
/// order: the same shapes under their DrawingML names (`ST_TextShapeType`).
const VML_TEXT_WARPS: [&str; 40] = [
    "textPlain",
    "textStop",
    "textTriangle",
    "textTriangleInverted",
    "textChevron",
    "textChevronInverted",
    "textRingInside",
    "textRingOutside",
    "textArchUp",
    "textArchDown",
    "textCircle",
    "textButton",
    "textArchUpPour",
    "textArchDownPour",
    "textCirclePour",
    "textButtonPour",
    "textCurveUp",
    "textCurveDown",
    "textCascadeUp",
    "textCascadeDown",
    "textWave1",
    "textWave2",
    "textDoubleWave1",
    "textWave4",
    "textInflate",
    "textDeflate",
    "textInflateBottom",
    "textDeflateBottom",
    "textInflateTop",
    "textDeflateTop",
    "textDeflateInflate",
    "textDeflateInflateDeflate",
    "textFadeRight",
    "textFadeLeft",
    "textFadeUp",
    "textFadeDown",
    "textSlantUp",
    "textSlantDown",
    "textCanUp",
    "textCanDown",
];

/// A VML shape's preset type (`o:spt`): its own, its shapetype's, or the number in Word's
/// shapetype id `_x0000_tNN` (`type_id`), Word writing each shapetype only at its first use.
fn vml_spt(shape: &El, shapetype: Option<&El>, type_id: Option<&str>) -> Option<u32> {
    let num = |v: &str| v.trim().parse::<u32>().ok();
    shape
        .attr("o:spt")
        .and_then(num)
        .or_else(|| shapetype.and_then(|t| t.attr("o:spt")).and_then(num))
        .or_else(|| type_id?.strip_prefix("_x0000_t").and_then(num))
}

/// The value of property `key` in a VML `style` attribute (CSS declarations; the last one wins).
/// The first `v:textbox` under `e` holding text, with its `w:txbxContent` (depth-first, bounded
/// like `El::find`): in a group, a member's margins go with that member's text.
/// A VML `coordsize="w,h"` or `coordorigin="x,y"` pair (ECMA-376 Part 4 §14.1.2): the member
/// space of a group, in its own unitless coordinates.
fn vml_coordpair(e: &El, name: &str) -> Option<(f32, f32)> {
    let v = e.attr(name)?;
    let (x, y) = v.split_once(',')?;
    let n = |s: &str| measure(s.trim(), 1.0).map(wordcraft_geom::finite);
    Some((n(x)?, n(y)?))
}

fn vml_textbox(e: &El, depth: usize) -> Option<(&El, &El)> {
    if depth > crate::xml::MAX_DEPTH {
        return None;
    }
    e.els().find_map(|c| match c.child("w:txbxContent") {
        Some(t) if c.name == "v:textbox" => Some((c, t)),
        _ => vml_textbox(c, depth + 1),
    })
}

fn vml_style<'a>(style: &'a str, key: &str) -> Option<&'a str> {
    style.rsplit(';').filter_map(|d| d.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| v.trim())
}

/// A `v:textpath` style value with one pair of surrounding quotes removed and the writer's CSS
/// escapes mapped back: `\` + up to six hex digits + one optional space is that character, and
/// `\x` is `x` (#3994; `write::watermark::run` writes them). Only the surrounding pair goes, so a
/// font name that starts or ends with an apostrophe keeps it. Word writes font names raw, and a
/// lone trailing `\` none of CSS would accept stays literal.
fn css_string(v: &str) -> String {
    let v = v.trim();
    let v = ['"', '\''].into_iter().find_map(|q| v.strip_prefix(q)?.strip_suffix(q)).unwrap_or(v).trim();
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 {
            match chars.clone().next() {
                Some(h) if h.is_ascii_hexdigit() => {
                    hex.push(h);
                    chars.next();
                }
                _ => break,
            }
        }
        if hex.is_empty() {
            match chars.next() {
                Some(e) => out.push(e),
                None => out.push('\\'),
            }
        } else {
            if let Some(d) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push(d);
            }
            // One whitespace after the escape is its terminator, not part of the value.
            if chars.clone().next().is_some_and(char::is_whitespace) {
                chars.next();
            }
        }
    }
    out
}

/// Placement of a VML shape (ECMA-376 Part 4, VML): `position:absolute` in its style makes it
/// float, offset by `left`/`margin-left` and `top`/`margin-top` from the area named by
/// `mso-position-horizontal-relative` / `mso-position-vertical-relative` (unless aligned with
/// `mso-position-horizontal` / `mso-position-vertical`), wrapped as its `w10:wrap` says. Without
/// wrapping (`none`, or no `w10:wrap`) it's in front of the text, or behind it at a negative
/// `z-index`. Anything else stays inline.
fn vml_float(shape: &El, style: &str, in_hf: bool) -> Float {
    let get = |k: &str| vml_style(style, k);
    let hidden = get("visibility").is_some_and(|v| v.eq_ignore_ascii_case("hidden"));
    if !get("position").is_some_and(|p| p.eq_ignore_ascii_case("absolute")) {
        // Inline: only the turn (an OLE object turned in WordCraft is saved so, #319).
        let mut f = Float { hidden, ..Float::default() };
        f.set_spin(vml_spin(style));
        return f;
    }
    let off = |k: &str| get(k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0);
    let max = crate::units::MAX_LEN_PT;
    let behind = get("z-index").and_then(int).is_some_and(|z| z < 0);
    let wrap_el = shape.child("w10:wrap");
    let wrap_type = wrap_el.and_then(|w| w.attr("type")).unwrap_or("none");
    let wrap = match wrap_type {
        t if t.eq_ignore_ascii_case("square") => Wrap::Square,
        t if t.eq_ignore_ascii_case("tight") => Wrap::Tight,
        t if t.eq_ignore_ascii_case("through") => Wrap::Through,
        t if t.eq_ignore_ascii_case("topAndBottom") || t.eq_ignore_ascii_case("top-and-bottom") => Wrap::TopAndBottom,
        _ if behind => Wrap::BehindText,
        _ => Wrap::InFrontOfText,
    };
    let wrap_text = wrap_el.and_then(|w| w.attr("side")).and_then(wordcraft_doc::para::WrapText::from_ooxml).unwrap_or_default();
    let h_rel_str = get("mso-position-horizontal-relative").unwrap_or("text");
    let h_rel = match h_rel_str {
        s if s.eq_ignore_ascii_case("page") => Anchor::Page,
        s if s.eq_ignore_ascii_case("margin") => Anchor::Margin,
        s if s.eq_ignore_ascii_case("char") => Anchor::Character,
        s if s.eq_ignore_ascii_case("left-margin-area") => Anchor::LeftMargin,
        s if s.eq_ignore_ascii_case("right-margin-area") => Anchor::RightMargin,
        s if s.eq_ignore_ascii_case("inner-margin-area") => Anchor::InsideMargin,
        s if s.eq_ignore_ascii_case("outer-margin-area") => Anchor::OutsideMargin,
        _ => Anchor::Column,
    };
    let v_rel_str = get("mso-position-vertical-relative").unwrap_or("text");
    let v_rel = match v_rel_str {
        s if s.eq_ignore_ascii_case("page") => Anchor::Page,
        s if s.eq_ignore_ascii_case("margin") => Anchor::Margin,
        s if s.eq_ignore_ascii_case("line") => Anchor::Line,
        s if s.eq_ignore_ascii_case("top-margin-area") => Anchor::TopMargin,
        s if s.eq_ignore_ascii_case("bottom-margin-area") => Anchor::BottomMargin,
        s if s.eq_ignore_ascii_case("inner-margin-area") => Anchor::InsideMargin,
        s if s.eq_ignore_ascii_case("outer-margin-area") => Anchor::OutsideMargin,
        _ => Anchor::Paragraph,
    };
    let dist = |k: &str| get(k).and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, 1584.0);
    let mut f = Float {
        wrap,
        wrap_text,
        h_rel,
        v_rel,
        x: (off("left") + off("margin-left")).clamp(-max, max),
        y: (off("top") + off("margin-top")).clamp(-max, max),
        h_align: get("mso-position-horizontal").and_then(FloatAlign::from_ooxml),
        v_align: get("mso-position-vertical").and_then(FloatAlign::from_ooxml),
        dist: dist("mso-wrap-distance-left").max(dist("mso-wrap-distance-right")),
        dist_top: dist("mso-wrap-distance-top"),
        dist_bottom: dist("mso-wrap-distance-bottom"),
        hidden,
        // Word for Mac draws a VML shape anchored in a header table cell on the page whatever
        // `o:allowincell` says (user-checked), as for Word's picture and text watermarks.
        outside_cell: in_hf,
        ..Float::default()
    };
    f.set_spin(vml_spin(style));
    f
}

/// Word's text watermark (Design › Watermark): a VML shape in a header whose `v:textpath` draws
/// its `string` as text (ECMA-376 Part 4, VML `textpath`), in the textpath's `font-family`, filled
/// with the shape's `fillcolor`, semitransparent when its `v:fill` has an `opacity` below 1, and
/// diagonal when the shape is turned. Only Word's watermark shape counts (its `id`), or text
/// placed absolutely and centred both ways; other header WordArt (a logo or a title) is None.
fn vml_watermark(shape: &El) -> Option<wordcraft_doc::Watermark> {
    let tp = shape.child("v:textpath")?;
    if tp.attr("on").is_some_and(|v| matches!(v, "f" | "false" | "0")) {
        return None;
    }
    let style = shape.attr("style").unwrap_or("");
    let centred = |k: &str| vml_style(style, k).is_some_and(|v| v.eq_ignore_ascii_case("center"));
    let word = shape.attr("id").is_some_and(|id| id.starts_with("PowerPlusWaterMarkObject"));
    let placed = vml_style(style, "position").is_some_and(|p| p.eq_ignore_ascii_case("absolute"))
        && centred("mso-position-horizontal")
        && centred("mso-position-vertical");
    if !word && !placed {
        return None;
    }
    let text: String = tp.attr("string")?.chars().take(200).collect();
    if text.trim().is_empty() {
        return None;
    }
    let default = wordcraft_doc::Watermark::default();
    let font = tp
        .attr("style")
        .and_then(|s| vml_style(s, "font-family"))
        .map(|f| css_string(f).chars().take(100).collect::<String>())
        .filter(|f| !f.is_empty())
        .unwrap_or(default.font);
    let color = shape.attr("fillcolor").and_then(vml_color).unwrap_or(default.color);
    let opacity = shape.child("v:fill").and_then(|f| f.attr("opacity")).and_then(vml_opacity).unwrap_or(1.0);
    // The box the text is stretched to (`fitshape`).
    let len = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(4000.0));
    let size = len("width").zip(len("height")).map(|(w, h)| [w, h]);
    Some(wordcraft_doc::Watermark { text, font, color, diagonal: vml_spin(style).deg != 0.0, semitransparent: opacity < 1.0, size })
}

/// A VML `opacity`, 0..=1.
fn vml_opacity(v: &str) -> Option<f32> {
    vml_fraction(v).map(|f| f.clamp(0.0, 1.0))
}

/// A VML shape's turn: `rotation` in degrees, or 65536ths of one with an `fd` suffix; `flip`: `x`
/// and/or `y`.
fn vml_spin(style: &str) -> wordcraft_geom::Spin {
    let get = |k: &str| vml_style(style, k);
    let rot = get("rotation").and_then(|v| match v.strip_suffix("fd") {
        Some(fd) => fd.trim().parse::<f32>().ok().map(|f| f / 65_536.0),
        None => v.trim().parse::<f32>().ok(),
    });
    let flip = get("flip").unwrap_or("");
    wordcraft_geom::Spin::new(rot.unwrap_or(0.0), flip.contains('x'), flip.contains('y'))
}
