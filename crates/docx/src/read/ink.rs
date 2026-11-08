//! Bounded native InkML previews built from the public W3C Recommendation.
//!
//! Original package bytes remain authoritative for saves. This decoder builds disposable vector
//! geometry; unsupported visual semantics, invalid input and exhausted work refuse the entire
//! preview so the reader can retain its original picture fallback. No external URI is fetched.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::xml::El;
use brush::{Brush, Stroke};
use format::Format;
use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg};
use wordcraft_doc::props::Rgb;

mod brush;
mod format;
mod mapping;
mod projection;
mod trace;
mod units;
mod views;
mod xml;

/// File-wide allowance for InkML decoding and geometry copies, including failed attempts.
/// Kept separate from chart/SmartArt work so hostile ink cannot erase unrelated previews.
pub(super) const MAX_WORK: usize = 2_000_000;
/// One part cannot consume the file's entire allowance, leaving room for later small ink.
const MAX_PART_WORK: usize = 1_000_000;

/// Maximum InkML XML size accepted for vector decoding (original bytes are preserved separately).
const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Maximum number of indexed XML elements, including annotations and definitions.
const MAX_NODES: usize = 8192;
/// Maximum XML, reference, mapping and trace-view nesting depth.
const MAX_DEPTH: usize = 16;
/// Maximum combined regular/intermittent channels in a trace format.
const MAX_CHANNELS: usize = 16;
/// Maximum decoded samples, including repeated references and pen-up samples.
const MAX_POINTS: usize = 100_000;
/// Bound on input numbers and evaluated mapping intermediates.
const MAX_VALUE: f64 = 1_000_000_000.0;

/// Decode a supported native ink preview under the document's shared ink-work allowance.
pub(super) fn read(bytes: &[u8], work: &mut usize) -> Option<Graphic> {
    let allowance = (*work).min(MAX_PART_WORK);
    let mut left = allowance;
    let result = read_part(bytes, &mut left);
    // Debit successful reservations even when decoding later fails, but not a
    // rejected reservation or the unused part allowance. Repeated parts are cached.
    *work -= allowance - left;
    result
}

/// Decode one part using only the work reserved for it by `read`.
fn read_part(bytes: &[u8], work: &mut usize) -> Option<Graphic> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    spend(work, bytes.len().div_ceil(16))?;
    let root = xml::parse(bytes, work)?;
    if root.name != "ink:ink" {
        return None;
    }
    let mut reader = InkReader {
        ids: HashMap::new(),
        definitions: HashSet::new(),
        definition_traces: HashSet::new(),
        contexts: HashMap::new(),
        work,
        strokes: Vec::new(),
        points: 0,
        default_context: Context::default(),
        tails: HashMap::new(),
    };
    reader.index(&root, &mut 0, 0, false)?;
    reader.walk(&root, Context::default(), 0)?;
    if reader.strokes.is_empty() {
        // A valid pen-up-only part must not resurrect an unrelated picture fallback.
        return Some(Graphic {
            kind: GraphicKind::Ink,
            items: vec![GraphicItem::InkOutline { segs: Vec::new(), color: Rgb::BLACK, alpha: 0.0, mask_pen: false }],
            w: 1.0,
            h: 1.0,
            ..Default::default()
        });
    }
    brush::drawing(reader.strokes, reader.work)
}

/// Fit local geometry to a frame; circular widths use the mean of the composed axis scales.
pub(super) fn fit_frame(graphic: &mut Graphic, w: f32, h: f32) {
    if graphic.w <= 0.0 || graphic.h <= 0.0 {
        return;
    }
    let (sx, sy) = (w / graphic.w, h / graphic.h);
    let width_scale = graphic.ink_width_scale(sx, sy);
    let [x, y] = graphic.ink_scale.unwrap_or([1.0, 1.0]);
    graphic.ink_scale = Some([x * f64::from(sx), y * f64::from(sy)]);
    for item in &mut graphic.items {
        match item {
            GraphicItem::Path { segs, stroke_width, .. } => {
                scale_segments(segs, sx, sy);
                *stroke_width = (*stroke_width * width_scale).min(crate::units::MAX_LEN_PT);
            }
            GraphicItem::InkOutline { segs, .. } => scale_segments(segs, sx, sy),
            GraphicItem::Shape { rect, kind, fill, stroke, .. } => {
                let [x, y, width, height] = *rect;
                if *kind == wordcraft_doc::para::ShapeKind::Ellipse && fill.is_some() && stroke.is_none() && width == height {
                    let size = (width * width_scale).min(crate::units::MAX_LEN_PT);
                    *rect = [(x + width / 2.0) * sx - size / 2.0, (y + height / 2.0) * sy - size / 2.0, size, size];
                } else {
                    for (v, scale) in rect.iter_mut().zip([sx, sy, sx, sy]) {
                        *v *= scale;
                    }
                }
            }
            _ => {}
        }
    }
    graphic.w = w;
    graphic.h = h;
}

