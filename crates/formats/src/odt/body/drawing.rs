//! Frames, drawing shapes and groups in the text (ODF 1.3 §10.3–10.4): pictures, formula objects,
//! text boxes, shapes and groups, each read into the paragraph (or the group) it is in.

use super::{Attrs, Body, Ev, FBlock, Inline, events, get, length, local, make_img, model};

/// An embedded ODF formula sub-document's `content.xml`, read for its `math:math` element
/// (its root, ODF 1.3 Part 3 §2.2.9 B; some writers wrap it in an `office:math`, which isn't
/// it): the first MathML element named `math` in document order, with its
/// subtree, passed to [`crate::mathml::read`]. `None` when there's no such element, it holds
/// nothing, or the XML is malformed. Nesting and element count are bounded like other XML here.
pub(in crate::odt) fn read_formula(xml: &[u8]) -> Option<(String, bool)> {
    use crate::mathml::{El, Item, MAX_NODES, local};
    let mut stack: Vec<El> = Vec::new();
    let mut math: Option<El> = None;
    let mut over = 0usize;
    let mut count = 0usize;
    events(xml, |ev| {
        if math.is_some() {
            return;
        }
        count += 1;
        if count > MAX_NODES * 4 {
            math = stack.drain(..).next();
            return;
        }
        match ev {
            Ev::Start(q, a) => {
                if stack.is_empty() && (local(&q) != "math" || q.starts_with("office:")) {
                    return;
                }
                if stack.len() < wordcraft_doc::math::MAX_DEPTH {
                    stack.push(El { name: q, attrs: a, children: Vec::new() });
                } else {
                    over += 1;
                }
            }
            Ev::End(_) => {
                if over > 0 {
                    over -= 1;
                } else if stack.len() > 1 {
                    if let Some(done) = stack.pop()
                        && let Some(parent) = stack.last_mut()
                    {
                        parent.children.push(Item::El(done));
                    }
                } else if stack.len() == 1 {
                    math = stack.pop();
                }
            }
            Ev::Text(t) => {
                if over == 0
                    && let Some(top) = stack.last_mut()
                {
                    top.children.push(Item::Text(t));
                }
            }
        }
    });
    crate::mathml::read(&math?)
}

/// A frame being read: its width and height, picture bytes, alt text and the equation read from
/// an embedded formula object.
/// A group being read (ODF 1.3 §10.3.15 `draw:g`): its anchor and page, its style, and its members
/// so far.
pub(in crate::odt) struct OpenGroup {
    pub(in crate::odt) anchor: Option<String>,
    /// Its `text:anchor-page-number`.
    pub(in crate::odt) page: Option<String>,
    pub(in crate::odt) style: Option<String>,
    pub(in crate::odt) members: Vec<model::Member>,
}

/// A picture's crop as fractions of its size (left, top, right, bottom) from its `fo:clip`
/// (top, right, bottom, left, in lengths of its original size: its pixels at 96 per inch). No crop
/// when the picture's size can't be read or the crop would leave nothing.
pub(in crate::odt) fn crop_fractions([t, r, b, l]: [f32; 4], data: &[u8]) -> [f32; 4] {
    let Some((pw, ph)) = model::image_px(data).filter(|(w, h)| *w > 0 && *h > 0) else { return [0.0; 4] };
    let (w, h) = (pw as f32 * 0.75, ph as f32 * 0.75);
    let c = [l / w, t / h, r / w, b / h].map(|v| if v.is_finite() { v.clamp(0.0, 0.99) } else { 0.0 });
    if c[0] + c[2] < 0.99 && c[1] + c[3] < 0.99 { c } else { [0.0; 4] }
}

/// Largest size or offset read in a group, points (22 inches).
pub(in crate::odt) const MAX_LEN: f32 = 1584.0;

/// Most shape ids connectors can glue to.
const MAX_CONN_IDS: usize = 10_000;

