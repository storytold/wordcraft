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
    // Which way its text runs (§20.404; upward is LibreOffice's own `bt-lr`; stacked letters
    // have no ODF form).
    a.push_str(match look.vert {
        wordcraft_doc::props::TextVert::Vert => " style:writing-mode=\"tb-rl\"",
        wordcraft_doc::props::TextVert::Vert270 => " loext:writing-mode=\"bt-lr\"",
        _ => "",
    });
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
    /// Its place in the stacking order (§19.231 `draw:z-index`, see `frames::z_of`).
    pub(in crate::odt) fn z_index(self) -> u32 {
        crate::odt::frames::z_index(self.float())
    }

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

/// A cropped picture's `fo:clip` (§20.179: top, right, bottom, left, in lengths of its original
/// size, its pixels at 96 per inch); empty for one not cropped or whose size can't be read.
fn clip_attr(img: &model::Img) -> String {
    let [l, t, r, b] = img.crop.map(|c| if c.is_finite() { c.clamp(0.0, 0.99) } else { 0.0 });
    if l + t + r + b <= 0.0 {
        return String::new();
    }
    let Some((pw, ph)) = model::image_px(&img.data).filter(|(w, h)| *w > 0 && *h > 0) else { return String::new() };
    let (w, h) = (pw as f32 * 0.75, ph as f32 * 0.75);
    format!(" fo:clip=\"rect({}, {}, {}, {})\"", pt(t * h), pt(r * w), pt(b * h), pt(l * w))
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
            // Raised or lowered (its float's `y`, points up): its top that far from the baseline
            // (§20.387 `from-top`); else standing on it.
            match f.map(|f| f.y).filter(|r| r.is_finite() && *r != 0.0 && turn.is_none()) {
                Some(raise) => (
                    anchor("as-char"),
                    " style:vertical-pos=\"from-top\" style:vertical-rel=\"baseline\"".to_string(),
                    format!(" svg:y=\"{}\"", pt(-(h + raise))),
                ),
                None => (anchor("as-char"), " style:vertical-pos=\"top\" style:vertical-rel=\"baseline\"".to_string(), turn.unwrap_or_default()),
            }
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
        // Not a name the file's own pictures, kept as read, have, nor one an earlier picture
        // took (after skipping a reserved name, the count is behind the names taken).
        let mut n = self.pictures.len() + 1;
        let mut name = format!("Pictures/{}image{n}.{ext}", self.prefix);
        while (self.reserved.contains(&name) || self.pictures.iter().any(|(taken, _, _)| *taken == name)) && n < usize::MAX {
            n += 1;
            name = format!("Pictures/{}image{n}.{ext}", self.prefix);
        }
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
    /// The next object's number and name (`Object 3`, `Object M3` in headers and footers).
    /// Not a name the file's own objects, kept as read, have (`Object wc1/content.xml`).
    pub(in crate::odt) fn object_name(&mut self) -> (usize, String) {
        loop {
            self.object_names = self.object_names.saturating_add(1);
            let name = format!("Object {}{}", self.prefix, self.object_names);
            let dir = format!("{name}/");
            if self.object_names == usize::MAX || !self.reserved.iter().any(|r| *r == name || r.starts_with(&dir)) {
                return (self.object_names, name);
            }
        }
    }

    /// Object `o`'s files in the package under `name`, and the link to it with its replacement
    /// picture (the one read, else the picture shown, `img`).
    fn kept_object(&mut self, o: &model::OdfObject, img: &model::Img) -> String {
        let (_, name) = self.object_name();
        self.object_files(o, &name);
        let replacement = format!("ObjectReplacements/{name}");
        match &o.replacement {
            Some(r) => self.replacement(&replacement, r),
            None => self.pictures.push((replacement.clone(), img.data.clone(), mime_of(&img.ext).to_string())),
        }
        let el = if o.ole { "draw:object-ole" } else { "draw:object" };
        format!(
            "<{el} xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/><draw:image xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
            x(&name),
            x(&replacement)
        )
    }

    /// Object `o` (a chart as read) in the package under the next object name, with its
    /// replacement picture when it had one: its number, name and the frame's content linking it.
    pub(in crate::odt) fn kept_object_files(&mut self, o: &model::OdfObject) -> (usize, String, String) {
        let (n, name) = self.object_name();
        self.object_files(o, &name);
        let mut link = format!("<draw:object xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>", x(&name));
        if let Some(r) = &o.replacement {
            let replacement = format!("ObjectReplacements/{name}");
            self.replacement(&replacement, r);
            link.push_str(&format!(
                "<draw:image xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
                x(&replacement)
            ));
        }
        (n, name, link)
    }

    /// Object `o`'s files in the package under `name`.
    fn object_files(&mut self, o: &model::OdfObject, name: &str) {
        if o.ole {
            if let Some((_, bytes)) = o.files.first() {
                self.pictures.push((name.to_string(), bytes.clone(), o.mime.clone()));
            }
        } else {
            self.object_dirs.push((format!("{name}/"), o.mime.clone()));
            for (path, bytes) in &o.files {
                let mime = if path.ends_with(".xml") {
                    "text/xml"
                } else if path.ends_with(".rdf") {
                    "application/rdf+xml"
                } else {
                    ""
                };
                self.pictures.push((format!("{name}/{path}"), bytes.clone(), mime.to_string()));
            }
        }
    }

    /// A replacement picture `r` read with an object, at `path`.
    fn replacement(&mut self, path: &str, r: &std::sync::Arc<Vec<u8>>) {
        let mime = crate::model::sniff_image(r).map(mime_of).unwrap_or("application/x-openoffice-gdimetafile;windows_formatname=\"GDIMetaFile\"");
        self.pictures.push((path.to_string(), r.clone(), mime.to_string()));
    }

    pub(in crate::odt) fn image(&mut self, img: &model::Img, place: Place<'_>, out: &mut String) {
        if let Some(o) = img.object.as_deref() {
            let inner = self.kept_object(o, img);
            return self.image_frame(img, place, &inner, out);
        }
        let href = self.picture(img);
        let inner = format!("<draw:image xlink:href=\"{href}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>");
        self.image_frame(img, place, &inner, out);
    }

    /// A picture's frame at `place`, holding `inner`.
    fn image_frame(&mut self, img: &model::Img, place: Place<'_>, inner: &str, out: &mut String) {
        let n = self.pictures.len();
        let clip = clip_attr(img);
        // Always placed by a style: in line, standing on the baseline (LibreOffice puts a frame
        // with no placement at the line's top).
        let (anchor, props, xy) = placement(place, img.w, img.h);
        let (anchor, style) =
            (anchor, format!(" draw:style-name=\"{}\"{xy}", self.graphic_style(format!("{}{props}{clip}", super::GRAPHICS_PARENT))));
        let z = place.z_index();
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Image{n}\"{style}{anchor} svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"{z}\">{inner}",
            self.prefix,
            pt(img.w),
            pt(img.h)
        ));
        if !img.alt.is_empty() {
            out.push_str(&format!("<svg:desc>{}</svg:desc>", x(&img.alt)));
        }
        // The outline tight or through wrapping follows, last (§10.4.2).
        if let Some(p) = img
            .float
            .as_deref()
            .filter(|f| matches!(f.wrap, wordcraft_doc::para::Wrap::Tight | wordcraft_doc::para::Wrap::Through))
            .and_then(|f| f.wrap_polygon.as_deref())
        {
            out.push_str(&crate::odt::frames::contour_xml(p, img.w, img.h));
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
                Inline::Chart(c) => self.chart(c, Place::Member(x, y, c.float.as_deref()), out),
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
        let style = self.graphic_style(format!("{}{props}{frame}", super::FRAME_PARENT));
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
        // Its name (its story's, which a linked box names), and the box its text continues in
        // (§19.112 `draw:chain-next-name`).
        let name = look.chain.as_deref().map_or_else(|| format!("{}Frame{}", self.prefix, self.boxes), |c| format!("{}{}", self.prefix, c.name));
        let next = look
            .chain
            .as_deref()
            .and_then(|c| c.next.as_deref())
            .map(|n| format!(" draw:chain-next-name=\"{}{}\"", self.prefix, x(n)))
            .unwrap_or_default();
        let z = place.z_index();
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}\" draw:style-name=\"{style}\"{anchor} svg:width=\"{}\"{height}{xy} draw:z-index=\"{z}\"><draw:text-box{min}{next}>{body}</draw:text-box></draw:frame>",
            x(&name),
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
        // A custom shape's geometry as read: written back whole, LibreOffice's own shape.
        let kept = look
            .geometry
            .as_deref()
            .filter(|_| (kind == wordcraft_doc::para::ShapeKind::Freeform && path.is_some()) || kind == wordcraft_doc::para::ShapeKind::Rectangle);
        let (element, geometry) = match crate::odt::shapes::element_of(kind) {
            _ if kept.is_some() => ("draw:custom-shape", None),
            // A freeform without paths has nothing to draw but its frame.
            ("draw:path", _) if path.is_none() => ("draw:rect", None),
            // Paths filled and unfilled: a custom shape, whose path says which is which
            // (LibreOffice fills no part of a path with an open part).
            ("draw:path", _) if path.is_some_and(crate::odt::path::is_mixed) => ("draw:custom-shape", Some("non-primitive")),
            e => e,
        };
        let mut custom_path = String::new();
        let len = |v: f32, least: f32| if v.is_finite() { v.clamp(least, 1584.0) } else { 72.0 };
        let at = if element == "draw:line" || element == "draw:connector" {
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
            if let Some(f) = path.filter(|_| geometry == Some("non-primitive")) {
                let (vw, vh) = ((w * 100.0).round().max(1.0) as u32, (h * 100.0).round().max(1.0) as u32);
                custom_path = format!(" svg:viewBox=\"0 0 {vw} {vh}\" draw:enhanced-path=\"{}\"", crate::odt::path::enhanced_path_data(f, vw, vh));
            }
            at
        };
        // Mirrored, as a custom shape's geometry says (§20.112, §20.113).
        let mirror = |on: bool, k: &str| if on { format!(" draw:mirror-{k}=\"true\"") } else { String::new() };
        let (fh, fv) = float.map_or((false, false), |f| (f.flip_h, f.flip_v));
        let geometry = match kept {
            // Its own mirroring, now, in the kept opening tag.
            Some(g) => match g.find('>') {
                Some(i) if g.get(..i).is_some_and(|h| !h.ends_with('/')) => {
                    format!(
                        "{}{}{}{}",
                        g.get(..i).unwrap_or_default(),
                        mirror(fh, "horizontal"),
                        mirror(fv, "vertical"),
                        g.get(i..).unwrap_or_default()
                    )
                }
                _ => g.to_string(),
            },
            None => String::new(),
        } + &geometry
            .map(|t| {
                if custom_path.is_empty() {
                    custom_path = " svg:viewBox=\"0 0 21600 21600\"".into();
                }
                format!("<draw:enhanced-geometry{custom_path} draw:type=\"{t}\"{}{}/>", mirror(fh, "horizontal"), mirror(fv, "vertical"))
            })
            .unwrap_or_default();
        let conn = self.conn_attrs(kind, look);
        let z = place.z_index();
        out.push_str(&format!(
            "<{element} draw:name=\"{}Shape{}\" draw:style-name=\"{style}\"{anchor}{at}{conn} draw:z-index=\"{z}\">{body}{geometry}</{element}>",
            self.prefix, self.boxes,
        ));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// A shape's drawing id as the id connectors name (`xml:id`, `draw:id`), and a connector's
    /// type and glued ends (§19.228–19.229), each end's site as the ODF glue point of the shape it
    /// is glued to (an end glued to a shape not written, or to a site ODF has no glue point for,
    /// is left unglued).
    fn conn_attrs(&self, kind: wordcraft_doc::para::ShapeKind, look: &model::BoxLook) -> String {
        let mut a = String::new();
        if kind.is_connector() {
            a.push_str(&format!(" draw:type=\"{}\"", crate::odt::shapes::connector_type(kind)));
        }
        let Some(c) = look.conn.as_deref() else { return a };
        if c.id != 0 {
            a.push_str(&format!(" xml:id=\"{0}shape{1}\" draw:id=\"{0}shape{1}\"", self.prefix, c.id));
        }
        if kind.is_connector() {
            for (which, end) in [("start", c.start), ("end", c.end)] {
                let glue = end.and_then(|e| self.conn_kinds.get(&e.id).and_then(|k| crate::odt::shapes::glue_of_site(*k, e.site)).map(|g| (e.id, g)));
                if let Some((id, g)) = glue {
                    a.push_str(&format!(" draw:{which}-shape=\"{}shape{id}\" draw:{which}-glue-point=\"{g}\"", self.prefix));
                }
            }
        }
        a
    }

    /// The paragraphs of a text box's or shape's text, written apart from the paragraph being
    /// written.
    pub(in crate::odt) fn story(&mut self, blocks: &[FBlock]) -> String {
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.take());
        let mut body = String::new();
        self.story_blocks(blocks, &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        body
    }

    /// An equation as an embedded formula object (ODF 1.3 §10.4.3): a frame in line with the text
    /// pointing at a sub-document whose `content.xml` is the equation's MathML, its frame about
    /// the size the formula takes (LibreOffice scales the formula to the frame).
    pub(in crate::odt) fn formula(&mut self, linear: &str, display: bool, out: &mut String) {
        let (n, name) = self.object_name();
        let mathml = crate::mathml::linear_to_mathml(linear, display);
        self.objects.push((
            name.clone(),
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{mathml}\n"),
            "application/vnd.oasis.opendocument.formula",
        ));
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

/// The name a watermark shape carries (Word's and LibreOffice's: how readers know it).
pub(in crate::odt) const WATERMARK_NAME: &str = "PowerPlusWaterMarkObject";

impl Writer {
    /// The document's watermark as LibreOffice and Word write one in a header: a fontwork shape
    /// (§10.3.2 `draw:custom-shape` whose geometry follows its text, `draw:text-path`) behind the
    /// text, centred on a page whose text area is `area` (width, height; points), turned up 45°
    /// when diagonal, filled with its colour (half transparent when semitransparent). It doesn't
    /// grow to its text: its text is stretched to it.
    pub(in crate::odt) fn watermark(&mut self, wm: &wordcraft_doc::Watermark, page: (f32, f32), area: (f32, f32)) -> String {
        let [w, h] = wm.frame(page.0, page.1);
        let opacity = if wm.semitransparent { " draw:opacity=\"50%\"" } else { "" };
        let style = self.graphic_style(format!(
            " draw:stroke=\"none\" draw:fill=\"solid\" draw:fill-color=\"#{}\"{opacity} style:run-through=\"background\" style:wrap=\"run-through\" style:horizontal-pos=\"center\" style:horizontal-rel=\"page-content\" style:vertical-pos=\"middle\" style:vertical-rel=\"page-content\" style:flow-with-text=\"false\" draw:auto-grow-height=\"false\" draw:auto-grow-width=\"false\"",
            wm.color.hex().to_ascii_lowercase()
        ));
        let (aw, ah) = (if area.0.is_finite() { area.0 } else { w }, if area.1.is_finite() { area.1 } else { h });
        let (x0, y0) = ((aw - w) / 2.0, (ah - h) / 2.0);
        let place =
            if wm.diagonal { crate::odt::frames::transform_attr(x0, y0, w, h, 315.0) } else { format!(" svg:x=\"{}\" svg:y=\"{}\"", pt(x0), pt(y0)) };
        let fmt = model::Fmt { font: Some(std::sync::Arc::from(wm.font.as_str())), color: Some(wm.color), ..model::Fmt::default() };
        let span =
            self.text_style(&fmt).map(|t| format!("<text:span text:style-name=\"{t}\">{}</text:span>", x(&wm.text))).unwrap_or_else(|| x(&wm.text));
        format!(
            "<draw:custom-shape text:anchor-type=\"char\" draw:z-index=\"0\" draw:name=\"{WATERMARK_NAME}\" draw:style-name=\"{style}\" svg:width=\"{}\" svg:height=\"{}\"{place}><text:p>{span}</text:p><draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:text-areas=\"0 0 21600 21600\" draw:text-path=\"true\" draw:type=\"fontwork-plain-text\" draw:enhanced-path=\"M 0 0 L 21600 0 N M 0 21600 L 21600 21600 N\"/></draw:custom-shape>",
            pt(w),
            pt(h)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_never_takes_a_kept_objects_name() {
        // Re-saving a file WordCraft saved: its `Object wc1` is kept as read, so a new object
        // (an edited paragraph's chart or formula) takes `Object wc2`, not the kept one's files.
        let mut w = Writer::with_prefix("wc");
        w.reserved = std::sync::Arc::new(["Object wc1/content.xml".to_string(), "Object wc3".to_string()].into_iter().collect());
        assert_eq!(w.object_name(), (2, "Object wc2".to_string()));
        assert_eq!(w.object_name(), (4, "Object wc4".to_string()));
        // Not `Object wc10` for `Object wc1`'s files: only the name itself or its folder.
        let mut w = Writer::with_prefix("wc");
        w.reserved = std::sync::Arc::new(std::iter::once("Object wc10/content.xml".to_string()).collect());
        assert_eq!(w.object_name().1, "Object wc1");
    }

    #[test]
    fn a_picture_never_takes_a_name_an_earlier_one_took() {
        // Re-saving a file WordCraft saved: its `wcimage1.png` is kept as read, so the first new
        // picture skips to 2, and the second must not take 2 again (a duplicate zip entry fails
        // the save).
        let mut w = Writer::with_prefix("wc");
        w.reserved = std::sync::Arc::new(std::iter::once("Pictures/wcimage1.png".to_string()).collect());
        let img = model::Img {
            data: std::sync::Arc::new(vec![1, 2, 3]),
            ext: "png".into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: None,
            object: None,
            crop: [0.0; 4],
        };
        let names: Vec<String> = (0..3).map(|_| w.picture(&img)).collect();
        assert_eq!(names, ["Pictures/wcimage2.png", "Pictures/wcimage3.png", "Pictures/wcimage4.png"]);
    }
}
