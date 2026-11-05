use std::collections::HashMap;

use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg};
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;

use crate::xml::El;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_NODES: usize = 8192;
const MAX_DEPTH: usize = 16;
const MAX_CHANNELS: usize = 16;
const MAX_POINTS: usize = 100_000;
const MAX_VALUE: f64 = 1_000_000_000.0;

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

fn spend(work: &mut usize, amount: usize) -> Option<()> {
    let enough = *work >= amount;
    *work = work.saturating_sub(amount);
    enough.then_some(())
}

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

#[derive(Clone)]
struct Format {
    channels: usize,
    x: usize,
    y: usize,
    scale: [f64; 2],
}

impl Default for Format {
    fn default() -> Self {
        Self { channels: 2, x: 0, y: 1, scale: [1.0; 2] }
    }
}

#[derive(Clone, Default)]
struct Brush {
    color: Option<Rgb>,
    width: Option<f64>,
    height: Option<f64>,
}

#[derive(Clone, Default)]
struct Context {
    format: Format,
    brush: Brush,
}

struct Stroke {
    points: Vec<[f64; 2]>,
    color: Rgb,
    width: f32,
}

struct InkReader<'a, 'w> {
    ids: HashMap<&'a str, &'a El>,
    work: &'w mut usize,
    strokes: Vec<Stroke>,
    points: usize,
    default_context: Context,
}

impl<'a> InkReader<'a, '_> {
    fn resolve(&mut self, reference: &str, name: &str) -> Option<&'a El> {
        spend(self.work, 1)?;
        let id = reference.strip_prefix('#')?;
        let el = *self.ids.get(id)?;
        (el.name == name).then_some(el)
    }

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

fn source_format(el: &El) -> Option<Format> {
    trace_format(el.child("ink:traceFormat")?)
}

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

fn bounded(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_VALUE
}

fn number(value: &str) -> Option<f64> {
    if value.len() > 64 {
        return None;
    }
    let value = value.parse::<f64>().ok()?;
    bounded(value).then_some(value)
}

#[derive(Clone, Default)]
struct ChannelState {
    value: Option<f64>,
    delta: Option<f64>,
    last: Option<f64>,
    order: u8,
}

impl ChannelState {
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

struct Tokens<'a>(&'a str);

impl Tokens<'_> {
    fn space(&mut self) {
        self.0 = self.0.trim_start_matches([' ', '\t', '\r', '\n']);
    }

    fn digits(&mut self) -> usize {
        let len = self.0.bytes().take_while(u8::is_ascii_digit).count();
        self.0 = self.0.get(len..).unwrap_or("");
        len
    }

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