impl Body<'_> {
    /// End a frame or drawing shape: its picture, equation, text box or shape goes into the
    /// paragraph.
    pub(in crate::odt) fn end_frame(&mut self) {
        if let Some((w, h, img, alt, eq, chart, object)) = self.frame.take() {
            let boxed = self.frame_box.take();
            let mut float = self.frame_float.take();
            if boxed.is_some()
                && self.shape_kind.is_some()
                && let Some(f) = &mut float
            {
                let (h, v) = std::mem::take(&mut self.shape_flip);
                f.flip_h |= h;
                f.flip_v ^= v;
            }
            // Kept when it floats, or turns or mirrors the object in line.
            let float =
                float.filter(|f| f.wrap != wordcraft_doc::para::Wrap::Inline || crate::odt::frames::turned(f) || f.flip_h || f.flip_v || f.y != 0.0);
            if let Some(blocks) = &boxed
                && self.take_watermark(blocks)
            {
                self.frame = None;
                return;
            }
            // In line with no stated size, a picture comes at its own: fitted to the text, as
            // other formats' pictures are. A stated size is the file's.
            let room = self.sec.text_width();
            let says = |v: Option<f32>| v.is_some_and(|v| v.is_finite() && v > 0.0);
            // (Still in line when raised, turned or mirrored.)
            let in_line = float.as_ref().is_none_or(|f| f.wrap == wordcraft_doc::para::Wrap::Inline);
            let fit = (!says(w) && !says(h) && in_line && room.is_finite() && room > 0.0).then_some(room);
            let inline = match (boxed, eq) {
                (Some(blocks), _) => {
                    let mut look = std::mem::take(&mut self.frame_look);
                    // Without a height it grows from nothing to its text.
                    let kind = self.shape_kind.take().unwrap_or(wordcraft_doc::para::ShapeKind::TextBox);
                    // A custom shape's geometry as read, kept with the path worked out from it.
                    let geometry = self.shape_geometry.take();
                    if kind == wordcraft_doc::para::ShapeKind::Freeform || kind == wordcraft_doc::para::ShapeKind::Rectangle {
                        look.geometry = geometry;
                    }
                    // A shape whose paragraphs are all empty (LibreOffice writes `<text:p/>` in
                    // each) holds no text.
                    let empty = blocks.iter().all(|b| matches!(b, FBlock::Para(p) if p.inlines.is_empty()));
                    let blocks = if kind != wordcraft_doc::para::ShapeKind::TextBox && empty { Vec::new() } else { blocks };
                    let path = self.shape_path.take().filter(|_| kind == wordcraft_doc::para::ShapeKind::Freeform);
                    let (w, h) = (w.unwrap_or(72.0), h.unwrap_or(1.0));
                    self.note_place(kind, (w, h), &look, float.as_ref());
                    Some(Inline::Shape { kind, w, h, float: float.map(Box::new), blocks, look, path })
                }
                (None, Some((linear, display))) => Some(Inline::Equation { linear, display }),
                (None, None) if let Some(spec) = chart => Some(Inline::Chart(model::Chart {
                    spec: std::sync::Arc::new(spec),
                    w: w.unwrap_or(360.0),
                    h: h.unwrap_or(216.0),
                    alt: alt.trim().to_string(),
                    float: float.map(Box::new),
                    source: object.map(std::sync::Arc::new),
                })),
                // An object kept whole: shown by its replacement picture when WordCraft can show
                // it, else by a placeholder.
                (None, None) if let Some(mut o) = object => {
                    if o.replacement.is_none() {
                        o.replacement = img.clone().map(std::sync::Arc::new);
                    }
                    let shown = img
                        .and_then(|data| picture(data, w, h, alt.trim()))
                        .or_else(|| crate::odt::objects::placeholder(&o.mime).and_then(|p| make_img(p, w, h, alt.trim())));
                    shown.map(|mut img| {
                        fit_to(&mut img, fit);
                        img.float = float.map(Box::new);
                        img.object = Some(std::sync::Arc::new(o));
                        Inline::Image(img)
                    })
                }
                (None, None) => img.and_then(|data| picture(data, w, h, alt.trim())).map(|mut img| {
                    fit_to(&mut img, fit);
                    img.float = float.map(Box::new);
                    if let Some(clip) = self.frame_clip.take() {
                        img.crop = crop_fractions(clip, &img.data);
                    }
                    Inline::Image(img)
                }),
            };
            if let Some(inline) = inline {
                // A group's member goes into the group, at its place in it.
                if let Some(((x, y), g)) = self.member_at.take().zip(self.group.as_mut().filter(|_| self.note.is_none())) {
                    if g.members.len() < wordcraft_doc::para::MAX_GROUP_CHILDREN {
                        g.members.push(model::Member { x, y, inline });
                    }
                } else {
                    self.push_inline(inline);
                }
            }
        }
        self.frame = None;
    }

    /// The shape ending is a watermark (see [`Self::start_shape`]) holding `blocks`: the
    /// document's watermark, from its text, font, colour, size and turn, and not a shape.
    fn take_watermark(&mut self, blocks: &[FBlock]) -> bool {
        let Some(((w, h), rot, color, see_through, true)) = self.watermark_shape.take() else { return false };
        let mut text = String::new();
        let mut font = None;
        for b in blocks {
            if let FBlock::Para(p) = b {
                for i in &p.inlines {
                    if let Inline::Text(t, f) = i {
                        text.push_str(t);
                        font = font.or_else(|| f.font.as_deref().map(str::to_string));
                    }
                }
            }
        }
        let text: String = text.trim().chars().take(200).collect();
        if text.is_empty() {
            return false;
        }
        let turned = rot.rem_euclid(360.0);
        let base = wordcraft_doc::Watermark::default();
        self.watermark = Some(wordcraft_doc::Watermark {
            text,
            font: font.unwrap_or(base.font),
            color: color.unwrap_or(base.color),
            diagonal: turned > 1.0 && turned < 359.0,
            semitransparent: see_through,
            size: Some([w, h]).filter(|[w, h]| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0),
        });
        true
    }

    /// A shape's drawing id (its `xml:id` or `draw:id`) and, for a connector, its glued ends (§19.228
    /// `draw:start-shape`, §19.229 `draw:start-glue-point`…), their glue points as written until
    /// the paragraph ends (see [`Self::glue_connectors`]). `None` for a shape with neither.
    fn conn_of(&mut self, a: &Attrs) -> Option<model::Conn> {
        let mut id_of = |name: &str| -> u32 {
            let next = u32::try_from(self.conn_ids.len()).unwrap_or(u32::MAX).saturating_add(1);
            match self.conn_ids.get(name) {
                Some(id) => *id,
                None if self.conn_ids.len() < MAX_CONN_IDS => {
                    self.conn_ids.insert(name.to_string(), next);
                    next
                }
                None => 0,
            }
        };
        let id = get(a, "xml:id").or_else(|| get(a, "draw:id")).map(str::trim).filter(|n| !n.is_empty()).map_or(0, &mut id_of);
        let mut end = |shape: &str, glue: &str| {
            let target = get(a, shape).map(str::trim).filter(|n| !n.is_empty()).map(&mut id_of).filter(|id| *id != 0)?;
            let site = get(a, glue).and_then(|g| g.trim().parse::<u32>().ok()).filter(|g| *g <= wordcraft_doc::connector::MAX_SITE)?;
            Some(wordcraft_doc::connector::ConnEnd { id: target, site })
        };
        let (start, finish) = (end("draw:start-shape", "draw:start-glue-point"), end("draw:end-shape", "draw:end-glue-point"));
        (id != 0 || start.is_some() || finish.is_some()).then_some(model::Conn { id, start, end: finish })
    }

    /// Where a shape with a drawing id (`look`) sits, for connectors glued to it.
    pub(in crate::odt) fn note_place(
        &mut self,
        kind: wordcraft_doc::para::ShapeKind,
        (w, h): (f32, f32),
        look: &model::BoxLook,
        float: Option<&wordcraft_doc::para::Float>,
    ) {
        let Some(c) = look.conn.as_deref().filter(|c| c.id != 0) else { return };
        if self.shape_places.len() >= MAX_CONN_IDS {
            return;
        }
        // A group's member (placed in the group, not on the paragraph) or a shape in line: known
        // by its kind only, never in the paragraph's coordinates.
        let seq = if self.member_at.is_some() { usize::MAX } else { self.para_seq };
        let f = float.cloned().unwrap_or_default();
        self.shape_places.insert(c.id, (kind, [f.x, f.y, w, h], f, if float.is_some() { seq } else { usize::MAX }));
    }

    /// The connectors of paragraph `p` glued to shapes (see [`Self::conn_of`]): each end's glue
    /// point made WordCraft's site of the shape it is glued to (an end glued to a shape not read,
    /// or to a glue point of the shape's own, is unglued); and a connector glued at both ends to
    /// shapes anchored in this paragraph, placed alike, runs between their sites. (A connector's
    /// own coordinates are in LibreOffice's drawing layer, not its anchor's.)
    pub(in crate::odt) fn glue_connectors(&self, p: &mut model::Para) {
        for i in &mut p.inlines {
            self.glue_connector(i, 0);
        }
    }

    /// Connector `i` of a paragraph or a group (`depth` deep in groups) glued to shapes (see
    /// [`Self::glue_connectors`]).
    fn glue_connector(&self, i: &mut Inline, depth: usize) {
        use crate::odt::shapes::site_of_glue;
        if let Inline::Group { members, .. } = i {
            if depth < model::MAX_GROUP_DEPTH {
                for m in members {
                    self.glue_connector(&mut m.inline, depth + 1);
                }
            }
            return;
        }
        let Inline::Shape { kind, w, h, float, look, .. } = i else { return };
        let Some(c) = look.conn.as_deref_mut().filter(|_| kind.is_connector()) else { return };
        let resolve = |e: Option<wordcraft_doc::connector::ConnEnd>| {
            let e = e?;
            let place = self.shape_places.get(&e.id)?;
            site_of_glue(place.0, e.site).map(|site| (wordcraft_doc::connector::ConnEnd { id: e.id, site }, place))
        };
        let (start, end) = (resolve(c.start), resolve(c.end));
        (c.start, c.end) = (start.map(|(e, _)| e), end.map(|(e, _)| e));
        let (Some((s, sp)), Some((e, ep)), Some(f)) = (start, end, float.as_deref_mut()) else { return };
        let same = |pl: &(wordcraft_doc::para::ShapeKind, [f32; 4], wordcraft_doc::para::Float, usize)| {
            pl.3 == self.para_seq && pl.2.h_rel == f.h_rel && pl.2.v_rel == f.v_rel
        };
        if !(same(sp) && same(ep)) {
            return;
        }
        let point = |pl: &(wordcraft_doc::para::ShapeKind, [f32; 4], wordcraft_doc::para::Float, usize), site| {
            wordcraft_doc::connector::site_point(pl.0, pl.1, pl.2.spin(), site)
        };
        let (Some((x1, y1)), Some((x2, y2))) = (point(sp, s.site), point(ep, e.site)) else { return };
        (f.x, f.y, f.flip_h, f.flip_v) = (x1.min(x2), y1.min(y2), x2 < x1, y2 < y1);
        (*w, *h) = ((x2 - x1).abs().min(MAX_LEN), (y2 - y1).abs().min(MAX_LEN));
    }

    /// A frame or shape `w` × `h` (attributes `a`) begins: in a group (not in a shape's text), where
    /// it sits in it, from its `svg:x`/`svg:y` or `draw:transform` (a line's from its ends, `line`).
    pub(in crate::odt) fn member_start(&mut self, a: &Attrs, w: f32, h: f32, line: Option<(Option<f32>, Option<f32>)>) {
        self.member_rot = None;
        if self.group.is_none() || self.note.is_some() {
            return;
        }
        let len = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(-MAX_LEN, MAX_LEN));
        let at = match (line, get(a, "draw:transform").and_then(|t| crate::odt::frames::transform(t, w, h))) {
            (Some((x, y)), _) => (x.unwrap_or(0.0), y.unwrap_or(0.0)),
            (None, Some((x, y, rot))) => {
                self.member_rot = Some(rot);
                (x, y)
            }
            (None, None) => (len("svg:x").unwrap_or(0.0), len("svg:y").unwrap_or(0.0)),
        };
        self.member_at = Some(at);
    }

    /// The group being read ends: its members, placed from the corner of the box around them,
    /// become one object anchored where the group is.
    pub(in crate::odt) fn end_group(&mut self) {
        let Some(g) = self.group.take() else { return };
        let size = |i: &Inline| match i {
            Inline::Shape { w, h, .. } => (*w, *h),
            Inline::Image(img) => (img.w, img.h),
            Inline::Group { w, h, .. } => (*w, *h),
            _ => (0.0, 0.0),
        };
        let fin = |v: f32| if v.is_finite() { v.clamp(-MAX_LEN, MAX_LEN) } else { 0.0 };
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for m in &g.members {
            let (w, h) = size(&m.inline);
            (x0, y0) = (x0.min(fin(m.x)), y0.min(fin(m.y)));
            (x1, y1) = (x1.max(fin(m.x + w)), y1.max(fin(m.y + h)));
        }
        if g.members.is_empty() {
            return;
        }
        let members = g.members.into_iter().map(|m| model::Member { x: fin(m.x) - x0, y: fin(m.y) - y0, ..m }).collect();
        let graphic = self.styles.shape_graphic(g.style.as_deref());
        let mut float =
            crate::odt::frames::float_of(g.anchor.as_deref(), Some(&crate::odt::writer::pt(x0)), Some(&crate::odt::writer::pt(y0)), &graphic);
        float.page = crate::odt::frames::page_of(g.anchor.as_deref(), g.page.as_deref(), &graphic);
        let float = (float.wrap != wordcraft_doc::para::Wrap::Inline).then(|| Box::new(float));
        let fit = |v: f32| if v.is_finite() { v.clamp(1.0, MAX_LEN) } else { 1.0 };
        self.push_inline(Inline::Group { w: fit(x1 - x0), h: fit(y1 - y0), float, members });
    }

    /// Begin a drawing shape (ODF 1.3 §10.3): its size, placement and look, and its text read
    /// as a text box's. A custom shape's kind comes from its geometry, read inside it.
    pub(in crate::odt) fn start_shape(&mut self, q: &str, a: &Attrs) {
        let mut kind = crate::odt::shapes::element_kind(local(q));
        if kind.is_connector() {
            kind = crate::odt::shapes::connector_kind(get(a, "draw:type"));
        }
        let g = self.styles.shape_graphic(get(a, "draw:style-name"));
        let (w, h, x, y, (flip_h, flip_v)) = crate::odt::shapes::geometry(kind, a);
        let mut float = if kind.is_open() {
            // A line is placed by its ends.
            let at = |v: Option<f32>| v.map(crate::odt::writer::pt);
            let mut f = crate::odt::frames::float_of(get(a, "text:anchor-type"), at(x).as_deref(), at(y).as_deref(), &g);
            f.z = crate::odt::frames::z_of(a);
            f.page = crate::odt::frames::page_of(get(a, "text:anchor-type"), get(a, "text:anchor-page-number"), &g);
            f
        } else {
            crate::odt::frames::place(a, w, h, &g, self.text_size())
        };
        (float.flip_h, float.flip_v) = (flip_h, flip_v);
        self.shape_flip = (false, false);
        self.member_start(a, w, h, kind.is_open().then_some((x, y)));
        if let Some(rot) = self.member_rot.take() {
            float.rot = rot;
        }
        self.frame_look = g.shape_look(kind, &self.styles.defs);
        self.frame_look.conn = self.conn_of(a).map(Box::new);
        // Word's and LibreOffice's watermark: a shape of this name whose geometry follows its text.
        self.watermark_shape = get(a, "draw:name").filter(|n| n.starts_with(crate::odt::writer::WATERMARK_NAME)).map(|_| {
            let (color, see_through) = g.watermark_look();
            ((w, h), float.rot, color, see_through, false)
        });
        self.shape_path = crate::odt::shapes::freeform(local(q), a, w, h).map(std::sync::Arc::new);
        self.shape_kind = Some(kind);
        let frame = (Some((Some(w), Some(h), None, String::new(), None, None, None)), Some(float), std::mem::take(&mut self.frame_look));
        self.start_note(a);
        if let Some(n) = &mut self.note {
            n.text_box = Some(frame);
        }
    }
}

