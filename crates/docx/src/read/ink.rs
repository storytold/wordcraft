//! Bounded decoding of the visually simple subset of Word's native InkML strokes.
//!
//! The original InkML remains opaque in the document model for lossless saves; this module only
//! builds disposable drawing items. Unsupported visual semantics, invalid input and exhausted
//! work return `None`, allowing the DOCX reader to keep its picture fallback instead.

use std::collections::HashMap;

use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg};
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;

use crate::xml::El;

/// Maximum InkML part size that the vector preview decoder will parse (2 MiB).
const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Maximum XML elements indexed from one InkML part.
const MAX_NODES: usize = 8192;
/// Maximum nesting or chained reference depth accepted in InkML.
const MAX_DEPTH: usize = 16;
/// Maximum regular trace channels supported per format.
const MAX_CHANNELS: usize = 16;
/// Maximum coordinate samples decoded from one InkML part.
const MAX_POINTS: usize = 100_000;
/// Maximum absolute channel value accepted before unit conversion.
const MAX_VALUE: f64 = 1_000_000_000.0;

/// Decode supported strokes from InkML, charging parsing and geometry work to the shared budget.
///
/// Returns `None` for malformed packages, unsupported visual semantics, excessive sizes, or
/// exhausted work; callers can then display the original picture fallback. Success contains
/// vector items in the drawing's intrinsic point coordinates, but never replaces the saved XML.
pub(super) fn read(bytes: &[u8], work: &mut usize) -> Option<Graphic> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    spend(work, bytes.len().div_ceil(16))?;
    let root = crate::xml::parse_strict(bytes).ok()?;
    if root.name != "ink:ink" {
        return None;
    }
    let mut ids = HashMap::new();
    index(&root, &mut ids, &mut 0, 0, work)?;
    let mut reader = InkReader { ids, work, strokes: Vec::new(), points: 0, default_context: Context::default() };
    reader.walk(&root, Context::default(), 0)?;
    drawing(reader.strokes)
}

/// Scale decoded vector geometry from its InkML bounds into the frame dimensions in points.
///
/// Both coordinates and pen widths scale with the frame; picture items and empty graphics need
/// no adjustment. The caller supplies already validated, nonnegative frame dimensions.
pub(super) fn fit_frame(graphic: &mut Graphic, w: f32, h: f32) {
    if graphic.w <= 0.0 || graphic.h <= 0.0 {
        return;
    }
    let (sx, sy) = (w / graphic.w, h / graphic.h);
    for item in &mut graphic.items {
        match item {
            GraphicItem::Path { segs, stroke_width, .. } => {
                for seg in segs {
                    match seg {
                        PathSeg::Move(x, y) | PathSeg::Line(x, y) => {
                            *x *= sx;
                            *y *= sy;
                        }
                        _ => {}
                    }
                }
                *stroke_width = (*stroke_width * ((sx + sy) / 2.0)).min(crate::units::MAX_LEN_PT);
            }
            GraphicItem::Shape { rect, .. } => {
                for (v, scale) in rect.iter_mut().zip([sx, sy, sx, sy]) {
                    *v *= scale;
                }
            }
            _ => {}
        }
    }
    graphic.w = w;
    graphic.h = h;
}

/// Debit `amount` from the reader's shared allowance, returning `None` if it was exhausted.
fn spend(work: &mut usize, amount: usize) -> Option<()> {
    let enough = *work >= amount;
    *work = work.saturating_sub(amount);
    enough.then_some(())
}

/// Index bounded, uniquely identified XML elements so local InkML references resolve in constant time.
fn index<'a>(el: &'a El, ids: &mut HashMap<&'a str, &'a El>, nodes: &mut usize, depth: usize, work: &mut usize) -> Option<()> {
    spend(work, 1)?;
    *nodes += 1;
    if *nodes > MAX_NODES || depth > MAX_DEPTH {
        return None;
    }
    if let Some(id) = el.attr("xml:id")
        && (id.is_empty() || id.len() > 256 || ids.insert(id, el).is_some())
    {
        return None;
    }
    for child in el.els() {
        index(child, ids, nodes, depth + 1, work)?;
    }
    Some(())
}

