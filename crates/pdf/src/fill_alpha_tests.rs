use super::*;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use kurbo::Shape;
use wordcraft_doc::connector::ShapeExtra;
use wordcraft_doc::effects::ShapeEffects;
use wordcraft_doc::freeform::{FreePath, Freeform};
use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::{CharProps, Document, Paragraph, para_block};

const ALPHA_TOLERANCE: f32 = 0.015;
const RED: wordcraft_doc::Rgb = wordcraft_doc::Rgb(255, 0, 0);
const GREEN: wordcraft_doc::Rgb = wordcraft_doc::Rgb(0, 255, 0);

struct PaintedPath {
    path: kurbo::BezPath,
    color: [u8; 4],
}

struct Paints {
    fills: Vec<PaintedPath>,
    strokes: Vec<PaintedPath>,
    groups: Vec<f32>,
}

impl Device<'_> for Paints {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, path: &kurbo::BezPath, transform: kurbo::Affine, paint: &Paint<'_>, mode: &PathDrawMode) {
        let Paint::Color(color) = paint else { panic!("solid fixture paint") };
        let mut color = color.to_rgba().to_rgba8();
        color[3] = (f32::from(color[3]) * self.groups.last().copied().unwrap_or(1.0)).round() as u8;
        let record = PaintedPath { path: transform * path.clone(), color };
        match mode {
            PathDrawMode::Fill(_) => self.fills.push(record),
            PathDrawMode::Stroke(_) => self.strokes.push(record),
        }
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn pop_clip_path(&mut self) {}
    fn push_transparency_group(&mut self, alpha: f32, _: Option<SoftMask<'_>>, _: BlendMode) {
        self.groups.push(self.groups.last().copied().unwrap_or(1.0) * alpha);
    }
    fn pop_transparency_group(&mut self) {
        assert!(self.groups.len() > 1);
        let _ = self.groups.pop();
    }
    fn draw_glyph(&mut self, _: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
}

impl Paints {
    fn red_coverage(&self, point: (f64, f64)) -> f32 {
        1.0 - self
            .fills
            .iter()
            .filter(|r| r.color[..3] == [255, 0, 0] && r.path.contains(point.into()))
            .map(|r| 1.0 - f32::from(r.color[3]) / 255.0)
            .product::<f32>()
    }
}

fn paints(shape: InlineObject) -> Paints {
    let mut doc = Document::new();
    let mut para = Paragraph::new();
    para.insert_object(0, shape, &CharProps::default()).unwrap();
    doc.body = vec![para_block(para)];
    let bytes = export(&doc, &PdfOptions { tagged: false, ..Default::default() }).unwrap();
    let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
    let cache = InterpreterCache::new();
    let mut paints = Paints { fills: Vec::new(), strokes: Vec::new(), groups: vec![1.0] };
    for page in pdf.pages().iter() {
        let bounds = page.media_box();
        let (width, height) = (bounds.width(), bounds.height());
        // PDF uses bottom-up coordinates; the fixture points use the layout's top-down frame.
        let top_down = kurbo::Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, height]);
        let mut ctx = Context::new(top_down, kurbo::Rect::new(0.0, 0.0, width, height), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut paints);
    }
    assert_eq!(paints.groups, [1.0]);
    paints
}

fn shape(kind: ShapeKind, transparency: f32, effects: ShapeEffects) -> InlineObject {
    InlineObject::Shape {
        kind,
        w: 24.0,
        h: 24.0,
        fill: Some(RED),
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
        story: None,
        freeform: None,
        effects,
        extra: ShapeExtra { fill_transparency: transparency, ..Default::default() },
    }
}

fn close(actual: f32, want: f32) {
    assert!((actual - want).abs() < ALPHA_TOLERANCE, "{actual} != {want}");
}

#[test]
fn freeform_fill_keeps_outline() {
    for (transparency, alpha) in [(0.0, 1.0), (50.0, 0.5), (100.0, 0.0)] {
        let mut item = shape(ShapeKind::Freeform, transparency, ShapeEffects::default());
        if let InlineObject::Shape { freeform, stroke, stroke_width, .. } = &mut item {
            *freeform = Some(std::sync::Arc::new(Freeform {
                w: 24.0,
                h: 24.0,
                paths: vec![FreePath { pts: vec![[0.0, 0.0], [24.0, 0.0], [24.0, 24.0], [0.0, 24.0]], closed: true }],
                alpha: 0.25,
                ..Default::default()
            }));
            *stroke = Some(GREEN);
            *stroke_width = 2.0;
        }
        let paints = paints(item);
        close(paints.red_coverage((112.0, 112.0)), alpha);
        assert_eq!(paints.strokes.len(), 1);
        assert_eq!(paints.strokes[0].color, [0, 255, 0, 255]);
    }
}

#[test]
fn soft_edge_target_alpha() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for (radius, count) in [(3.0, 3.0), (6.0, 4.0)] {
            for (transparency, alpha) in [(0.0, 1.0), (50.0, 0.5), (100.0, 0.0), (f32::NAN, 1.0)] {
                let effects = ShapeEffects { soft_edge: Some(radius), ..Default::default() };
                let paints = paints(shape(kind, transparency, effects));
                close(paints.red_coverage((112.0, 112.0)), alpha);
                close(paints.red_coverage((101.125, 112.0)), alpha / count);
                close(paints.red_coverage((99.0, 112.0)), 0.0);
                assert!(paints.strokes.is_empty());
            }
        }
    }
}

#[test]
fn soft_edge_keeps_effect_paint() {
    let effects = ShapeEffects {
        shadow: Some(wordcraft_doc::effects::Shadow::default()),
        glow: Some(wordcraft_doc::effects::Glow::default()),
        soft_edge: Some(6.0),
    };
    let opaque = paints(shape(ShapeKind::Rectangle, 0.0, effects));
    let clear = paints(shape(ShapeKind::Rectangle, 100.0, effects));
    let other = |r: &&PaintedPath| r.color[..3] != [255, 0, 0];
    let before: Vec<_> = opaque.fills.iter().filter(other).map(|r| (&r.path, r.color)).collect();
    let after: Vec<_> = clear.fills.iter().filter(other).map(|r| (&r.path, r.color)).collect();
    assert!(!before.is_empty());
    assert_eq!(before, after);
    assert!(opaque.strokes.is_empty() && clear.strokes.is_empty());
    for (transparency, alpha) in [(0.0, 1.0), (50.0, 0.5), (100.0, 0.0)] {
        close(paints(shape(ShapeKind::Rectangle, transparency, ShapeEffects::default())).red_coverage((112.0, 112.0)), alpha);
    }
}