pub(super) fn place_frame(graphic: &mut Graphic, outer: (f32, f32), inner: [f32; 4]) {
    if graphic.items.is_empty() {
        return;
    }
    let [x, y, w, h] = inner;
    fit_frame(graphic, w, h);
    for item in &mut graphic.items {
        match item {
            GraphicItem::Path { segs, .. } | GraphicItem::InkOutline { segs, .. } => {
                for seg in segs {
                    match seg {
                        PathSeg::Move(a, b) | PathSeg::Line(a, b) => {
                            *a += x;
                            *b += y;
                        }
                        PathSeg::Cubic(a, b, c, d, e, f) => {
                            *a += x;
                            *b += y;
                            *c += x;
                            *d += y;
                            *e += x;
                            *f += y;
                        }
                        PathSeg::Close => {}
                    }
                }
            }
            GraphicItem::Shape { rect, .. } => {
                let [a, b, _, _] = rect;
                *a += x;
                *b += y;
            }
            _ => {}
        }
    }
    if w <= 0.0 || h <= 0.0 {
        graphic.items = vec![GraphicItem::InkOutline { segs: Vec::new(), color: Rgb::BLACK, alpha: 0.0, mask_pen: false }];
    }
    graphic.ink_frame = Some(inner);
    graphic.w = outer.0;
    graphic.h = outer.1;
}

/// Scale bounded path coordinates, including any future curve segments.
fn scale_segments(segs: &mut [PathSeg], sx: f32, sy: f32) {
    for seg in segs {
        match seg {
            PathSeg::Move(x, y) | PathSeg::Line(x, y) => {
                *x *= sx;
                *y *= sy;
            }
            PathSeg::Cubic(a, b, c, d, e, f) => {
                *a *= sx;
                *b *= sy;
                *c *= sx;
                *d *= sy;
                *e *= sx;
                *f *= sy;
            }
            PathSeg::Close => {}
        }
    }
}

/// Reserve before doing work; a refused operation spends nothing. Previously
/// completed operations remain charged even when the whole preview is rejected.
fn spend(work: &mut usize, amount: usize) -> Option<()> {
    *work = work.checked_sub(amount)?;
    Some(())
}

/// Accept only finite numbers small enough for bounded geometry and mapping evaluation.
fn bounded(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_VALUE
}

/// Parse a bounded decimal/exponent attribute or MathML literal.
fn number(value: &str) -> Option<f64> {
    if value.len() > 64 {
        return None;
    }
    let value = value.parse::<f64>().ok()?;
    bounded(value).then_some(value)
}

/// Fully resolved non-geometric context; large declarations are shared rather than cloned per sample.
#[derive(Clone, Default)]
struct Context<'a> {
    /// Recorded sample format, optionally calibrated by an ink source.
    format: Arc<Format>,
    /// Source metadata is retained when a later trace format overrides the source's format.
    source: Option<&'a El>,
    /// Brush property overrides.
    brush: Brush,
    /// Explicit output canvas; absent means use the source's physical coordinates.
    canvas: Option<Arc<Format>>,
    /// Forward mapping element, compiled against the resolved source/canvas formats per trace.
    transform: Option<&'a El>,
}

/// The last pen-down run of a continuation fragment, and whether another fragment may follow.
struct Tail {
    /// Index of the final stroke, absent if the fragment ends pen-up.
    stroke: Option<usize>,
    /// A begin/middle fragment may be consumed exactly once.
    open: bool,
}

/// Stateful stream reader with bounded local references and document-wide work accounting.
struct InkReader<'a, 'w> {
    /// Unique local IDs; URI references cannot escape this part.
    ids: HashMap<&'a str, &'a El>,
    /// IDs syntactically declared inside definitions, where context defaults are isolated.
    definitions: HashSet<&'a str>,
    /// Identity of definition-contained traces, including anonymous group children. Pointers
    /// are compared only, never dereferenced; the immutable XML tree outlives this reader.
    definition_traces: HashSet<*const El>,
    /// Contexts encountered in stream order retain their declaration-time defaults.
    contexts: HashMap<&'a str, Context<'a>>,
    /// Shared allowance for parsing, reference expansion, mappings, geometry and copying.
    work: &'w mut usize,
    /// Pen-down runs ready for vector construction.
    strokes: Vec<Stroke>,
    /// Decoded sample count including unknown/pen-up data and expanded views.
    points: usize,
    /// Stream properties used by the reserved DefaultContext URI.
    default_context: Context<'a>,
    /// Continuation references already seen, preventing forks, cycles and use after end.
    tails: HashMap<&'a str, Tail>,
}

