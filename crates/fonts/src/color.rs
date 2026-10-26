//! Colour glyphs (#1269): emoji and other glyphs drawn from a font's colour tables instead of a
//! plain outline. COLR v0/v1 paint graphs become a flat list of [`ColorOp`]s; bitmap strikes
//! (`sbix`, `CBDT`) become a [`ColorBitmap`] placed in font units. The renderer turns either into
//! fills or an image; nothing here depends on a rasteriser.
//!
//! Fonts are hostile input: every lookup is checked, the op list and bitmap sizes are capped, and
//! a paint graph that fails part way (skrifa rejects cycles and over-deep graphs) yields `None`, so
//! the caller falls back to the outline.

use kurbo::{Affine, BezPath, Point, Rect};
use skrifa::color::{Brush, ColorGlyphCollection, ColorPainter, ColorPalettes, ColorStop, Transform};
use skrifa::raw::types::BoundingBox;
use skrifa::{GlyphId, Tag};

pub use skrifa::color::{CompositeMode, Extend};

use crate::FontFace;

/// Upper bound on the ops one colour glyph may expand to.
pub const MAX_OPS: usize = 20_000;
/// Upper bound on the gradient stops and clip-path elements one colour glyph may copy in total: a
/// paint graph can reference one long stop list or outline from every op.
pub const MAX_PAYLOAD: usize = 1 << 16;
/// Upper bound on a bitmap glyph's width and height in pixels.
pub const MAX_BITMAP_SIDE: u32 = 2048;

/// A colour from the font's palette, or the text colour (palette index 0xFFFF).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlyphColor {
    /// Straight (not premultiplied) RGBA.
    Rgba([u8; 4]),
    /// The run's text colour, with this alpha multiplier.
    Foreground(f32),
}

/// A gradient stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub offset: f32,
    pub color: GlyphColor,
}

/// What a [`ColorOp::Fill`] paints the current clip with. Coordinates are in the space set by the
/// enclosing transforms.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorPaint {
    Solid(GlyphColor),
    Linear {
        p0: Point,
        p1: Point,
        stops: Vec<Stop>,
        extend: Extend,
    },
    Radial {
        c0: Point,
        r0: f32,
        c1: Point,
        r1: f32,
        stops: Vec<Stop>,
        extend: Extend,
    },
    /// Angles in degrees, clockwise, as skrifa reports them.
    Sweep {
        c0: Point,
        start_angle: f32,
        end_angle: f32,
        stops: Vec<Stop>,
        extend: Extend,
    },
}

/// One drawing step of a COLR glyph, in font units with y up. Pushes and pops are balanced.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorOp {
    /// Concatenate a transform onto the current one.
    PushTransform(Affine),
    PopTransform,
    /// Intersect the clip with this path (in the current transform's space).
    PushClip(BezPath),
    PopClip,
    /// Open a layer that composites with this mode when popped.
    PushLayer(CompositeMode),
    PopLayer,
    /// Fill the current clip.
    Fill(ColorPaint),
}

/// The encoded pixels of a bitmap glyph.
#[derive(Clone, Debug, PartialEq)]
pub enum BitmapBytes {
    Png(Vec<u8>),
    /// Premultiplied BGRA rows, `width * height * 4` bytes.
    Bgra(Vec<u8>),
}

/// A bitmap glyph: its pixels and where they go, in font units with y down from the pen position
/// (the same space as [`FontFace::glyph_outline`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorBitmap {
    pub data: BitmapBytes,
    pub width: u32,
    pub height: u32,
    pub rect: Rect,
    /// The strike's pixels per em.
    pub ppem: f32,
}

/// A glyph drawn from the font's colour data.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorGlyph {
    Layers(Vec<ColorOp>),
    Bitmap(ColorBitmap),
}

impl FontFace {
    /// Does this face carry colour glyph tables (COLR, sbix or CBDT)? Cheap: a table lookup.
    pub fn has_color_glyphs(&self) -> bool {
        self.skrifa().is_some_and(|f| [b"COLR", b"sbix", b"CBDT"].iter().any(|t| f.table_data(Tag::new(t)).is_some()))
    }

