//! Drawing state shared by the WMF and EMF readers: window/viewport mapping and the EMF world
//! transform, pen and brush selection through an object table, the DC save stack and path brackets.
//! Output is device-space items; `lib.rs` maps them into the picture frame.

use crate::dib::Rgba;
use crate::geom::{Op, P};
use crate::{MAX_BYTES, MAX_ITEMS, MAX_OBJECTS, MAX_PATH_OPS, MAX_STACK};

pub(crate) const MM_TEXT: u32 = 1;
const MM_ISOTROPIC: u32 = 7;
const STOCK: u32 = 0x8000_0000;
pub(crate) const PS_NULL: u32 = 5;
/// Pen style bits; the rest of the style word holds end-cap, join and type flags.
pub(crate) const PS_STYLE_MASK: u32 = 0x0F;
const BS_SOLID: u32 = 0;
pub(crate) const BS_NULL: u32 = 1;
const WINDING: u32 = 2;
const PATCOPY: u32 = 0x00F0_0021;
const BLACKNESS: u32 = 0x0000_0042;
const WHITENESS: u32 = 0x00FF_0062;
/// Device pixels per millimetre assumed for WMF (96 dpi), and for EMF files without a device size.
const DEFAULT_PX_PER_MM: f64 = 96.0 / 25.4;

