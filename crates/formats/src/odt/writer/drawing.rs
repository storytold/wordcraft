//! Pictures, text boxes, drawing shapes, groups and formula objects (ODF 1.3 §10.3–10.4): where
//! each is anchored and placed, how it looks, and its content.

use super::{FBlock, Inline, Rgb, Writer, mime_of, model, pt, x};

/// A text box frame's graphic properties for `look`: its line (§20.183 `fo:border`), padding
/// (§20.210 `fo:padding-*`) and fill, both ways (LibreOffice reads `draw:fill`, and fills a
/// frame without it).
pub(in crate::odt) fn box_look_attrs(look: &model::BoxLook, defs: &mut crate::odt::strokes::DrawDefs, prefix: &str) -> String {
    let hex = |c: Rgb| format!("#{}", c.hex().to_ascii_lowercase());
    let mut a = match look.line {
        Some((c, w)) if w.is_finite() && w > 0.0 => format!(" fo:border=\"{} solid {}\"", pt(w.min(12.0)), hex(c)),
        _ => " fo:border=\"none\"".to_string(),
    };
    let [l, t, r, b] = look.inset.unwrap_or(DEFAULT_BOX_INSET).map(|v| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 });
    a.push_str(&format!(
        " fo:padding-left=\"{}\" fo:padding-top=\"{}\" fo:padding-right=\"{}\" fo:padding-bottom=\"{}\"",
        pt(l),
        pt(t),
        pt(r),
        pt(b)
    ));
    a.push_str(&fill_attrs(look.fill.as_ref(), true, defs, prefix));
    a
}

/// The graphic properties of fill `fill` (§20.118 `draw:fill`, §20.119 `draw:fill-color`,
/// §20.120 `draw:fill-gradient-name`; a frame's `fo:background-color` too): a gradient by the
/// name of its definition, with its main colour for readers that draw none.
pub(in crate::odt) fn fill_attrs(
    fill: Option<&wordcraft_doc::fill::Fill>,
    frame: bool,
    defs: &mut crate::odt::strokes::DrawDefs,
    prefix: &str,
) -> String {
    use wordcraft_doc::fill::Fill;
    let hex = |c: Rgb| format!("#{}", c.hex().to_ascii_lowercase());
    let background = |c: &str| if frame { format!(" fo:background-color=\"{c}\"") } else { String::new() };
    match fill {
        Some(f @ Fill::Gradient(g)) => {
            let main = hex(f.main_color());
            match defs.gradient(g, prefix) {
                Some(name) => format!("{} draw:fill=\"gradient\" draw:fill-gradient-name=\"{name}\" draw:fill-color=\"{main}\"", background(&main)),
                None => format!("{} draw:fill=\"solid\" draw:fill-color=\"{main}\"", background(&main)),
            }
        }
        Some(f) => {
            let c = hex(f.main_color());
            format!("{} draw:fill=\"solid\" draw:fill-color=\"{c}\"", background(&c))
        }
        None => format!("{} draw:fill=\"none\"", background("transparent")),
    }
}

/// Where a frame or shape is written.
#[derive(Clone, Copy)]
pub(in crate::odt) enum Place<'a> {
    /// In the text: anchored and placed as its float says, in line without one.
    Text(Option<&'a wordcraft_doc::para::Float>),
    /// A member of a group, at (x, y) from where the group is anchored; its float turns or
    /// mirrors it.
    Member(f32, f32, Option<&'a wordcraft_doc::para::Float>),
}

impl<'a> Place<'a> {
    /// What turns or mirrors the object, and places it in the text.
    pub(in crate::odt) fn float(self) -> Option<&'a wordcraft_doc::para::Float> {
        match self {
            Place::Text(f) | Place::Member(_, _, f) => f,
        }
    }

    /// Its top left corner (unturned) from where it is anchored, points.
    pub(in crate::odt) fn origin(self) -> (f32, f32) {
        let off = |v: f32, aligned: bool| if aligned || !v.is_finite() { 0.0 } else { v.clamp(-1584.0, 1584.0) };
        match self {
            Place::Member(x, y, _) => (off(x, false), off(y, false)),
            Place::Text(f) => f
                .filter(|f| f.wrap != wordcraft_doc::para::Wrap::Inline)
                .map_or((0.0, 0.0), |f| (off(f.x, f.h_align.is_some()), off(f.y, f.v_align.is_some()))),
        }
    }
}