    /// Glyph `gid` drawn in colour: its COLR paint (preferred: it scales), else the bitmap from the
    /// strike nearest `ppem` device pixels per em. None when the glyph has no usable colour data;
    /// the caller then draws the outline.
    pub fn color_glyph(&self, gid: u32, ppem: f32) -> Option<ColorGlyph> {
        let f = self.skrifa()?;
        let glyph = GlyphId::new(gid);
        if let Some(cg) = ColorGlyphCollection::new(&f).get(glyph) {
            let palette: Vec<[u8; 4]> =
                ColorPalettes::new(&f).get(0).map(|p| p.colors().iter().map(|c| [c.red, c.green, c.blue, c.alpha]).collect()).unwrap_or_default();
            let mut rec = Recorder::new(&palette, |g: GlyphId, room: usize| {
                let mut p = self.glyph_outline_capped(g.to_u32(), room)?;
                p.apply_affine(Affine::FLIP_Y);
                Some(p)
            });
            if cg.paint(self.location(), &mut rec).is_ok()
                && let Some(ops) = rec.finish()
            {
                return Some(ColorGlyph::Layers(ops));
            }
        }
        let upem = self.units_per_em();
        let ppem = if ppem.is_finite() && ppem > 0.0 { ppem.min(4096.0) } else { 64.0 };
        let bg = skrifa::bitmap::BitmapStrikes::new(&f).glyph_for_size(skrifa::instance::Size::new(ppem), glyph)?;
        bitmap_placement(&bg, upem).map(ColorGlyph::Bitmap)
    }
}

/// Where a bitmap glyph goes, in font units, y down from the pen position. None for an empty,
/// oversized or unsupported (monochrome mask) bitmap.
pub fn bitmap_placement(bg: &skrifa::bitmap::BitmapGlyph<'_>, upem: f64) -> Option<ColorBitmap> {
    let (w, h) = (bg.width, bg.height);
    if w == 0 || h == 0 || w > MAX_BITMAP_SIDE || h > MAX_BITMAP_SIDE {
        return None;
    }
    let data = match bg.data {
        skrifa::bitmap::BitmapData::Png(b) => BitmapBytes::Png(b.to_vec()),
        skrifa::bitmap::BitmapData::Bgra(b) => {
            let n = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
            BitmapBytes::Bgra(b.get(..n)?.to_vec())
        }
        skrifa::bitmap::BitmapData::Mask(_) => return None,
    };
    let (px, py) = (bg.ppem_x as f64, bg.ppem_y as f64);
    if !(px.is_finite() && py.is_finite() && px > 0.0 && py > 0.0 && upem.is_finite() && upem > 0.0) {
        return None;
    }
    // Font units per bitmap pixel.
    let (sx, sy) = (upem / px, upem / py);
    let (bw, bh) = (w as f64 * sx, h as f64 * sy);
    let x0 = bg.bearing_x as f64 + bg.inner_bearing_x as f64 * sx;
    // Top edge, y up.
    let top = match bg.placement_origin {
        skrifa::bitmap::Origin::TopLeft => bg.bearing_y as f64 + bg.inner_bearing_y as f64 * sy,
        skrifa::bitmap::Origin::BottomLeft => bg.bearing_y as f64 + bg.inner_bearing_y as f64 * sy + bh,
    };
    let rect = Rect::new(x0, -top, x0 + bw, -top + bh);
    let finite = [rect.x0, rect.y0, rect.x1, rect.y1].iter().all(|v| v.is_finite());
    finite.then_some(ColorBitmap { data, width: w, height: h, rect, ppem: bg.ppem_y })
}

/// Approximate heap bytes held by `ops`, counting each op's copied stops and path elements.
pub fn ops_bytes(ops: &[ColorOp]) -> usize {
    let payload = |op: &ColorOp| match op {
        ColorOp::PushClip(p) => p.elements().len().saturating_mul(std::mem::size_of::<kurbo::PathEl>()),
        ColorOp::Fill(ColorPaint::Linear { stops, .. } | ColorPaint::Radial { stops, .. } | ColorPaint::Sweep { stops, .. }) => {
            stops.len().saturating_mul(std::mem::size_of::<Stop>())
        }
        _ => 0,
    };
    ops.iter().fold(ops.len().saturating_mul(std::mem::size_of::<ColorOp>()), |n, op| n.saturating_add(payload(op)))
}