/// Whether WordCraft can show these bytes as a picture (see the function below).
pub(in crate::odt) fn readable_picture(data: &[u8]) -> bool {
    wordcraft_metafile::is_metafile(data) || model::sniff_image(data).is_some()
}

/// A picture from a package's bytes: a raster image, or a Windows metafile (WMF, EMF: LibreOffice
/// keeps clip art and logos so), sized `w` × `h` (points) when the frame says.
pub(in crate::odt) fn picture(data: Vec<u8>, w: Option<f32>, h: Option<f32>, alt: &str) -> Option<model::Img> {
    // What the frame states, if it could be a size (far past any page, kept from overflowing).
    let stated = |v: Option<f32>| v.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(1e6));
    let (w, h) = (stated(w), stated(h));
    let mut img = if wordcraft_metafile::is_metafile(&data) {
        // An EMF starts with its header record, type 1.
        let ext = if data.get(..4) == Some(&[1, 0, 0, 0]) { "emf" } else { "wmf" };
        // Its own size, when the frame gives none: a placeable WMF's or an EMF's frame, points.
        let natural = if w.is_some() && h.is_some() { None } else { metafile_size(&data) };
        model::sized_img(std::sync::Arc::new(data), ext, natural, w, h, alt)
    } else {
        make_img(data, w, h, alt)?
    };
    // A size the frame states is the file's, however large (a picture across a whole page): only
    // a picture with none is shrunk to a column. Stated on one side, the other follows the
    // picture's proportions.
    let (w, h) = match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * height_per_width(&img.data)),
        (None, Some(h)) => (h / height_per_width(&img.data), h),
        (None, None) => return Some(img),
    };
    // Past what a document holds, it shrinks in proportion.
    let (w, h) = match w.max(h) {
        most if most <= 1584.0 => (w, h),
        _ if w >= h => (1584.0, h * 1584.0 / w),
        _ => (w * 1584.0 / h, 1584.0),
    };
    (img.w, img.h) = (w.max(1.0), h.max(1.0));
    Some(img)
}

