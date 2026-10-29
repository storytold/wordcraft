use super::text::GlyphPaint;
use super::{
    Draw, Exporter, Fill, FillRule, LineCap, LineJoin, LineStyle, PathBuilder, Rect, Rgb, ShapeKind, Size, Stroke, StrokeDash, Surface, Transform,
    append_path, fill, grown, krilla, norm, ok, paint_fill, pdf_line_cap, rect_path, rgb, shape_effects, shape_path, vector_picture,
};
use crate::picture::{image_decodes, svg_picture};

impl Exporter<'_> {
    /// Paint one display item on the PDF surface in its current point-coordinate system.
    /// Nested clips and turns are pushed and popped around their child items. Shape and path
    /// outlines set their own dash and cap state, then clear their fill and stroke; ink retains
    /// round caps independently of the optional shape-outline setting.
    pub(super) fn draw(&mut self, s: &mut Surface, d: &Draw) {
        match d {
            Draw::Figure { draws, .. } => {
                for d in draws {
                    self.draw(s, d);
                }
            }
            Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, x_scale, .. } => {
                let paint = GlyphPaint { color: *color, alpha: *alpha, bold: *synth_bold, italic: *synth_italic };
                match glyphs.first().map(|g| g.1).filter(|_| x_scale.is_finite() && *x_scale > 0.0 && (x_scale - 1.0).abs() > 1e-4) {
                    // Scaled letters (`w:w`): drawn stretched across from the run's start, their
                    // positions brought back so the stretch puts each where the layout has it.
                    Some(x0) => {
                        let k = *x_scale;
                        let moved: Vec<(u32, f32, f32)> = glyphs.iter().map(|&(g, x, y)| (g, x0 + (x - x0) / k, y)).collect();
                        s.push_transform(&Transform::from_row(k, 0.0, 0.0, 1.0, x0 * (1.0 - k), 0.0));
                        self.glyphs(s, face, *size, &moved, paint, (text, ranges));
                        s.pop();
                    }
                    None => self.glyphs(s, face, *size, glyphs, paint, (text, ranges)),
                }
                if let Some(l) = link {
                    self.link_rect(face, *size, glyphs, l);
                }
            }
            Draw::Fill { rect, color, alpha, .. } => {
                if let Some(p) = rect_path(rect) {
                    s.set_stroke(None);
                    s.set_fill(Some(fill(*color, *alpha)));
                    s.draw_path(&p);
                    s.set_fill(None);
                }
            }
            Draw::Line { x0, y0, x1, y1, width, color, stroke, alpha, .. } => self.line(s, (*x0, *y0, *x1, *y1), *width, *color, *stroke, *alpha),
            Draw::Image { rect, media, crop, alpha, .. } => self.picture(s, rect, media, crop, *alpha),
            Draw::Path { segs, fill: f, stroke, stroke_width, dash, cap } => {
                let mut pb = PathBuilder::new();
                append_path(&mut pb, &wordcraft_layout::display::seg_path(segs));
                let Some(p) = pb.finish() else { return };
                s.set_fill(f.as_ref().map(paint_fill));
                s.set_stroke(stroke.map(|c| Stroke {
                    paint: rgb::Color::new(c.0, c.1, c.2).into(),
                    // Width 0 (or none) is a hairline.
                    width: if stroke_width.is_finite() && *stroke_width > 0.0 { stroke_width.clamp(0.25, 200.0) } else { 0.75 },
                    dash: (!dash.is_empty()).then(|| StrokeDash { array: dash.clone(), offset: 0.0 }),
                    line_cap: pdf_line_cap(*cap),
                    ..Default::default()
                }));
                if f.is_some() || stroke.is_some() {
                    s.draw_path(&p);
                }
                s.set_fill(None);
                s.set_stroke(None);
            }
            Draw::Ink { pts, color, width, alpha } => {
                let mut pb = PathBuilder::new();
                let mut used = 0usize;
                let mut last = (0.0, 0.0);
                for &(x, y) in pts.iter().filter(|(x, y)| ok(*x) && ok(*y)) {
                    if used > 0 {
                        pb.line_to(x, y);
                    } else {
                        pb.move_to(x, y);
                    }
                    used += 1;
                    last = (x, y);
                }
                // A tap (one usable point): a zero-length line, which round caps draw as a dot.
                if used == 1 {
                    pb.line_to(last.0, last.1);
                }
                let Some(p) = pb.finish() else { return };
                s.set_fill(None);
                s.set_stroke(Some(Stroke {
                    paint: rgb::Color::new(color.0, color.1, color.2).into(),
                    width: if width.is_finite() { width.clamp(0.25, 200.0) } else { 1.0 },
                    opacity: norm(*alpha),
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Default::default()
                }));
                s.draw_path(&p);
                s.set_stroke(None);
            }
            Draw::Shape { rect, kind, fill: f, stroke, stroke_width, effects, corner, dash, cap } => {
                let Some(p) = shape_path(*kind, rect, *corner) else { return };
                let can_fill = *kind != ShapeKind::Line;
                let fx = effects.sanitized();
                if !fx.is_empty() && (f.is_some() && can_fill || stroke.is_some()) {
                    shape_effects(s, (*kind, *corner), rect, &fx, (f.is_some() && can_fill, stroke.is_some(), *stroke_width));
                }
                // Soft edges: the fill fades out toward the outline (which fades with it).
                if let (Some(rad), Some(c), true) = (fx.soft_edge, f.as_ref(), can_fill) {
                    let rad = rad.min(rect.w.min(rect.h) / 2.0).max(0.0);
                    s.set_stroke(None);
                    for (g, a) in wordcraft_doc::effects::bands(-rad, 0.0, 1.0) {
                        if let Some(p) = shape_path(*kind, &grown(rect, g, 0.0, 0.0), *corner) {
                            s.set_fill(Some(paint_fill(&c.faded(a))));
                            s.draw_path(&p);
                        }
                    }
                    s.set_fill(None);
                    return;
                }
                s.set_fill(f.as_ref().filter(|_| can_fill).map(paint_fill));
                s.set_stroke(stroke.map(|c| Stroke {
                    paint: rgb::Color::new(c.0, c.1, c.2).into(),
                    width: if stroke_width.is_finite() { stroke_width.clamp(0.25, 200.0) } else { 0.75 },
                    dash: (!dash.is_empty()).then(|| StrokeDash { array: dash.clone(), offset: 0.0 }),
                    line_cap: pdf_line_cap(*cap),
                    ..Default::default()
                }));
                if f.is_some() && can_fill || stroke.is_some() {
                    s.draw_path(&p);
                }
                s.set_fill(None);
                s.set_stroke(None);
            }
            Draw::Art { path, fx, color, alpha, .. } => {
                for l in wordcraft_layout::wordart::art_layers(path, fx, *color, *alpha) {
                    let mut pb = PathBuilder::new();
                    append_path(&mut pb, &l.path);
                    let Some(p) = pb.finish() else { continue };
                    let Fill { paint, opacity, .. } = paint_fill(&l.paint);
                    match l.stroke {
                        Some(w) => {
                            s.set_fill(None);
                            s.set_stroke(Some(Stroke {
                                paint,
                                width: w.clamp(0.1, 400.0),
                                opacity,
                                line_join: LineJoin::Round,
                                ..Default::default()
                            }));
                        }
                        None => {
                            s.set_stroke(None);
                            s.set_fill(Some(Fill { paint, opacity, rule: FillRule::NonZero }));
                        }
                    }
                    s.draw_path(&p);
                }
                s.set_fill(None);
                s.set_stroke(None);
            }
            Draw::Mark { .. } | Draw::MarkText { .. } => {}
            Draw::Clip { rect, items } => {
                let Some(clip) = rect_path(rect) else { return };
                s.push_clip_path(&clip, &FillRule::NonZero);
                for it in items {
                    self.draw(s, it);
                }
                s.pop();
            }
            Draw::Turned { x, y, turn, items } => {
                if !ok(*x) || !ok(*y) {
                    return;
                }
                let [a, b, c, d, e, f] = Draw::turn_matrix(*turn, *x, *y);
                s.push_transform(&Transform::from_row(a, b, c, d, e, f));
                for it in items {
                    self.draw(s, it);
                }
                s.pop();
            }
            Draw::Rotated { cx, cy, spin, items } => {
                if !ok(*cx) || !ok(*cy) {
                    return;
                }
                let [a, b, c, d, e, f] = spin.matrix(*cx, *cy);
                s.push_transform(&Transform::from_row(a, b, c, d, e, f));
                for it in items {
                    self.draw(s, it);
                }
                s.pop();
            }
        }
    }

    fn line(&mut self, s: &mut Surface, (x0, y0, x1, y1): (f32, f32, f32, f32), width: f32, c: Rgb, style: LineStyle, alpha: f32) {
        if ![x0, y0, x1, y1].iter().all(|v| ok(*v)) {
            return;
        }
        let w = if width.is_finite() { width.clamp(0.1, 100.0) } else { 0.5 };
        let paint: krilla::paint::Paint = rgb::Color::new(c.0, c.1, c.2).into();
        let stroke = |width: f32, dash: Option<StrokeDash>, cap: LineCap| Stroke {
            paint: paint.clone(),
            width,
            opacity: norm(alpha),
            dash,
            line_cap: cap,
            ..Default::default()
        };
        let seg = |s: &mut Surface, dx: f32, dy: f32| {
            let mut pb = PathBuilder::new();
            pb.move_to(x0 + dx, y0 + dy);
            pb.line_to(x1 + dx, y1 + dy);
            if let Some(p) = pb.finish() {
                s.draw_path(&p);
            }
        };
        s.set_fill(None);
        match style {
            LineStyle::Solid => {
                s.set_stroke(Some(stroke(w, None, LineCap::Butt)));
                seg(s, 0.0, 0.0);
            }
            LineStyle::Dotted | LineStyle::Dashed | LineStyle::BorderDotted | LineStyle::BorderDashed | LineStyle::BorderDotDash => {
                let array = style.dashes().unwrap_or(&[]).iter().map(|k| k * w).collect();
                s.set_stroke(Some(stroke(w, Some(StrokeDash { array, offset: 0.0 }), LineCap::Butt)));
                seg(s, 0.0, 0.0);
            }
            LineStyle::Double => {
                // Two thinner lines either side of the centre line.
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (nx, ny) = (-dy / len * w * 0.9, dx / len * w * 0.9);
                s.set_stroke(Some(stroke(w * 0.6, None, LineCap::Butt)));
                seg(s, nx, ny);
                seg(s, -nx, -ny);
            }
            LineStyle::Triple => {
                // Three thin lines, spaced as a double's, across the line's direction.
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (nx, ny) = (-dy / len * w * 1.8, dx / len * w * 1.8);
                s.set_stroke(Some(stroke(w * 0.6, None, LineCap::Butt)));
                seg(s, nx, ny);
                seg(s, 0.0, 0.0);
                seg(s, -nx, -ny);
            }
            LineStyle::Wave => {
                let amp = (w * 1.2).max(0.8);
                let step = amp * 2.0;
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len = (dx * dx + dy * dy).sqrt();
                if len >= 1e-3 {
                    let (ux, uy) = (dx / len, dy / len);
                    let (nx, ny) = (-uy, ux);
                    let mut pb = PathBuilder::new();
                    pb.move_to(x0, y0);
                    let mut t = 0.0;
                    let mut up = true;
                    while t < len && t < 20_000.0 {
                        let nt = (t + step).min(len);
                        let mid_t = (t + nt) / 2.0;
                        let sign = if up { -1.0 } else { 1.0 };
                        let cx = x0 + ux * mid_t + nx * amp * sign;
                        let cy = y0 + uy * mid_t + ny * amp * sign;
                        let ex = x0 + ux * nt;
                        let ey = y0 + uy * nt;
                        pb.quad_to(cx, cy, ex, ey);
                        t = nt;
                        up = !up;
                    }
                    s.set_stroke(Some(stroke(w * 0.8, None, LineCap::Round)));
                    if let Some(p) = pb.finish() {
                        s.draw_path(&p);
                    }
                }
            }
        }
        s.set_stroke(None);
    }

    /// The page's picture or texture fill: tiled at 96 dpi for a `tile` fill (when that is a sane
    /// number of tiles), else stretched over the page.
    pub(super) fn page_picture(&mut self, s: &mut Surface, pic: &wordcraft_doc::PagePicture, w: f32, h: f32) {
        const MAX_TILES: f32 = 4096.0;
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return;
        }
        let dims = self
            .doc
            .media
            .get(&pic.media)
            .and_then(|b| image::ImageReader::new(std::io::Cursor::new(b.as_slice())).with_guessed_format().ok()?.into_dimensions().ok());
        if let Some((pw, ph)) = dims.filter(|_| pic.fill_type.eq_ignore_ascii_case("tile")) {
            let (tw, th) = (pw as f32 * 0.75, ph as f32 * 0.75);
            if tw >= 1.0 && th >= 1.0 && (w / tw).ceil() * (h / th).ceil() <= MAX_TILES {
                let mut y = 0.0;
                while y < h {
                    let mut x = 0.0;
                    while x < w {
                        self.picture(s, &Rect::new(x, y, tw, th), &pic.media, &[0.0; 4], 1.0);
                        x += tw;
                    }
                    y += th;
                }
                return;
            }
        }
        self.picture(s, &Rect::new(0.0, 0.0, w, h), &pic.media, &[0.0; 4], 1.0);
    }

    fn picture(&mut self, s: &mut Surface, rect: &Rect, media: &str, crop: &[f32; 4], alpha: f32) {
        let Some(clip) = rect_path(rect) else { return };
        if let Some(v) = self.vector(media) {
            vector_picture(self, s, &v, rect, crop, &clip, alpha);
            return;
        }
        if let Some(Some(svg)) = self.svg(media)
            && svg_picture(s, &svg, rect, crop, &clip, alpha, || {
                *self
                    .svg_images_ok
                    .entry(media.to_string())
                    .or_insert_with(|| svg.raster_images().iter().all(|(kind, data)| image_decodes(kind, data)))
            })
            .is_some()
        {
            return;
        }
        let Some(img) = self.image(media) else {
            s.set_stroke(None);
            s.set_fill(Some(fill(Rgb(0xD0, 0xD0, 0xD0), 1.0)));
            s.draw_path(&clip);
            s.set_fill(None);
            return;
        };
        let [cl, ct, vis_w, vis_h] = wordcraft_metafile::crop_window(*crop).map(|v| v as f32);
        let (fw, fh) = (rect.w / vis_w, rect.h / vis_h);
        let Some(size) = Size::from_wh(fw.max(0.01), fh.max(0.01)) else { return };
        let fade = alpha < 0.999;
        if fade {
            s.push_opacity(norm(alpha));
        }
        s.push_clip_path(&clip, &FillRule::NonZero);
        s.push_transform(&Transform::from_translate(rect.x - cl * fw, rect.y - ct * fh));
        s.draw_image(img, size);
        s.pop();
        s.pop();
        if fade {
            s.pop();
        }
    }
}