/// A [`ColorPainter`] that records the paint calls as [`ColorOp`]s, resolving palette colours and
/// clip glyphs. Stops recording (and [`Recorder::finish`] gives None) past [`MAX_OPS`] or
/// [`MAX_PAYLOAD`].
pub struct Recorder<'a, F: Fn(GlyphId, usize) -> Option<BezPath>> {
    palette: &'a [[u8; 4]],
    outline: F,
    ops: Vec<ColorOp>,
    payload: usize,
    overflow: bool,
}

impl<'a, F: Fn(GlyphId, usize) -> Option<BezPath>> Recorder<'a, F> {
    /// A recorder with palette `palette` (straight RGBA) and clip outlines (font units, y up) from
    /// `outline`, which is given the path elements left in the budget and gives None past them.
    pub fn new(palette: &'a [[u8; 4]], outline: F) -> Self {
        Recorder { palette, outline, ops: vec![], payload: 0, overflow: false }
    }

    /// The recorded ops, None when the glyph was too big or its pushes and pops don't balance.
    pub fn finish(self) -> Option<Vec<ColorOp>> {
        if self.overflow {
            return None;
        }
        let mut depth = 0i64;
        for op in &self.ops {
            match op {
                ColorOp::PushTransform(_) | ColorOp::PushClip(_) | ColorOp::PushLayer(_) => depth += 1,
                ColorOp::PopTransform | ColorOp::PopClip | ColorOp::PopLayer => depth -= 1,
                ColorOp::Fill(_) => {}
            }
            if depth < 0 {
                return None;
            }
        }
        (depth == 0).then_some(self.ops)
    }

    fn push(&mut self, op: ColorOp) {
        if self.ops.len() >= MAX_OPS {
            self.overflow = true;
            return;
        }
        self.ops.push(op);
    }

    /// Charge `n` stops or path elements to the glyph's payload; false (and overflow) past the budget.
    fn charge(&mut self, n: usize) -> bool {
        self.payload = self.payload.saturating_add(n);
        if self.payload > MAX_PAYLOAD {
            self.overflow = true;
        }
        !self.overflow
    }