/// How a frame or shape `w` × `h` at `place` is anchored and placed: its `text:anchor-type`
/// attribute, its graphic style's placement and its `svg:x`/`svg:y` (or `draw:transform` when
/// it is turned). A group's member has no anchor of its own: it is placed by its position alone.
pub(in crate::odt) fn placement(place: Place<'_>, w: f32, h: f32) -> (String, String, String) {
    let anchor = |a: &str| format!(" text:anchor-type=\"{a}\"");
    match place {
        Place::Text(Some(f)) if f.wrap != wordcraft_doc::para::Wrap::Inline => {
            let (a, props, xy) = crate::odt::frames::frame_of(f, w, h);
            (anchor(a), props, xy)
        }
        Place::Text(f) => {
            let turn = f.filter(|f| crate::odt::frames::turned(f)).map(|f| crate::odt::frames::transform_attr(0.0, 0.0, w, h, f.rot));
            (anchor("as-char"), " style:vertical-pos=\"top\" style:vertical-rel=\"baseline\"".to_string(), turn.unwrap_or_default())
        }
        Place::Member(_, _, f) => {
            let (x, y) = place.origin();
            let xy = match f.filter(|f| crate::odt::frames::turned(f)) {
                Some(f) => crate::odt::frames::transform_attr(x, y, w, h, f.rot),
                None => format!(" svg:x=\"{}\" svg:y=\"{}\"", pt(x), pt(y)),
            };
            (String::new(), String::new(), xy)
        }
    }
}

/// A drawing shape's graphic properties for `look`: its line (§20.167 `draw:stroke`,
/// `svg:stroke-color`, `svg:stroke-width`), fill (§20.118 `draw:fill`, `draw:fill-color`),
/// whether it grows to its text (§20.105 `draw:auto-grow-height`) and the padding around its
/// text.
pub(in crate::odt) fn shape_look_attrs(look: &model::BoxLook, defs: &mut crate::odt::strokes::DrawDefs, prefix: &str) -> String {
    let hex = |c: Rgb| format!("#{}", c.hex().to_ascii_lowercase());
    let mut a = match look.line {
        Some((c, w)) if w.is_finite() && w > 0.0 => {
            let w = w.min(12.0);
            let mut a = match defs.dash(&look.dash, prefix) {
                Some(name) => format!(" draw:stroke=\"dash\" draw:stroke-dash=\"{name}\""),
                None => " draw:stroke=\"solid\"".to_string(),
            };
            a.push_str(&format!(" svg:stroke-color=\"{}\" svg:stroke-width=\"{}\"", hex(c), pt(w)));
            // Arrowheads (§20.164 `draw:marker-start`, §20.162 `draw:marker-end`).
            for (on, end) in [(look.arrows.0, "start"), (look.arrows.1, "end")] {
                if on {
                    let marker = defs.arrow();
                    a.push_str(&format!(" draw:marker-{end}=\"{marker}\" draw:marker-{end}-width=\"{}\"", crate::odt::strokes::arrow_width(w)));
                }
            }
            a
        }
        _ => " draw:stroke=\"none\"".to_string(),
    };
    a.push_str(&fill_attrs(look.fill.as_ref(), false, defs, prefix));
    // Its size is its own unless it resizes to fit its text (LibreOffice otherwise grows and
    // shrinks a shape with text to it).
    a.push_str(&format!(" draw:auto-grow-height=\"{}\" draw:auto-grow-width=\"false\"", look.auto_fit));
    if let Some(inset) = look.inset {
        let [l, t, r, b] = inset.map(|v| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 });
        a.push_str(&format!(
            " fo:padding-left=\"{}\" fo:padding-top=\"{}\" fo:padding-right=\"{}\" fo:padding-bottom=\"{}\"",
            pt(l),
            pt(t),
            pt(r),
            pt(b)
        ));
    }
    a
}