/// `img` shrunk in proportion to `room` points wide, when it's given and the picture is wider.
fn fit_to(img: &mut model::Img, room: Option<f32>) {
    if let Some(room) = room.filter(|room| img.w > *room) {
        img.h = (img.h * room / img.w).max(1.0);
        img.w = room.max(1.0);
    }
}

/// A picture's height for each point of its width, by its own size, before anything caps it (1
/// when it doesn't say).
fn height_per_width(data: &[u8]) -> f32 {
    let own = if wordcraft_metafile::is_metafile(data) { metafile_size(data) } else { model::image_px(data).map(|(w, h)| (w as f32, h as f32)) };
    own.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0).map_or(1.0, |(w, h)| h / w)
}

/// A metafile's own size, points, from its header: a placeable WMF's bounding box in its units
/// per inch, or an EMF's frame (`rclFrame`, 0.01 mm). `None` when the header doesn't say.
fn metafile_size(d: &[u8]) -> Option<(f32, f32)> {
    let i16_at = |o: usize| d.get(o..o + 2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])));
    let i32_at = |o: usize| d.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32);
    let (w, h) = if d.starts_with(&[0xd7, 0xcd, 0xc6, 0x9a]) {
        let inch = d.get(14..16).map(|b| f32::from(u16::from_le_bytes([b[0], b[1]]))).filter(|v| *v > 0.0)?;
        let (l, t, r, b) = (i16_at(6)?, i16_at(8)?, i16_at(10)?, i16_at(12)?);
        ((r - l).abs() / inch * 72.0, (b - t).abs() / inch * 72.0)
    } else if d.get(..4) == Some(&[1, 0, 0, 0]) {
        let (l, t, r, b) = (i32_at(24)?, i32_at(28)?, i32_at(32)?, i32_at(36)?);
        let pt = 72.0 / 2540.0;
        ((r - l).abs() * pt, (b - t).abs() * pt)
    } else {
        return None;
    };
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w.min(1584.0), h.min(1584.0)))
}