/// Channel positions and unit conversion for the subset of an InkML trace format we draw.
#[derive(Clone)]
struct Format {
    /// Number of regular channels that must be decoded for each sample.
    channels: usize,
    /// Zero-based channel index for the horizontal coordinate.
    x: usize,
    /// Zero-based channel index for the vertical coordinate.
    y: usize,
    /// Conversion factors from the X and Y units into document points.
    scale: [f64; 2],
}

impl Default for Format {
    /// Use InkML's default point-like X/Y canvas.
    fn default() -> Self {
        Self { channels: 2, x: 0, y: 1, scale: [1.0; 2] }
    }
}

/// The supported visual properties inherited by an InkML trace.
#[derive(Clone, Default)]
struct Brush {
    /// Brush colour, or black when the InkML document leaves it unspecified.
    color: Option<Rgb>,
    /// Circular pen diameter in points, or the renderer's one-point default.
    width: Option<f64>,
    /// Explicit tip height, if present; only circles whose height matches `width` are drawn.
    height: Option<f64>,
}

/// Resolved trace-format and brush properties inherited by descendant traces.
#[derive(Clone, Default)]
struct Context {
    /// Coordinate channels and their point conversion.
    format: Format,
    /// Visual pen properties for a trace using this context.
    brush: Brush,
}

/// One decoded pen-down trace, before placement in a drawing frame.
struct Stroke {
    /// Samples as finite X/Y values in the source InkML canvas.
    points: Vec<[f64; 2]>,
    /// Opaque pen colour.
    color: Rgb,
    /// Pen width in points.
    width: f32,
}

/// Stateful InkML walker with a part-local reference table and document-wide work allowance.
struct InkReader<'a, 'w> {
    /// Unique XML IDs retained for local context, brush, and format references.
    ids: HashMap<&'a str, &'a El>,
    /// Remaining DOCX graphic-work budget shared with other embedded graphics.
    work: &'w mut usize,
    /// Supported pen-down traces accumulated for the final vector graphic.
    strokes: Vec<Stroke>,
    /// Total coordinate samples consumed from the current InkML part.
    points: usize,
    /// Stream defaults in effect for an explicit `#DefaultContext` reference.
    default_context: Context,
}