/// A text box's default inner margins (left, top, right, bottom), points: WordCraft's and
/// Word's (0.1" and 0.05").
pub(in crate::odt) const DEFAULT_BOX_INSET: [f32; 4] = [7.2, 3.6, 7.2, 3.6];

impl Writer {
    pub(in crate::odt) fn picture(&mut self, img: &model::Img) -> String {
        let ext = if img.ext == "jpeg" { "jpg".to_string() } else { img.ext.clone() };
        let name = format!("Pictures/{}image{}.{}", self.prefix, self.pictures.len() + 1, ext);
        self.pictures.push((name.clone(), img.data.clone(), mime_of(&img.ext).to_string()));
        name
    }

    /// A text box or another drawing shape at `place`.
    pub(in crate::odt) fn drawing_shape(
        &mut self,
        kind: wordcraft_doc::para::ShapeKind,
        size: (f32, f32),
        place: Place<'_>,
        (look, path): (&model::BoxLook, Option<&wordcraft_doc::freeform::Freeform>),
        blocks: &[FBlock],
        out: &mut String,
    ) {
        if kind == wordcraft_doc::para::ShapeKind::TextBox {
            self.text_box(size, place, look, blocks, out);
        } else {
            self.shape(kind, size, place, (look, path), blocks, out);
        }
    }