/// Affine transform `[m11, m12, m21, m22, dx, dy]` in EMF XFORM order: `x' = x·m11 + y·m21 + dx`.
pub(crate) type Xform = [f64; 6];
const IDENTITY: Xform = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b` in EMF row-vector convention: the matrix product `a × b`.
pub(crate) fn mul(a: Xform, b: Xform) -> Xform {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Pen {
    pub style: u32,
    /// Logical width; 0 means a cosmetic one-pixel hairline.
    pub width: f64,
    pub color: [u8; 4],
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Brush {
    pub style: u32,
    pub color: [u8; 4],
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Obj {
    Pen(Pen),
    Brush(Brush),
    /// Fonts, palettes, pattern brushes and regions: they take a table slot but draw nothing.
    Other,
}

/// Converts a COLORREF (0x00BBGGRR) to opaque RGBA.
pub(crate) fn colorref(c: u32) -> [u8; 4] {
    [(c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8, 255]
}

/// Millimetres per logical unit of the metric map modes (LOMETRIC, HIMETRIC, LOENGLISH, HIENGLISH, TWIPS).
fn metric_mm(mode: u32) -> Option<f64> {
    match mode {
        2 => Some(0.1),
        3 => Some(0.01),
        4 => Some(25.4 / 100.0),
        5 => Some(25.4 / 1000.0),
        6 => Some(25.4 / 1440.0),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Mapping {
    wo: P,
    we: P,
    vo: P,
    ve: P,
    mode: u32,
    world: Xform,
    /// Device pixels per millimetre on each axis (EMF: from szlDevice and szlMillimeters).
    px_mm: P,
}

impl Mapping {
    fn identity() -> Self {
        Mapping {
            wo: (0.0, 0.0),
            we: (1.0, 1.0),
            vo: (0.0, 0.0),
            ve: (1.0, 1.0),
            mode: MM_TEXT,
            world: IDENTITY,
            px_mm: (DEFAULT_PX_PER_MM, DEFAULT_PX_PER_MM),
        }
    }

    /// Logical point to device point: world transform, then window to viewport.
    pub fn device(&self, p: P) -> P {
        let w = self.world;
        let (x, y) = (p.0 * w[0] + p.1 * w[2] + w[4], p.0 * w[1] + p.1 * w[3] + w[5]);
        let (sx, sy) = self.ratios();
        ((x - self.wo.0) * sx + self.vo.0, (y - self.wo.1) * sy + self.vo.1)
    }

    /// Window-to-viewport scale per axis. MM_TEXT fixes the scale at 1; the metric modes use a fixed
    /// scale with y up; MM_ISOTROPIC uses one scale on both axes (the smaller magnitude, each axis's
    /// sign kept); anisotropic and unknown modes use the extents as given.
    fn ratios(&self) -> (f64, f64) {
        if self.mode == MM_TEXT {
            return (1.0, 1.0);
        }
        if let Some(mm) = metric_mm(self.mode) {
            return (mm * self.px_mm.0, -mm * self.px_mm.1);
        }
        let (rx, ry) = (ratio(self.ve.0, self.we.0), ratio(self.ve.1, self.we.1));
        if self.mode == MM_ISOTROPIC {
            let s = rx.abs().min(ry.abs());
            return (if rx < 0.0 { -s } else { s }, if ry < 0.0 { -s } else { s });
        }
        (rx, ry)
    }

    /// Approximate device length per logical length, for pen widths.
    fn scale(&self) -> f64 {
        let w = self.world;
        let det = (w[0] * w[3] - w[1] * w[2]).abs().sqrt();
        let (sx, sy) = self.ratios();
        det * (sx.abs() + sy.abs()) / 2.0
    }

    /// The window in device units as `[x, y, w, h]` with positive extents, used as the picture frame
    /// of a plain WMF.
    pub fn window_rect(&self) -> [f64; 4] {
        let a = self.device(self.wo);
        let b = self.device((self.wo.0 + self.we.0, self.wo.1 + self.we.1));
        [a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs()]
    }
}

fn ratio(num: f64, den: f64) -> f64 {
    if den == 0.0 { 1.0 } else { num / den }
}

/// Output of drawing, in device units. Path points are already mapped; `lib.rs` normalises them.
pub(crate) enum Raw {
    Path { ops: Vec<Op>, fill: Option<[u8; 4]>, stroke: Option<([u8; 4], f64)>, even_odd: bool },
    Bitmap { rect: [f64; 4], img: Rgba },
}

#[derive(Clone)]
struct Saved {
    map: Mapping,
    pen: Pen,
    brush: Brush,
    even_odd: bool,
}

pub(crate) struct Canvas {
    pub items: Vec<Raw>,
    pub map: Mapping,
    /// True once SETWINDOWEXT was seen, so a plain WMF has a frame to use.
    pub window_set: bool,
    saved: Vec<Saved>,
    /// Bytes held by the items so far (path ops and pixels), capped at [`MAX_BYTES`].
    used: usize,
    objects: Vec<Option<Obj>>,
    pen: Pen,
    brush: Brush,
    even_odd: bool,
    pos: P,
    in_path: bool,
    path: Vec<Op>,
    full: bool,
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            items: Vec::new(),
            map: Mapping::identity(),
            window_set: false,
            saved: Vec::new(),
            used: 0,
            objects: Vec::new(),
            pen: Pen { style: 0, width: 0.0, color: [0, 0, 0, 255] },
            brush: Brush { style: BS_SOLID, color: [255, 255, 255, 255] },
            even_odd: true,
            pos: (0.0, 0.0),
            in_path: false,
            path: Vec::new(),
            full: false,
        }
    }

    /// True once the item cap is hit; the readers stop there and keep what they have.
    pub fn full(&self) -> bool {
        self.full
    }

    fn push(&mut self, raw: Raw) {
        let cost = match &raw {
            Raw::Path { ops, .. } => ops.len() * std::mem::size_of::<Op>(),
            Raw::Bitmap { img, .. } => img.px.len(),
        };
        if self.items.len() >= MAX_ITEMS || self.used.saturating_add(cost) > MAX_BYTES {
            self.full = true;
        } else {
            self.used += cost;
            self.items.push(raw);
        }
    }

    pub fn set_window_org(&mut self, p: P) {
        self.map.wo = p;
    }

    pub fn set_window_ext(&mut self, e: P) {
        self.map.we = e;
        self.window_set = true;
    }

    pub fn set_viewport_org(&mut self, p: P) {
        self.map.vo = p;
    }

    pub fn set_viewport_ext(&mut self, e: P) {
        self.map.ve = e;
    }

    pub fn set_map_mode(&mut self, mode: u32) {
        self.map.mode = mode;
    }

    /// Device pixels per millimetre (x, y) for the metric map modes. Ignored unless both are positive.
    pub fn set_device_mm(&mut self, px_mm: P) {
        if px_mm.0.is_finite() && px_mm.1.is_finite() && px_mm.0 > 0.0 && px_mm.1 > 0.0 {
            self.map.px_mm = px_mm;
        }
    }

    pub fn set_polyfill(&mut self, mode: u32) {
        self.even_odd = mode != WINDING;
    }

    /// EMF SETWORLDTRANSFORM (`mode` 4) and MODIFYWORLDTRANSFORM (1 identity, 2 left, 3 right, 4 set).
    /// An unknown mode leaves the world transform as it was.
    pub fn modify_world(&mut self, x: Xform, mode: u32) {
        match mode {
            1 => self.map.world = IDENTITY,
            2 => self.map.world = mul(x, self.map.world),
            3 => self.map.world = mul(self.map.world, x),
            4 => self.map.world = x,
            _ => {}
        }
    }

    /// Pushes the drawing state. Past [`MAX_STACK`] levels the parse stops, since later restores
    /// could no longer match the file.
    pub fn save_dc(&mut self) {
        if self.saved.len() < MAX_STACK {
            self.saved.push(Saved { map: self.map, pen: self.pen, brush: self.brush, even_odd: self.even_odd });
        } else {
            self.full = true;
        }
    }

    /// Negative `n` pops `-n` levels; positive `n` restores absolute level `n`.
    pub fn restore_dc(&mut self, n: i32) {
        let idx = if n < 0 { self.saved.len().checked_sub(n.unsigned_abs() as usize) } else { (n as usize).checked_sub(1) };
        let Some(idx) = idx else { return };
        let Some(s) = self.saved.get(idx).cloned() else { return };
        self.saved.truncate(idx);
        self.map = s.map;
        self.pen = s.pen;
        self.brush = s.brush;
        self.even_odd = s.even_odd;
    }

    /// Stores an object in `slot` (EMF index), or in the first free slot (WMF handle table).
    pub fn add_object(&mut self, slot: Option<usize>, obj: Obj) {
        match slot {
            Some(i) if i < MAX_OBJECTS => {
                if self.objects.len() <= i {
                    self.objects.resize(i + 1, None);
                }
                if let Some(s) = self.objects.get_mut(i) {
                    *s = Some(obj);
                }
            }
            Some(_) => {}
            None => match self.objects.iter().position(Option::is_none) {
                Some(i) => {
                    if let Some(s) = self.objects.get_mut(i) {
                        *s = Some(obj);
                    }
                }
                None if self.objects.len() < MAX_OBJECTS => self.objects.push(Some(obj)),
                None => {}
            },
        }
    }

    pub fn delete_object(&mut self, i: u32) {
        if let Some(s) = self.objects.get_mut(i as usize) {
            *s = None;
        }
    }

    /// Selects an object by index; indices with the high bit set are stock objects.
    pub fn select(&mut self, i: u32) {
        if i & STOCK != 0 {
            return self.select_stock(i & !STOCK);
        }
        match self.objects.get(i as usize).copied().flatten() {
            Some(Obj::Pen(p)) => self.pen = p,
            Some(Obj::Brush(b)) => self.brush = b,
            Some(Obj::Other) | None => {}
        }
    }

    fn select_stock(&mut self, i: u32) {
        let white = [255, 255, 255, 255];
        let black = [0, 0, 0, 255];
        let gray = |v: u8| Brush { style: BS_SOLID, color: [v, v, v, 255] };
        match i {
            0 | 18 => self.brush = Brush { style: BS_SOLID, color: white },
            1 => self.brush = gray(192),
            2 => self.brush = gray(128),
            3 => self.brush = gray(64),
            4 => self.brush = Brush { style: BS_SOLID, color: black },
            5 => self.brush = Brush { style: BS_NULL, color: black },
            6 => self.pen = Pen { style: 0, width: 0.0, color: white },
            7 | 19 => self.pen = Pen { style: 0, width: 0.0, color: black },
            8 => self.pen = Pen { style: PS_NULL, width: 0.0, color: black },
            _ => {}
        }
    }

    fn map_op(&self, op: Op) -> Op {
        match op {
            Op::M(p) => Op::M(self.map.device(p)),
            Op::L(p) => Op::L(self.map.device(p)),
            Op::C(a, b, c) => Op::C(self.map.device(a), self.map.device(b), self.map.device(c)),
            Op::Z => Op::Z,
        }
    }

    fn fill_color(&self) -> Option<[u8; 4]> {
        (self.brush.style == BS_SOLID).then_some(self.brush.color)
    }

    fn stroke_pen(&self) -> Option<([u8; 4], f64)> {
        if (self.pen.style & PS_STYLE_MASK) == PS_NULL {
            return None;
        }
        // A cosmetic pen (width 0) stays 0: the renderers draw that as a hairline.
        let w = if self.pen.width == 0.0 { 0.0 } else { self.pen.width.abs() * self.map.scale() };
        Some((self.pen.color, w))
    }

    fn push_path_ops(&mut self, ops: impl IntoIterator<Item = Op>) {
        for op in ops {
            if self.path.len() >= MAX_PATH_OPS {
                return;
            }
            self.path.push(op);
        }
    }

    /// Draws logical-space ops with the current pen (`stroke`) and brush (`fill`), or appends them to
    /// the open path bracket, where fill and stroke wait for FILLPATH/STROKEPATH.
    pub fn shape(&mut self, ops: &[Op], fill: bool, stroke: bool) {
        let dev: Vec<Op> = ops.iter().map(|o| self.map_op(*o)).collect();
        if self.in_path {
            return self.push_path_ops(dev);
        }
        let fill = if fill { self.fill_color() } else { None };
        let stroke = if stroke { self.stroke_pen() } else { None };
        if (fill.is_none() && stroke.is_none()) || dev.len() < 2 {
            return;
        }
        let even_odd = self.even_odd;
        self.push(Raw::Path { ops: dev, fill, stroke, even_odd });
    }

    pub fn move_to(&mut self, p: P) {
        if self.in_path {
            self.push_path_ops([Op::M(self.map.device(p))]);
        }
        self.pos = p;
    }

    /// LINETO from the current point, then moves the current point to `p`.
    pub fn line_to(&mut self, p: P) {
        self.line_chain(&[p], false, true);
    }

    /// Lines from the current point through `pts` (POLYLINETO), or a polyline from the current point.
    pub fn line_chain(&mut self, pts: &[P], fill: bool, stroke: bool) {
        // Inside an open bracket the path already ends at the current point, so no MoveTo is needed.
        let mut ops = if self.in_path && !self.path.is_empty() { Vec::new() } else { vec![Op::M(self.pos)] };
        ops.extend(pts.iter().map(|p| Op::L(*p)));
        if let Some(last) = pts.last() {
            self.pos = *last;
        }
        self.shape(&ops, fill, stroke);
    }

    /// POLYBEZIER: cubic segments that start a new figure at `from`. The current point is unchanged.
    pub fn bezier(&mut self, from: P, pts: &[P]) {
        let mut ops = vec![Op::M(from)];
        ops.extend(cubics(pts));
        self.shape(&ops, false, true);
    }

    /// POLYBEZIERTO: cubic segments from the current point through groups of three control or end
    /// points; the current point moves to the last point. Inside an open bracket the path already
    /// ends at the current point, so no MoveTo is needed.
    pub fn bezier_to(&mut self, pts: &[P]) {
        let mut ops = if self.in_path && !self.path.is_empty() { Vec::new() } else { vec![Op::M(self.pos)] };
        ops.extend(cubics(pts));
        if let Some(last) = pts.last() {
            self.pos = *last;
        }
        self.shape(&ops, false, true);
    }

    pub fn poly(&mut self, pts: &[P], close: bool, fill: bool, stroke: bool) {
        let Some(first) = pts.first() else { return };
        let mut ops = vec![Op::M(*first)];
        ops.extend(pts.iter().skip(1).map(|p| Op::L(*p)));
        if close {
            ops.push(Op::Z);
        }
        self.shape(&ops, fill, stroke);
    }

    pub fn begin_path(&mut self) {
        self.in_path = true;
        self.path.clear();
    }

    pub fn end_path(&mut self) {
        self.in_path = false;
    }

    pub fn close_figure(&mut self) {
        if self.in_path {
            self.push_path_ops([Op::Z]);
        }
    }

    /// FILLPATH, STROKEPATH and STROKEANDFILLPATH: paints the bracketed path and empties it.
    pub fn paint_path(&mut self, fill: bool, stroke: bool) {
        let ops = std::mem::take(&mut self.path);
        let fill = if fill { self.fill_color() } else { None };
        let stroke = if stroke { self.stroke_pen() } else { None };
        if ops.len() < 2 || (fill.is_none() && stroke.is_none()) {
            return;
        }
        let even_odd = self.even_odd;
        self.push(Raw::Path { ops, fill, stroke, even_odd });
    }

    /// Fills a logical rectangle `[l, t, r, b]` with the current brush (nothing when it is not solid).
    pub fn fill_brush_rect(&mut self, rect: [f64; 4]) {
        if let Some(color) = self.fill_color() {
            self.fill_rect_with(rect, color);
        }
    }

    /// Fills a logical rectangle `[l, t, r, b]` with a solid colour (ROP fills such as BLACKNESS).
    pub fn fill_rect_with(&mut self, [l, t, r, b]: [f64; 4], color: [u8; 4]) {
        let dev: Vec<Op> = crate::geom::rect(l, t, r, b).iter().map(|o| self.map_op(*o)).collect();
        self.push(Raw::Path { ops: dev, fill: Some(color), stroke: None, even_odd: true });
    }

    /// Whether a raster operation draws without a source bitmap (PATCOPY, BLACKNESS, WHITENESS).
    pub fn sourceless(rop: u32) -> bool {
        matches!(rop, PATCOPY | BLACKNESS | WHITENESS)
    }

    /// Fills a logical rectangle `[x, y, w, h]` for a source-less raster operation. PATCOPY uses the
    /// brush (when solid); the others fill black or white. Other operations are not handled here.
    pub fn fill_rop(&mut self, [x, y, w, h]: [f64; 4], rop: u32) {
        let rect = [x, y, x + w, y + h];
        match rop {
            PATCOPY => self.fill_brush_rect(rect),
            BLACKNESS => self.fill_rect_with(rect, [0, 0, 0, 255]),
            WHITENESS => self.fill_rect_with(rect, [255, 255, 255, 255]),
            _ => {}
        }
    }

    /// A bitmap stretched into a logical rectangle `[x, y, w, h]`. A negative extent mirrors the
    /// bitmap, as the mapping does.
    pub fn bitmap(&mut self, rect: [f64; 4], mut img: Rgba) {
        let [x, y, w, h] = rect;
        let a = self.map.device((x, y));
        let b = self.map.device((x + w, y + h));
        crate::dib::flip(&mut img, a.0 > b.0, a.1 > b.1);
        let dev = [a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs()];
        self.push(Raw::Bitmap { rect: dev, img });
    }
}

/// Cubic segments from groups of three points (control, control, end). A trailing partial group is
/// dropped.
fn cubics(pts: &[P]) -> Vec<Op> {
    let mut ops = Vec::with_capacity(pts.len() / 3);
    let mut it = pts.iter().copied();
    while let (Some(a), Some(b), Some(e)) = (it.next(), it.next(), it.next()) {
        ops.push(Op::C(a, b, e));
    }
    ops
}