    fn color(&self, palette_index: u16, alpha: f32) -> GlyphColor {
        let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
        if palette_index == 0xFFFF {
            return GlyphColor::Foreground(alpha);
        }
        match self.palette.get(palette_index as usize) {
            Some(&[r, g, b, a]) => GlyphColor::Rgba([r, g, b, (a as f32 * alpha).round().clamp(0.0, 255.0) as u8]),
            None => GlyphColor::Rgba([0, 0, 0, 0]),
        }
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<Stop> {
        let mut v: Vec<Stop> =
            stops.iter().filter(|s| s.offset.is_finite()).map(|s| Stop { offset: s.offset, color: self.color(s.palette_index, s.alpha) }).collect();
        v.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        v
    }
}

fn pt(p: skrifa::raw::types::Point<f32>) -> Point {
    Point::new(p.x as f64, p.y as f64)
}

impl<F: Fn(GlyphId, usize) -> Option<BezPath>> ColorPainter for Recorder<'_, F> {
    fn push_transform(&mut self, t: Transform) {
        self.push(ColorOp::PushTransform(Affine::new([t.xx, t.yx, t.xy, t.yy, t.dx, t.dy].map(|v| v as f64))));
    }
    fn pop_transform(&mut self) {
        self.push(ColorOp::PopTransform);
    }
    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        if self.overflow {
            return;
        }
        // Built within what is left of the budget: a huge outline is refused while it is drawn.
        let Some(p) = (self.outline)(glyph_id, MAX_PAYLOAD.saturating_sub(self.payload)) else {
            self.overflow = true;
            return;
        };
        if self.charge(p.elements().len()) {
            self.push(ColorOp::PushClip(p));
        }
    }
    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        let r = Rect::new(b.x_min as f64, b.y_min as f64, b.x_max as f64, b.y_max as f64);
        self.push(ColorOp::PushClip(kurbo::Shape::to_path(&r, 0.1)));
    }
    fn pop_clip(&mut self) {
        self.push(ColorOp::PopClip);
    }
    fn fill(&mut self, brush: Brush<'_>) {
        let stops = match &brush {
            Brush::Solid { .. } => 0,
            Brush::LinearGradient { color_stops, .. } | Brush::RadialGradient { color_stops, .. } | Brush::SweepGradient { color_stops, .. } => {
                color_stops.len()
            }
        };
        if !self.charge(stops) {
            return;
        }
        let paint = match brush {
            Brush::Solid { palette_index, alpha } => ColorPaint::Solid(self.color(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                ColorPaint::Linear { p0: pt(p0), p1: pt(p1), stops: self.stops(color_stops), extend }
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                ColorPaint::Radial { c0: pt(c0), r0, c1: pt(c1), r1, stops: self.stops(color_stops), extend }
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                ColorPaint::Sweep { c0: pt(c0), start_angle, end_angle, stops: self.stops(color_stops), extend }
            }
        };
        self.push(ColorOp::Fill(paint));
    }
    fn push_layer(&mut self, mode: CompositeMode) {
        self.push(ColorOp::PushLayer(mode));
    }
    fn pop_layer(&mut self) {
        self.push(ColorOp::PopLayer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(_: GlyphId, room: usize) -> Option<BezPath> {
        Some(kurbo::Shape::to_path(&Rect::new(0.0, 0.0, 10.0, 10.0), 0.1)).filter(|p| p.elements().len() <= room)
    }

    #[test]
    fn records_a_clipped_solid_fill_with_palette_colours() {
        let palette = [[255, 0, 0, 255], [0, 0, 255, 128]];
        let mut r = Recorder::new(&palette, square);
        r.fill_glyph(GlyphId::new(3), None, Brush::Solid { palette_index: 1, alpha: 0.5 });
        r.fill_glyph(GlyphId::new(3), None, Brush::Solid { palette_index: 0xFFFF, alpha: 1.0 });
        let ops = r.finish().unwrap();
        assert_eq!(ops.len(), 6);
        assert!(matches!(&ops[0], ColorOp::PushClip(p) if !p.elements().is_empty()));
        assert_eq!(ops[1], ColorOp::Fill(ColorPaint::Solid(GlyphColor::Rgba([0, 0, 255, 64]))));
        assert_eq!(ops[2], ColorOp::PopClip);
        assert_eq!(ops[4], ColorOp::Fill(ColorPaint::Solid(GlyphColor::Foreground(1.0))));
    }

    #[test]
    fn hostile_palette_indices_and_stops_never_panic() {
        let palette = [[1, 2, 3, 255]];
        let mut r = Recorder::new(&palette, square);
        let stops = [
            ColorStop { offset: 1.0, palette_index: 9999, alpha: f32::NAN },
            ColorStop { offset: f32::NAN, palette_index: 0, alpha: 1.0 },
            ColorStop { offset: 0.0, palette_index: 0, alpha: 7.0 },
        ];
        r.fill(Brush::LinearGradient { p0: Default::default(), p1: Default::default(), color_stops: &stops, extend: Extend::Pad });
        let ops = r.finish().unwrap();
        let ColorOp::Fill(ColorPaint::Linear { stops, .. }) = &ops[0] else { panic!("{ops:?}") };
        assert_eq!(stops.len(), 2, "the NaN offset is dropped");
        assert_eq!(stops[0], Stop { offset: 0.0, color: GlyphColor::Rgba([1, 2, 3, 255]) });
        assert_eq!(stops[1].color, GlyphColor::Rgba([0, 0, 0, 0]), "an out-of-range index is transparent");
    }

    #[test]
    fn unbalanced_or_oversized_paint_is_rejected() {
        let mut r = Recorder::new(&[], square);
        r.pop_clip();
        r.push_clip_glyph(GlyphId::new(1));
        assert!(r.finish().is_none());
        let mut r = Recorder::new(&[], square);
        r.push_clip_glyph(GlyphId::new(1));
        assert!(r.finish().is_none());
        let mut r = Recorder::new(&[], square);
        for _ in 0..MAX_OPS + 5 {
            r.fill(Brush::Solid { palette_index: 0, alpha: 1.0 });
        }
        assert!(r.finish().is_none());
    }

    #[test]
    fn gradient_stops_and_clip_paths_share_a_payload_budget() {
        // A few hundred fills reusing one long stop list would copy it per fill, far under MAX_OPS.
        let stops = vec![ColorStop { offset: 0.5, palette_index: 0, alpha: 1.0 }; 4096];
        let fills = MAX_PAYLOAD / stops.len() + 1;
        assert!(fills < MAX_OPS);
        let outlines = std::cell::Cell::new(0);
        let count = |g, room| {
            outlines.set(outlines.get() + 1);
            square(g, room)
        };
        let mut r = Recorder::new(&[], count);
        for _ in 0..fills {
            r.fill(Brush::LinearGradient { p0: Default::default(), p1: Default::default(), color_stops: &stops, extend: Extend::Pad });
        }
        assert!(r.ops.len() < fills, "stops past the budget are not copied");
        r.push_clip_glyph(GlyphId::new(1));
        r.pop_clip();
        assert_eq!(outlines.get(), 0, "no clip outline is built once the budget is spent");
        assert!(r.finish().is_none());
        // Within budget nothing changes, and the byte estimate counts the copied stops.
        let mut r = Recorder::new(&[], square);
        r.fill(Brush::LinearGradient { p0: Default::default(), p1: Default::default(), color_stops: &stops, extend: Extend::Pad });
        let ops = r.finish().unwrap();
        assert!(ops_bytes(&ops) >= stops.len() * std::mem::size_of::<Stop>());
    }

    #[test]
    fn a_clip_outline_is_drawn_within_the_payload_left() {
        // #3716 review: the outline is told the room left, so an oversized one is never built whole.
        let stops = vec![ColorStop { offset: 0.5, palette_index: 0, alpha: 1.0 }; MAX_PAYLOAD - 3];
        let rooms = std::cell::RefCell::new(vec![]);
        let seen = |g, room| {
            rooms.borrow_mut().push(room);
            square(g, room)
        };
        let mut r = Recorder::new(&[], seen);
        r.fill(Brush::LinearGradient { p0: Default::default(), p1: Default::default(), color_stops: &stops, extend: Extend::Pad });
        r.push_clip_glyph(GlyphId::new(1));
        r.pop_clip();
        assert_eq!(*rooms.borrow(), [3]);
        assert!(r.finish().is_none(), "a square needs more than the 3 elements left");
    }

    fn bitmap(origin: skrifa::bitmap::Origin, data: skrifa::bitmap::BitmapData<'_>) -> skrifa::bitmap::BitmapGlyph<'_> {
        skrifa::bitmap::BitmapGlyph {
            data,
            bearing_x: 0.0,
            bearing_y: 0.0,
            inner_bearing_x: 2.0,
            inner_bearing_y: 100.0,
            ppem_x: 100.0,
            ppem_y: 100.0,
            advance: None,
            width: 120,
            height: 110,
            placement_origin: origin,
        }
    }

    /// A CBDT strike places the bitmap's top-left corner `inner_bearing` pixels from the pen; sbix
    /// places its bottom-left corner. Both scale pixels to font units by upem / ppem.
    #[test]
    fn bitmap_glyphs_are_placed_in_font_units() {
        let png = [0u8; 4];
        let b = bitmap_placement(&bitmap(skrifa::bitmap::Origin::TopLeft, skrifa::bitmap::BitmapData::Png(&png)), 1000.0).unwrap();
        assert_eq!(b.rect, Rect::new(20.0, -1000.0, 1220.0, 100.0));
        let b = bitmap_placement(&bitmap(skrifa::bitmap::Origin::BottomLeft, skrifa::bitmap::BitmapData::Png(&png)), 1000.0).unwrap();
        assert_eq!(b.rect, Rect::new(20.0, -2100.0, 1220.0, -1000.0));
        // Short BGRA data, a zero ppem and a monochrome mask are refused, not read past.
        assert!(bitmap_placement(&bitmap(skrifa::bitmap::Origin::TopLeft, skrifa::bitmap::BitmapData::Bgra(&png)), 1000.0).is_none());
        let mut z = bitmap(skrifa::bitmap::Origin::TopLeft, skrifa::bitmap::BitmapData::Png(&png));
        z.ppem_y = 0.0;
        assert!(bitmap_placement(&z, 1000.0).is_none());
        let mut big = bitmap(skrifa::bitmap::Origin::TopLeft, skrifa::bitmap::BitmapData::Png(&png));
        big.width = MAX_BITMAP_SIDE + 1;
        assert!(bitmap_placement(&big, 1000.0).is_none());
    }

    #[test]
    fn faces_without_colour_tables_have_no_colour_glyphs() {
        let face = crate::word::resolve("Source Sans 3", false, false).face;
        assert!(!face.has_color_glyphs());
        assert_eq!(face.color_glyph(face.glyph_for('A'), 32.0), None);
    }
}