impl<'a> InkReader<'a, '_> {
    /// Resolve a fragment-only URI to an indexed element of the expected InkML kind.
    fn resolve(&mut self, reference: &str, name: &str) -> Option<&'a El> {
        spend(self.work, 1)?;
        let id = reference.strip_prefix('#')?;
        let el = *self.ids.get(id)?;
        (el.name == name).then_some(el)
    }

    /// Read a brush's supported properties, recursively resolving its optional inherited brush.
    fn brush(&mut self, el: &El, depth: usize) -> Option<Brush> {
        spend(self.work, 1)?;
        if depth > MAX_DEPTH {
            return None;
        }
        let mut brush = match el.attr("brushRef") {
            None | Some("#DefaultBrush") => Brush::default(),
            Some(reference) => {
                let parent = self.resolve(reference, "ink:brush")?;
                self.brush(parent, depth + 1)?
            }
        };
        for p in el.els() {
            spend(self.work, p.attrs.len().saturating_mul(4).max(1))?;
            if matches!(p.name.as_str(), "ink:annotation" | "ink:annotationXML") {
                continue;
            }
            if p.name != "ink:brushProperty" {
                return None;
            }
            let value = p.attr("value")?;
            match p.attr("name")? {
                "color" => brush.color = Some(Rgb::parse(value)?),
                "width" | "height" => {
                    let size = number(value)? * unit(p.attr("units"))?;
                    if !(0.001..=100_000.0).contains(&size) {
                        return None;
                    }
                    if p.attr("name") == Some("width") {
                        brush.width = Some(size);
                    } else {
                        brush.height = Some(size);
                    }
                }
                "transparency" if number(value)? == 0.0 => {}
                "tip" if value == "ellipse" => {}
                "rasterOp" if value == "copyPen" => {}
                _ => return None,
            }
        }
        Some(brush)
    }

    /// Resolve inherited, referenced, and directly specified properties for one context element.
    fn context(&mut self, el: &El, inherited: Context, depth: usize) -> Option<Context> {
        spend(self.work, el.kids.len().saturating_add(el.attrs.len()).saturating_mul(12).max(1))?;
        if depth > MAX_DEPTH {
            return None;
        }
        // A transform we cannot evaluate must not silently become an identity transform.
        if el.attr("canvasRef").is_some_and(|r| r != "#DefaultCanvas")
            || el.attr("canvasTransformRef").is_some()
            || el.child("ink:canvas").is_some()
            || el.child("ink:canvasTransform").is_some()
        {
            return None;
        }
        let mut ctx = match el.attr("contextRef") {
            None => inherited,
            Some("#DefaultContext") => self.default_context.clone(),
            Some(reference) => {
                let parent = self.resolve(reference, "ink:context")?;
                self.context(parent, inherited, depth + 1)?
            }
        };
        if let Some(reference) = el.attr("inkSourceRef") {
            let source = self.resolve(reference, "ink:inkSource")?;
            ctx.format = source_format(source)?;
        }
        if let Some(source) = el.child("ink:inkSource") {
            ctx.format = source_format(source)?;
        }
        if let Some(reference) = el.attr("traceFormatRef") {
            let format = self.resolve(reference, "ink:traceFormat")?;
            ctx.format = trace_format(format)?;
        }
        if let Some(format) = el.child("ink:traceFormat") {
            ctx.format = trace_format(format)?;
        }
        if let Some(reference) = el.attr("brushRef") {
            ctx.brush = if reference == "#DefaultBrush" {
                Brush::default()
            } else {
                let brush = self.resolve(reference, "ink:brush")?;
                self.brush(brush, 0)?
            };
        }
        if let Some(brush) = el.child("ink:brush") {
            ctx.brush = self.brush(brush, 0)?;
        }
        Some(ctx)
    }

    /// Visit supported stream elements in document order and decode their traces recursively.
    fn walk(&mut self, el: &El, mut ctx: Context, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        for child in el.els() {
            spend(self.work, 1)?;
            if depth == 0 {
                self.default_context = ctx.clone();
            }
            match child.name.as_str() {
                "ink:definitions" | "ink:annotation" | "ink:annotationXML" | "ink:timestamp" => {}
                "ink:traceFormat" => ctx.format = trace_format(child)?,
                "ink:inkSource" => ctx.format = source_format(child)?,
                "ink:brush" => ctx.brush = self.brush(child, 0)?,
                "ink:context" => ctx = self.context(child, ctx, 0)?,
                "ink:traceGroup" => {
                    let group = self.context(child, ctx.clone(), 0)?;
                    self.walk(child, group, depth + 1)?;
                }
                "ink:trace" => {
                    let trace = self.context(child, ctx.clone(), 0)?;
                    self.trace(child, &trace)?;
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// Decode one simple pen-down trace using its resolved channels and circular-pen properties.
    fn trace(&mut self, el: &El, ctx: &Context) -> Option<()> {
        if el.attr("continuation").is_some() || el.attr("priorRef").is_some() || el.els().next().is_some() {
            return None;
        }
        match el.attr("type") {
            Some("penUp") => return Some(()),
            None | Some("penDown") => {}
            _ => return None,
        }
        let width = ctx.brush.width.unwrap_or(1.0);
        if ctx.brush.height.is_some_and(|h| (h - width).abs() > 0.0001) {
            return None;
        }
        let text = el.text();
        let mut tokens = Tokens(text.trim());
        let mut state = vec![ChannelState::default(); ctx.format.channels];
        let mut points = Vec::new();
        while !tokens.0.is_empty() {
            spend(self.work, ctx.format.channels + 1)?;
            if self.points >= MAX_POINTS {
                return None;
            }
            for channel in &mut state {
                channel.next(&mut tokens)?;
            }
            let x = state.get(ctx.format.x)?.value? * ctx.format.scale[0];
            let y = state.get(ctx.format.y)?.value? * ctx.format.scale[1];
            if !bounded(x) || !bounded(y) {
                return None;
            }
            points.push([x, y]);
            self.points += 1;
            tokens.space();
            if !tokens.0.is_empty() {
                tokens.0 = tokens.0.strip_prefix(',')?;
                tokens.space();
            }
        }
        if points.is_empty() {
            return None;
        }
        self.strokes.push(Stroke { points, color: ctx.brush.color.unwrap_or(Rgb::BLACK), width: width as f32 });
        Some(())
    }
}

/// Get the trace format declared inside an InkML input source.
fn source_format(el: &El) -> Option<Format> {
    trace_format(el.child("ink:traceFormat")?)
}

/// Read ordered regular channels, accepting supported X/Y axes and optional nonvisual channels.
fn trace_format(el: &El) -> Option<Format> {
    let mut format = Format { channels: 0, ..Default::default() };
    let (mut x, mut y) = (None, None);
    for channel in el.els() {
        if channel.name != "ink:channel" || format.channels >= MAX_CHANNELS {
            return None;
        }
        let index = format.channels;
        format.channels += 1;
        let axis = match channel.attr("name")? {
            "X" if x.is_none() => {
                x = Some(index);
                Some(0)
            }
            "Y" if y.is_none() => {
                y = Some(index);
                Some(1)
            }
            "F" | "Z" | "T" | "OT" | "OA" | "OE" | "OR" => None,
            _ => return None,
        };
        if let Some(axis) = axis {
            let sign = match channel.attr("orientation") {
                None | Some("+ve") => 1.0,
                Some("-ve") => -1.0,
                _ => return None,
            };
            *format.scale.get_mut(axis)? = unit(channel.attr("units"))? * sign;
        }
    }
    format.x = x?;
    format.y = y?;
    Some(format)
}

/// Convert a supported InkML physical unit into points; absent units mean one point per unit.
fn unit(value: Option<&str>) -> Option<f64> {
    match value {
        None | Some("em" | "pt") => Some(1.0),
        Some("in") => Some(72.0),
        Some("cm") => Some(72.0 / 2.54),
        Some("mm") => Some(72.0 / 25.4),
        Some("m") => Some(72.0 / 0.0254),
        Some("pc") => Some(12.0),
        _ => None,
    }
}

/// Whether a decoded coordinate or channel value is finite and within the parser's safe range.
fn bounded(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_VALUE
}

/// Parse a bounded decimal/exponent channel number, rejecting long and nonfinite values.
fn number(value: &str) -> Option<f64> {
    if value.len() > 64 {
        return None;
    }
    let value = value.parse::<f64>().ok()?;
    bounded(value).then_some(value)
}

/// Accumulated explicit, first-difference, and second-difference state for one channel.
#[derive(Clone, Default)]
struct ChannelState {
    /// Reconstructed absolute value at the current sample.
    value: Option<f64>,
    /// Reconstructed first difference used for second-difference samples.
    delta: Option<f64>,
    /// Last encoded value, reused when a `*` wildcard occurs.
    last: Option<f64>,
    /// Active encoding order: 0 explicit, 1 first difference, 2 second difference.
    order: u8,
}

impl ChannelState {
    /// Consume one trace token and update this channel's reconstructed value.
    fn next(&mut self, tokens: &mut Tokens<'_>) -> Option<()> {
        tokens.space();
        let prefix = match tokens.0.as_bytes().first()? {
            b'!' => Some(0),
            b'\'' => Some(1),
            b'"' => Some(2),
            _ => None,
        };
        if let Some(order) = prefix {
            self.order = order;
            tokens.0 = tokens.0.get(1..)?;
            tokens.space();
        }
        let value = if let Some(rest) = tokens.0.strip_prefix('*') {
            if prefix.is_some() {
                return None;
            }
            tokens.0 = rest;
            self.last?
        } else {
            tokens.number()?
        };
        let next = match self.order {
            0 => {
                self.delta = None;
                value
            }
            1 => {
                self.delta = Some(value);
                self.value? + value
            }
            2 => {
                let delta = self.delta? + value;
                self.delta = Some(delta);
                self.value? + delta
            }
            _ => return None,
        };
        if !bounded(next) {
            return None;
        }
        self.last = Some(value);
        self.value = Some(next);
        Some(())
    }
}

/// Cursor over a trace's compact comma-separated sample data.
struct Tokens<'a>(&'a str);

impl Tokens<'_> {
    /// Skip the XML whitespace allowed between trace values.
    fn space(&mut self) {
        self.0 = self.0.trim_start_matches([' ', '\t', '\r', '\n']);
    }

    /// Consume consecutive ASCII decimal digits and return their count.
    fn digits(&mut self) -> usize {
        let len = self.0.bytes().take_while(u8::is_ascii_digit).count();
        self.0 = self.0.get(len..).unwrap_or("");
        len
    }

    /// Parse one signed InkML decimal, exponent, or hexadecimal token from the cursor.
    fn number(&mut self) -> Option<f64> {
        self.space();
        let sign = if let Some(rest) = self.0.strip_prefix('-') {
            self.0 = rest;
            self.space();
            -1.0
        } else {
            1.0
        };
        if let Some(rest) = self.0.strip_prefix('#') {
            let len = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
            if len == 0 || len > 8 {
                return None;
            }
            let value = u32::from_str_radix(rest.get(..len)?, 16).ok()? as f64 * sign;
            self.0 = rest.get(len..)?;
            return bounded(value).then_some(value);
        }
        let start = self.0;
        let mut digits = self.digits();
        if let Some(rest) = self.0.strip_prefix('.') {
            self.0 = rest;
            digits += self.digits();
        }
        if digits == 0 {
            return None;
        }
        if self.0.starts_with(['e', 'E']) {
            self.0 = self.0.get(1..)?;
            if self.0.starts_with(['+', '-']) {
                self.0 = self.0.get(1..)?;
            }
            if self.digits() == 0 {
                return None;
            }
        }
        Some(number(start.get(..start.len().saturating_sub(self.0.len()))?)? * sign)
    }
}

/// Build bounded vector items, centring the strokes in their intrinsic graphic bounds.
fn drawing(strokes: Vec<Stroke>) -> Option<Graphic> {
    let (mut left, mut top, mut right, mut bottom) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for stroke in &strokes {
        let radius = f64::from(stroke.width) / 2.0;
        for &[x, y] in &stroke.points {
            left = left.min(x - radius);
            top = top.min(y - radius);
            right = right.max(x + radius);
            bottom = bottom.max(y + radius);
        }
    }
    let (w, h) = (right - left, bottom - top);
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let scale = (f64::from(crate::units::MAX_LEN_PT) / w.max(h)).min(1.0);
    let point = |[x, y]: [f64; 2]| (((x - left) * scale) as f32, ((y - top) * scale) as f32);
    let mut items = Vec::with_capacity(strokes.len());
    for stroke in strokes {
        let width = (f64::from(stroke.width) * scale) as f32;
        let item = if let [p] = stroke.points.as_slice() {
            let (x, y) = point(*p);
            GraphicItem::Shape {
                rect: [x - width / 2.0, y - width / 2.0, width, width],
                kind: ShapeKind::Ellipse,
                fill: Some(stroke.color),
                stroke: None,
                stroke_width: 0.0,
            }
        } else {
            let segs = stroke
                .points
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    let (x, y) = point(p);
                    if i == 0 { PathSeg::Move(x, y) } else { PathSeg::Line(x, y) }
                })
                .collect();
            GraphicItem::Path { segs, fill: None, stroke: Some(stroke.color), stroke_width: width }
        };
        items.push(item);
    }
    Some(Graphic { kind: GraphicKind::Ink, items, w: (w * scale) as f32, h: (h * scale) as f32, ..Default::default() })
}

#[cfg(test)]
mod tests;