    /// A picture (ODF 1.3 §10.4.4 `draw:image` in a `draw:frame`) at `place`: a floating one or a
    /// group's in a graphic style that places and wraps it.
    pub(in crate::odt) fn image(&mut self, img: &model::Img, place: Place<'_>, out: &mut String) {
        let href = self.picture(img);
        let n = self.pictures.len();
        let (anchor, style) = match place {
            Place::Text(None) => (" text:anchor-type=\"as-char\"".to_string(), String::new()),
            _ => {
                let (anchor, props, xy) = placement(place, img.w, img.h);
                (anchor, format!(" draw:style-name=\"{}\"{xy}", self.graphic_style(props)))
            }
        };
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Image{n}\"{style}{anchor} svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"0\"><draw:image xlink:href=\"{href}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
            self.prefix,
            pt(img.w),
            pt(img.h)
        ));
        if !img.alt.is_empty() {
            out.push_str(&format!("<svg:desc>{}</svg:desc>", x(&img.alt)));
        }
        out.push_str("</draw:frame>");
    }

    /// A group (ODF 1.3 §10.3.15 `draw:g`) of `size`: anchored and wrapping as its float says, in
    /// line without one; its members at their offsets from its corner.
    pub(in crate::odt) fn group(
        &mut self,
        size: (f32, f32),
        float: Option<&wordcraft_doc::para::Float>,
        members: &[model::Member],
        out: &mut String,
    ) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        // The group isn't turned: its members are placed by their own positions.
        let (anchor, props, _) = placement(Place::Text(float.filter(|f| f.wrap != wordcraft_doc::para::Wrap::Inline)), size.0, size.1);
        let style = self.graphic_style(props);
        self.boxes += 1;
        out.push_str(&format!("<draw:g draw:name=\"{}Group{}\" draw:style-name=\"{style}\"{anchor} draw:z-index=\"0\">", self.prefix, self.boxes));
        let at = Place::Text(float).origin();
        self.members(members, at, 0, out);
        out.push_str("</draw:g>");
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// A group's `members`, its corner at `at` from where it is anchored (a group in it `depth`
    /// deep).
    pub(in crate::odt) fn members(&mut self, members: &[model::Member], at: (f32, f32), depth: usize, out: &mut String) {
        for m in members {
            let (x, y) = (at.0 + m.x, at.1 + m.y);
            match &m.inline {
                Inline::Shape { kind, w, h, float, blocks, look, path } => {
                    self.drawing_shape(*kind, (*w, *h), Place::Member(x, y, float.as_deref()), (look, path.as_deref()), blocks, out)
                }
                Inline::Image(img) => self.image(img, Place::Member(x, y, img.float.as_deref()), out),
                Inline::Group { members, .. } if depth < model::MAX_GROUP_DEPTH => {
                    out.push_str("<draw:g>");
                    self.members(members, (x, y), depth + 1, out);
                    out.push_str("</draw:g>");
                }
                _ => {}
            }
        }
    }

    /// A text box (ODF 1.3 §10.4.3 `draw:text-box` in a `draw:frame`) of `size` (points): in line
    /// with the text, or anchored and placed as it floats, with its fill, line and padding.
    pub(in crate::odt) fn text_box(&mut self, size: (f32, f32), place: Place<'_>, look: &model::BoxLook, blocks: &[FBlock], out: &mut String) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        let frame = box_look_attrs(look, &mut self.draw_defs, self.prefix);
        let (anchor, props, xy) = placement(place, size.0, size.1);
        let style = self.graphic_style(format!("{props}{frame}"));
        let mut body = self.story(blocks);
        if body.is_empty() {
            body.push_str("<text:p/>");
        }
        self.boxes += 1;
        let fit = |v: f32| if v.is_finite() { v.clamp(1.0, 1584.0) } else { 72.0 };
        // A box growing to its text has a least height (§20.207 `fo:min-height`), not a height.
        let (height, min) = if look.auto_fit {
            (String::new(), format!(" fo:min-height=\"{}\"", pt(fit(size.1))))
        } else {
            (format!(" svg:height=\"{}\"", pt(fit(size.1))), String::new())
        };
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Frame{}\" draw:style-name=\"{style}\"{anchor} svg:width=\"{}\"{height}{xy} draw:z-index=\"0\"><draw:text-box{min}>{body}</draw:text-box></draw:frame>",
            self.prefix,
            self.boxes,
            pt(fit(size.0)),
        ));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// A drawing shape (ODF 1.3 §10.3) of `size` (points): a rectangle, ellipse or line, or a
    /// custom shape of a predefined geometry, in line with the text or anchored and placed as it
    /// floats, with its fill, line and text.
    pub(in crate::odt) fn shape(
        &mut self,
        kind: wordcraft_doc::para::ShapeKind,
        size: (f32, f32),
        place: Place<'_>,
        (look, path): (&model::BoxLook, Option<&wordcraft_doc::freeform::Freeform>),
        blocks: &[FBlock],
        out: &mut String,
    ) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        let float = place.float();
        let (anchor, props, xy) = placement(place, size.0, size.1);
        let look_attrs = shape_look_attrs(look, &mut self.draw_defs, self.prefix);
        let style = self.graphic_style(format!("{props}{look_attrs}"));
        let body = self.story(blocks);
        self.boxes += 1;
        let (element, geometry) = match crate::odt::shapes::element_of(kind) {
            // A freeform without paths has nothing to draw but its frame.
            ("draw:path", _) if path.is_none() => ("draw:rect", None),
            e => e,
        };
        let len = |v: f32, least: f32| if v.is_finite() { v.clamp(least, 1584.0) } else { 72.0 };
        let at = if element == "draw:line" {
            {
                // A line by its ends: from its box's top left to its bottom right, or bottom left
                // to top right when it rises.
                let (w, h) = (len(size.0, 0.0), len(size.1, 0.0));
                let (x, y) = place.origin();
                // From its start to its end: mirrored, it starts on the right or at the bottom.
                let (fh, fv) = float.map_or((false, false), |f| (f.flip_h, f.flip_v));
                let ((x1, x2), (y1, y2)) = (if fh { (x + w, x) } else { (x, x + w) }, if fv { (y + h, y) } else { (y, y + h) });
                // Turned about its centre: its ends turned with it.
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                let (sin, cos) = float.filter(|f| crate::odt::frames::turned(f)).map_or(0.0, |f| f.rot.to_radians()).sin_cos();
                let turn = |px: f32, py: f32| (cx + (px - cx) * cos - (py - cy) * sin, cy + (px - cx) * sin + (py - cy) * cos);
                let ((x1, y1), (x2, y2)) = (turn(x1, y1), turn(x2, y2));
                format!(" svg:x1=\"{}\" svg:y1=\"{}\" svg:x2=\"{}\" svg:y2=\"{}\"", pt(x1), pt(y1), pt(x2), pt(y2))
            }
        } else {
            let (w, h) = (len(size.0, 1.0), len(size.1, 1.0));
            let mut at = format!(" svg:width=\"{}\" svg:height=\"{}\"{xy}", pt(w), pt(h));
            if let Some(f) = path.filter(|_| element == "draw:path") {
                // Its paths in a view box of hundredths of a point (whole numbers, §19.570).
                let (vw, vh) = ((w * 100.0).round().max(1.0) as u32, (h * 100.0).round().max(1.0) as u32);
                at.push_str(&format!(" svg:viewBox=\"0 0 {vw} {vh}\" svg:d=\"{}\"", crate::odt::path::path_data(f, vw, vh)));
            }
            at
        };
        // Mirrored, as a custom shape's geometry says (§20.112, §20.113).
        let mirror = |on: bool, k: &str| if on { format!(" draw:mirror-{k}=\"true\"") } else { String::new() };
        let (fh, fv) = float.map_or((false, false), |f| (f.flip_h, f.flip_v));
        let geometry = geometry
            .map(|t| {
                format!(
                    "<draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"{t}\"{}{}/>",
                    mirror(fh, "horizontal"),
                    mirror(fv, "vertical")
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<{element} draw:name=\"{}Shape{}\" draw:style-name=\"{style}\"{anchor}{at} draw:z-index=\"0\">{body}{geometry}</{element}>",
            self.prefix, self.boxes,
        ));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// The paragraphs of a text box's or shape's text, written apart from the paragraph being
    /// written.
    pub(in crate::odt) fn story(&mut self, blocks: &[FBlock]) -> String {
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.take());
        let mut body = String::new();
        self.blocks(blocks, &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        body
    }

    /// An equation as an embedded formula object (ODF 1.3 §10.4.3): a frame in line with the text
    /// pointing at a sub-document whose `content.xml` is the equation's MathML, its frame about
    /// the size the formula takes (LibreOffice scales the formula to the frame).
    pub(in crate::odt) fn formula(&mut self, linear: &str, display: bool, out: &mut String) {
        let n = self.formulas.len() + 1;
        let name = format!("Object {}{n}", self.prefix);
        let mathml = crate::mathml::linear_to_mathml(linear, display);
        self.formulas.push((name.clone(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{mathml}\n")));
        let (w, h) = crate::odt::formula_size::size(linear);
        // Centred on the line, as LibreOffice sets its own formulas.
        let style = self.graphic_style(" style:vertical-pos=\"middle\" style:vertical-rel=\"text\"".into());
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Formula{n}\" draw:style-name=\"{style}\" text:anchor-type=\"as-char\" svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"0\"><draw:object xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/><svg:desc>{}</svg:desc></draw:frame>",
            self.prefix,
            pt(w),
            pt(h),
            x(&name),
            x(linear)
        ));
    }

    /// The automatic graphic style of a frame placed by `props`.
    pub(in crate::odt) fn graphic_style(&mut self, props: String) -> String {
        let k = match self.graphic_styles.iter().position(|s| *s == props) {
            Some(k) => k,
            None => {
                self.graphic_styles.push(props);
                self.graphic_styles.len() - 1
            }
        };
        format!("{}fr{}", self.prefix, k + 1)
    }
}