impl<'a> InkReader<'a, '_> {
    /// Index uniquely identified elements and enforce nesting/node bounds before reference traversal.
    fn index(&mut self, el: &'a El, nodes: &mut usize, depth: usize, in_definitions: bool) -> Option<()> {
        spend(self.work, 1)?;
        *nodes += 1;
        if *nodes > MAX_NODES || depth > MAX_DEPTH {
            return None;
        }
        if matches!(el.name.as_str(), "ink:annotation" | "ink:annotationXML") {
            return Some(());
        }
        let in_definitions = in_definitions || el.name == "ink:definitions";
        if in_definitions && el.name == "ink:trace" {
            self.definition_traces.insert(el as *const El);
        }
        if let Some(id) = xml::id(el) {
            if id.is_empty()
                || id.len() > 256
                || matches!(id, "DefaultContext" | "DefaultBrush" | "DefaultCanvas" | "DefaultTraceFormat")
                || self.ids.insert(id, el).is_some()
            {
                return None;
            }
            if in_definitions {
                self.definitions.insert(id);
            }
        }
        for child in el.els() {
            self.index(child, nodes, depth + 1, in_definitions)?;
        }
        Some(())
    }

    /// Resolve only a local fragment of the expected InkML kind.
    fn resolve(&mut self, reference: &str, name: &str) -> Option<&'a El> {
        spend(self.work, 1)?;
        let el = *self.ids.get(reference.strip_prefix('#')?)?;
        (el.name == name).then_some(el)
    }

    /// Resolve a brush's inheritance chain, bounded independently of XML nesting.
    fn brush(&mut self, el: &'a El, depth: usize) -> Option<Brush> {
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
        brush.properties(el, self.work)?;
        Some(brush)
    }

    /// Resolve a format child before its reference; overridden references need not be valid.
    fn format(&mut self, el: &'a El) -> Option<Option<Format>> {
        if let Some(format) = el.child("ink:traceFormat") {
            return Some(Some(format::read(format, self.work)?));
        }
        match el.attr("traceFormatRef") {
            None => Some(None),
            Some("#DefaultTraceFormat") => Some(Some(Format::default())),
            Some(reference) => {
                let format = self.resolve(reference, "ink:traceFormat")?;
                Some(Some(format::read(format, self.work)?))
            }
        }
    }

    /// Resolve context inheritance and property precedence according to InkML §4.6.
    fn context(&mut self, el: &'a El, inherited: Context<'a>, depth: usize) -> Option<Context<'a>> {
        spend(self.work, el.kids.len().saturating_add(el.attrs.len()).saturating_mul(12).max(1))?;
        if depth > MAX_DEPTH {
            return None;
        }
        let mut ctx = match el.attr("contextRef") {
            None => inherited,
            Some("#DefaultContext") => self.default_context.clone(),
            Some(reference) => {
                let id = reference.strip_prefix('#')?;
                if let Some(ctx) = self.contexts.get(id) {
                    ctx.clone()
                } else {
                    let parent = self.resolve(reference, "ink:context")?;
                    // Definitions are independent of stream state. Forward references outside
                    // definitions are not previously defined contexts and cannot be guessed.
                    if !self.definitions.contains(id) {
                        return None;
                    }
                    self.context(parent, Context::default(), depth + 1)?
                }
            }
        };
        if let Some(source) = el.child("ink:inkSource") {
            ctx.source = Some(source);
            ctx.format = Arc::new(format::source(source, self.work)?);
        } else if let Some(reference) = el.attr("inkSourceRef") {
            let source = self.resolve(reference, "ink:inkSource")?;
            ctx.source = Some(source);
            ctx.format = Arc::new(format::source(source, self.work)?);
        }
        if let Some(mut format) = self.format(el)? {
            if let Some(source) = ctx.source {
                format::calibrate(&mut format, source, self.work)?;
            }
            ctx.format = Arc::new(format);
        }
        if let Some(canvas) = el.child("ink:canvas") {
            ctx.canvas = Some(Arc::new(self.format(canvas)??));
        } else if let Some(reference) = el.attr("canvasRef") {
            ctx.canvas = Some(Arc::new(if reference == "#DefaultCanvas" {
                Format::default()
            } else {
                let canvas = self.resolve(reference, "ink:canvas")?;
                self.format(canvas)??
            }));
        }
        if let Some(transform) = el.child("ink:canvasTransform") {
            ctx.transform = Some(transform);
        } else if let Some(reference) = el.attr("canvasTransformRef") {
            ctx.transform = Some(self.resolve(reference, "ink:canvasTransform")?);
        }
        if let Some(brush) = el.child("ink:brush") {
            ctx.brush = self.brush(brush, 0)?;
        } else if let Some(reference) = el.attr("brushRef") {
            ctx.brush = if reference == "#DefaultBrush" {
                Brush::default()
            } else {
                let brush = self.resolve(reference, "ink:brush")?;
                self.brush(brush, 0)?
            };
        }
        Some(ctx)
    }

    /// Process stream elements in order without leaking group properties into following siblings.
    fn walk(&mut self, el: &'a El, mut ctx: Context<'a>, depth: usize) -> Option<()> {
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
                "ink:traceFormat" => {
                    let mut format = format::read(child, self.work)?;
                    if let Some(source) = ctx.source {
                        format::calibrate(&mut format, source, self.work)?;
                    }
                    ctx.format = Arc::new(format);
                }
                "ink:inkSource" => {
                    ctx.source = Some(child);
                    ctx.format = Arc::new(format::source(child, self.work)?);
                }
                "ink:brush" => ctx.brush = self.brush(child, 0)?,
                "ink:context" => {
                    ctx = self.context(child, ctx, 0)?;
                    if let Some(id) = xml::id(child) {
                        self.contexts.insert(id, ctx.clone());
                    }
                }
                "ink:traceGroup" => {
                    let group = self.context(child, ctx.clone(), 0)?;
                    self.walk(child, group, depth + 1)?;
                }
                "ink:trace" => {
                    let trace = self.context(child, ctx.clone(), 0)?;
                    self.trace(child, &trace, None)?;
                }
                "ink:traceView" => self.view(child, ctx.clone(), depth + 1)?,
                _ => return None,
            }
        }
        Some(())
    }

    fn prior(&mut self, el: &'a El, range: Option<(usize, usize)>) -> Option<Option<usize>> {
        match (el.attr("continuation"), el.attr("priorRef")) {
            (None, None) | (Some("begin"), None) => Some(None),
            (Some("middle" | "end"), Some(reference)) => {
                if range.is_some() {
                    self.validate_prior(el, 0)?;
                }
                let id = reference.strip_prefix('#')?;
                let Some(prior) = self.tails.get_mut(id) else {
                    return range.map(|_| None);
                };
                if !prior.open {
                    return range.map(|_| None);
                }
                prior.open = false;
                Some(if range.is_none_or(|r| r.0 == 1) { prior.stroke } else { None })
            }
            _ => None,
        }
    }

    fn validate_prior(&mut self, el: &'a El, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        spend(self.work, el.attrs.len().saturating_mul(8).max(1))?;
        match (el.attr("continuation"), el.attr("priorRef")) {
            (Some("begin"), None) => Some(()),
            (Some("middle" | "end"), Some(reference)) => {
                let prior = self.resolve(reference, "ink:trace")?;
                if !matches!(prior.attr("continuation"), Some("begin" | "middle")) {
                    return None;
                }
                self.validate_prior(prior, depth + 1)
            }
            _ => None,
        }
    }

    /// Decode, map and append one trace fragment. A view optionally selects a 1-based inclusive range.
    fn trace(&mut self, el: &'a El, ctx: &Context<'a>, range: Option<(usize, usize)>) -> Option<()> {
        if el.els().next().is_some() {
            return None;
        }
        let default_down = match el.attr("type") {
            None | Some("penDown") => Some(true),
            Some("penUp") => Some(false),
            Some("indeterminate") => None,
            _ => return None,
        };
        let mut tail = self.prior(el, range)?;
        let samples = trace::read(&el.text(), &ctx.format, self.work, &mut self.points)?;
        let (from, to) = range.unwrap_or((1, samples.len()));
        if from == 0 || to < from || to > samples.len() {
            return None;
        }
        let projection = projection::Projection::new(ctx, &self.ids, self.work)?;
        let visual = &projection.format;
        for values in samples.get(from - 1..to)? {
            let resolved = projection.apply(values, self.work)?;
            let down = if visual.index("S").is_some() {
                match visual.value(&resolved, "S") {
                    Some(0.0) => false,
                    Some(1.0) => true,
                    _ => return None,
                }
            } else {
                default_down?
            };
            if !down {
                tail = None;
                continue;
            }
            let xy = [visual.value(&resolved, "X")?, visual.value(&resolved, "Y")?];
            if !xy.iter().all(|v| bounded(*v)) {
                return None;
            }
            let point = ctx.brush.point(visual, &resolved, xy)?;
            match tail {
                Some(i) => {
                    let stroke = self.strokes.get_mut(i)?;
                    if stroke.curve != ctx.brush.curve {
                        return None;
                    }
                    stroke.points.push(point);
                }
                None => {
                    tail = Some(self.strokes.len());
                    self.strokes.push(Stroke { points: vec![point], curve: ctx.brush.curve });
                }
            }
        }
        if let Some(id) = xml::id(el) {
            self.tails.insert(id, Tail { stroke: tail, open: to == samples.len() && matches!(el.attr("continuation"), Some("begin" | "middle")) });
        }
        Some(())
    }
}

#[cfg(test)]
mod tests;
