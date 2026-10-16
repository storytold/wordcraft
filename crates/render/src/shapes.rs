//! Kurbo path geometry for Office shapes and UI preview contours.

use vello_cpu::kurbo::{self, BezPath, Shape};
use wordcraft_doc::para::ShapeKind;

pub fn shape_path(kind: ShapeKind, r: kurbo::Rect, corner: Option<u32>) -> BezPath {
    let (cx, cy) = (r.center().x, r.center().y);
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);

    let poly = |pts: &[(f64, f64)]| {
        let mut p = BezPath::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                p.move_to((*x, *y));
            } else {
                p.line_to((*x, *y));
            }
        }
        p.close_path();
        p
    };

    let ngon = |sides: usize, rot: f64| {
        let mut pts = Vec::with_capacity(sides);
        let rx = w / 2.0;
        let ry = h / 2.0;
        for i in 0..sides {
            let a = rot + 2.0 * std::f64::consts::PI * (i as f64) / (sides as f64);
            pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
        }
        poly(&pts)
    };

    let star = |pts_count: usize, inner_ratio: f64| {
        let mut pts = Vec::with_capacity(pts_count * 2);
        let rx = w / 2.0;
        let ry = h / 2.0;
        for i in 0..(pts_count * 2) {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * (i as f64) / (pts_count as f64);
            let rad = if i % 2 == 0 { 1.0 } else { inner_ratio };
            pts.push((cx + rx * rad * a.cos(), cy + ry * rad * a.sin()));
        }
        poly(&pts)
    };

    match kind {
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::DrawingCanvas | ShapeKind::FlowchartProcess => r.to_path(0.1),
        ShapeKind::RoundedRectangle => {
            let radius = wordcraft_doc::connector::corner_radius(corner, r.width() as f32, r.height() as f32);
            kurbo::RoundedRect::from_rect(r, f64::from(radius)).to_path(0.1)
        }
        ShapeKind::FlowchartAlternateProcess => kurbo::RoundedRect::from_rect(r, w.min(h) * 0.16).to_path(0.1),
        ShapeKind::SingleCornerSnipped => {
            let cut = w.min(h) * 0.2;
            poly(&[(x0 + cut, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::TopCornersSnipped => {
            let cut = w.min(h) * 0.2;
            poly(&[(x0 + cut, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::DiagonalCornersSnipped => {
            let cut = w.min(h) * 0.2;
            poly(&[(x0 + cut, y0), (x1, y0), (x1, y1 - cut), (x1 - cut, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::SnippedAndRounded => {
            let cut = w.min(h) * 0.2;
            poly(&[(x0 + cut, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1), (x0, y1)])
        }
        ShapeKind::RoundSingleCorner => {
            let rad = w.min(h) * 0.25;
            let mut p = BezPath::new();
            p.move_to((x0 + rad, y0));
            p.line_to((x1, y0));
            p.line_to((x1, y1));
            p.line_to((x0, y1));
            p.line_to((x0, y0 + rad));
            p.quad_to((x0, y0), (x0 + rad, y0));
            p.close_path();
            p
        }
        ShapeKind::RoundSameSideCorner => {
            let rad = w.min(h) * 0.25;
            let mut p = BezPath::new();
            p.move_to((x0 + rad, y0));
            p.line_to((x1 - rad, y0));
            p.quad_to((x1, y0), (x1, y0 + rad));
            p.line_to((x1, y1));
            p.line_to((x0, y1));
            p.line_to((x0, y0 + rad));
            p.quad_to((x0, y0), (x0 + rad, y0));
            p.close_path();
            p
        }
        ShapeKind::RoundDiagonalCorner => {
            let rad = w.min(h) * 0.25;
            let mut p = BezPath::new();
            p.move_to((x0 + rad, y0));
            p.line_to((x1, y0));
            p.line_to((x1, y1 - rad));
            p.quad_to((x1, y1), (x1 - rad, y1));
            p.line_to((x0, y1));
            p.line_to((x0, y0 + rad));
            p.quad_to((x0, y0), (x0 + rad, y0));
            p.close_path();
            p
        }
        ShapeKind::Ellipse | ShapeKind::FlowchartConnector => kurbo::Ellipse::from_rect(r).to_path(0.1),
        ShapeKind::Triangle | ShapeKind::FlowchartExtract => poly(&[(cx, y0), (x1, y1), (x0, y1)]),
        ShapeKind::RightTriangle => poly(&[(x0, y0), (x1, y1), (x0, y1)]),
        ShapeKind::Parallelogram | ShapeKind::FlowchartData => {
            let off = w * 0.2;
            poly(&[(x0 + off, y0), (x1, y0), (x1 - off, y1), (x0, y1)])
        }
        ShapeKind::Trapezoid | ShapeKind::FlowchartManualOperation => {
            let off = w * 0.2;
            poly(&[(x0 + off, y0), (x1 - off, y0), (x1, y1), (x0, y1)])
        }
        ShapeKind::Diamond | ShapeKind::FlowchartDecision => poly(&[(cx, y0), (x1, cy), (cx, y1), (x0, cy)]),
        ShapeKind::Pentagon => ngon(5, -std::f64::consts::FRAC_PI_2),
        ShapeKind::Hexagon | ShapeKind::FlowchartPreparation => ngon(6, 0.0),
        ShapeKind::Heptagon => ngon(7, -std::f64::consts::FRAC_PI_2),
        ShapeKind::Octagon => ngon(8, std::f64::consts::PI / 8.0),
        ShapeKind::Decagon => ngon(10, -std::f64::consts::FRAC_PI_2),
        ShapeKind::Dodecagon => ngon(12, 0.0),

        ShapeKind::Pie => {
            let mut p = BezPath::new();
            p.move_to((cx, cy));
            p.line_to((x1, cy));
            let mut sub = BezPath::new();
            let ell = kurbo::Ellipse::from_rect(r);
            sub.extend(ell.to_path(0.1).elements().iter().copied());
            p.extend(sub.elements().iter().copied());
            p.close_path();
            p
        }
        ShapeKind::Chord => {
            let mut p = BezPath::new();
            p.move_to((x0, cy));
            p.quad_to((cx, y1), (x1, cy));
            p.close_path();
            p
        }
        ShapeKind::Teardrop => {
            let mut p = BezPath::new();
            p.move_to((cx, y0));
            p.quad_to((x1, y0), (x1, cy));
            p.quad_to((x1, y1), (cx, y1));
            p.quad_to((x0, y1), (x0, cy));
            p.line_to((cx, y0));
            p.close_path();
            p
        }
        ShapeKind::Frame => {
            let th = w.min(h) * 0.15;
            let mut p = r.to_path(0.1);
            p.extend(kurbo::Rect::new(x0 + th, y0 + th, x1 - th, y1 - th).to_path(0.1).elements().iter().copied());
            p
        }
        ShapeKind::HalfFrame => {
            let th = w.min(h) * 0.15;
            poly(&[(x0, y0), (x1, y0), (x1, y0 + th), (x0 + th, y0 + th), (x0 + th, y1), (x0, y1)])
        }
        ShapeKind::LShape => {
            let th = w.min(h) * 0.25;
            poly(&[(x0, y0), (x0 + th, y0), (x0 + th, y1 - th), (x1, y1 - th), (x1, y1), (x0, y1)])
        }
        ShapeKind::DiagonalStripe => {
            let th = w.min(h) * 0.3;
            poly(&[(x0 + th, y0), (x1, y1 - th), (x1, y1), (x1 - th, y1), (x0, y0 + th), (x0, y0)])
        }
        ShapeKind::Cross | ShapeKind::MathPlus => {
            let xw = w * 0.3;
            let yh = h * 0.3;
            poly(&[
                (cx - xw / 2.0, y0),
                (cx + xw / 2.0, y0),
                (cx + xw / 2.0, cy - yh / 2.0),
                (x1, cy - yh / 2.0),
                (x1, cy + yh / 2.0),
                (cx + xw / 2.0, cy + yh / 2.0),
                (cx + xw / 2.0, y1),
                (cx - xw / 2.0, y1),
                (cx - xw / 2.0, cy + yh / 2.0),
                (x0, cy + yh / 2.0),
                (x0, cy - yh / 2.0),
                (cx - xw / 2.0, cy - yh / 2.0),
            ])
        }
        ShapeKind::Plaque => {
            let cut = w.min(h) * 0.2;
            poly(&[(x0 + cut, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1 - cut), (x1 - cut, y1), (x0 + cut, y1), (x0, y1 - cut), (x0, y0 + cut)])
        }
        ShapeKind::Cylinder | ShapeKind::FlowchartMagneticDisk => {
            let cap = h * 0.2;
            let mut p = BezPath::new();
            p.move_to((x0, y0 + cap / 2.0));
            p.quad_to((cx, y0 - cap / 2.0), (x1, y0 + cap / 2.0));
            p.line_to((x1, y1 - cap / 2.0));
            p.quad_to((cx, y1 + cap / 2.0), (x0, y1 - cap / 2.0));
            p.close_path();
            p
        }
        ShapeKind::Cube => {
            let off = w.min(h) * 0.25;
            let mut p = poly(&[(x0, y0 + off), (x1 - off, y0 + off), (x1 - off, y1), (x0, y1)]);
            p.move_to((x0, y0 + off));
            p.line_to((x0 + off, y0));
            p.line_to((x1, y0));
            p.line_to((x1 - off, y0 + off));
            p.move_to((x1, y0));
            p.line_to((x1, y1 - off));
            p.line_to((x1 - off, y1));
            p
        }
        ShapeKind::Bevel => {
            let th = w.min(h) * 0.15;
            let mut p = r.to_path(0.1);
            p.move_to((x0, y0));
            p.line_to((x0 + th, y0 + th));
            p.move_to((x1, y0));
            p.line_to((x1 - th, y0 + th));
            p.move_to((x1, y1));
            p.line_to((x1 - th, y1 - th));
            p.move_to((x0, y1));
            p.line_to((x0 + th, y1 - th));
            p
        }
        ShapeKind::Donut => {
            let mut p = kurbo::Ellipse::from_rect(r).to_path(0.1);
            p.extend(
                kurbo::Ellipse::from_rect(kurbo::Rect::new(x0 + w * 0.2, y0 + h * 0.2, x1 - w * 0.2, y1 - h * 0.2))
                    .to_path(0.1)
                    .elements()
                    .iter()
                    .copied(),
            );
            p
        }
        ShapeKind::NoSymbol => {
            let mut p = kurbo::Ellipse::from_rect(r).to_path(0.1);
            p.move_to((x0 + w * 0.15, y0 + h * 0.15));
            p.line_to((x1 - w * 0.15, y1 - h * 0.15));
            p
        }
        ShapeKind::BlockArc => {
            let mut p = BezPath::new();
            p.move_to((x0, cy));
            p.quad_to((x0, y0), (cx, y0));
            p.line_to((cx, y0 + h * 0.2));
            p.quad_to((x0 + w * 0.2, y0 + h * 0.2), (x0 + w * 0.2, cy));
            p.close_path();
            p
        }
        ShapeKind::FoldedCorner => {
            let cut = w.min(h) * 0.25;
            poly(&[(x0, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1), (x0, y1)])
        }
        ShapeKind::SmileyFace => {
            let mut p = kurbo::Ellipse::from_rect(r).to_path(0.1);
            p.move_to((cx - w * 0.2, cy - h * 0.15));
            p.line_to((cx - w * 0.2, cy - h * 0.05));
            p.move_to((cx + w * 0.2, cy - h * 0.15));
            p.line_to((cx + w * 0.2, cy - h * 0.05));
            p.move_to((cx - w * 0.25, cy + h * 0.15));
            p.quad_to((cx, cy + h * 0.35), (cx + w * 0.25, cy + h * 0.15));
            p
        }
        ShapeKind::Heart => {
            let mut p = BezPath::new();
            p.move_to((cx, y0 + h * 0.3));
            p.curve_to((cx, y0), (x0, y0), (x0, y0 + h * 0.3));
            p.curve_to((x0, y0 + h * 0.6), (cx - w * 0.1, y0 + h * 0.75), (cx, y1));
            p.curve_to((cx + w * 0.1, y0 + h * 0.75), (x1, y0 + h * 0.6), (x1, y0 + h * 0.3));
            p.curve_to((x1, y0), (cx, y0), (cx, y0 + h * 0.3));
            p.close_path();
            p
        }
        ShapeKind::LightningBolt => poly(&[
            (cx + w * 0.1, y0),
            (x0 + w * 0.1, cy + h * 0.05),
            (cx, cy + h * 0.05),
            (cx - w * 0.1, y1),
            (x1 - w * 0.1, cy - h * 0.05),
            (cx, cy - h * 0.05),
        ]),
        ShapeKind::Sun => {
            let mut p = star(12, 0.75);
            p.extend(
                kurbo::Ellipse::from_rect(kurbo::Rect::new(x0 + w * 0.2, y0 + h * 0.2, x1 - w * 0.2, y1 - h * 0.2))
                    .to_path(0.1)
                    .elements()
                    .iter()
                    .copied(),
            );
            p
        }
        ShapeKind::Moon => {
            let mut p = BezPath::new();
            p.move_to((cx, y0));
            p.quad_to((x1, cy), (cx, y1));
            p.quad_to((cx + w * 0.15, cy), (cx, y0));
            p.close_path();
            p
        }
        ShapeKind::Cloud | ShapeKind::CloudCallout => {
            let mut p = BezPath::new();
            p.move_to((x0 + w * 0.2, y1 - h * 0.2));
            p.quad_to((x0, y1 - h * 0.2), (x0, cy));
            p.quad_to((x0, y0 + h * 0.1), (x0 + w * 0.3, y0 + h * 0.15));
            p.quad_to((cx, y0), (cx + w * 0.2, y0 + h * 0.15));
            p.quad_to((x1, y0 + h * 0.1), (x1, cy));
            p.quad_to((x1, y1 - h * 0.2), (x1 - w * 0.2, y1 - h * 0.2));
            p.close_path();
            p
        }

        // Lines & Connectors
        ShapeKind::Line | ShapeKind::StraightConnector => {
            let mut p = BezPath::new();
            p.move_to((x0, y0));
            p.line_to((x1, y1));
            p
        }
        ShapeKind::Arrow => poly(&[
            (x0, cy - h * 0.2),
            (x1 - h * 0.5, cy - h * 0.2),
            (x1 - h * 0.5, y0),
            (x1, cy),
            (x1 - h * 0.5, y1),
            (x1 - h * 0.5, cy + h * 0.2),
            (x0, cy + h * 0.2),
        ]),
        ShapeKind::DoubleArrow => {
            let ah = h * 0.4;
            poly(&[
                (x0 + ah, y0),
                (x0, cy),
                (x0 + ah, y1),
                (x0 + ah, cy + h * 0.15),
                (x1 - ah, cy + h * 0.15),
                (x1 - ah, y1),
                (x1, cy),
                (x1 - ah, y0),
                (x1 - ah, cy - h * 0.15),
                (x0 + ah, cy - h * 0.15),
            ])
        }
        ShapeKind::ElbowConnector | ShapeKind::ElbowArrow | ShapeKind::ElbowDoubleArrow => {
            let mut p = BezPath::new();
            p.move_to((x0, y0));
            p.line_to((cx, y0));
            p.line_to((cx, y1));
            p.line_to((x1, y1));
            if kind == ShapeKind::ElbowArrow || kind == ShapeKind::ElbowDoubleArrow {
                p.move_to((x1 - 6.0, y1 - 4.0));
                p.line_to((x1, y1));
                p.line_to((x1 - 6.0, y1 + 4.0));
            }
            if kind == ShapeKind::ElbowDoubleArrow {
                p.move_to((x0 + 6.0, y0 - 4.0));
                p.line_to((x0, y0));
                p.line_to((x0 + 6.0, y0 + 4.0));
            }
            p
        }
        ShapeKind::CurvedConnector | ShapeKind::CurvedArrow | ShapeKind::CurvedDoubleArrow | ShapeKind::Curve | ShapeKind::Arc => {
            let mut p = BezPath::new();
            p.move_to((x0, y1));
            p.quad_to((x0, y0), (x1, y0));
            if kind == ShapeKind::CurvedArrow || kind == ShapeKind::CurvedDoubleArrow {
                p.move_to((x1 - 6.0, y0 - 4.0));
                p.line_to((x1, y0));
                p.line_to((x1 - 6.0, y0 + 4.0));
            }
            p
        }
        ShapeKind::Freeform | ShapeKind::Scribble => {
            let mut p = BezPath::new();
            p.move_to((x0, cy));
            p.quad_to((x0 + w * 0.25, y0), (cx, cy));
            p.quad_to((x0 + w * 0.75, y1), (x1, cy));
            p
        }
        ShapeKind::DoubleBracket | ShapeKind::LeftBracket | ShapeKind::RightBracket => {
            let mut p = BezPath::new();
            p.move_to((x0 + w * 0.3, y0));
            p.line_to((x0, y0));
            p.line_to((x0, y1));
            p.line_to((x0 + w * 0.3, y1));
            if kind != ShapeKind::LeftBracket {
                p.move_to((x1 - w * 0.3, y0));
                p.line_to((x1, y0));
                p.line_to((x1, y1));
                p.line_to((x1 - w * 0.3, y1));
            }
            p
        }
        ShapeKind::DoubleBrace | ShapeKind::LeftBrace | ShapeKind::RightBrace => {
            let mut p = BezPath::new();
            p.move_to((x0 + w * 0.3, y0));
            p.quad_to((x0 + w * 0.15, y0), (x0 + w * 0.15, cy - h * 0.1));
            p.line_to((x0, cy));
            p.line_to((x0 + w * 0.15, cy + h * 0.1));
            p.quad_to((x0 + w * 0.15, y1), (x0 + w * 0.3, y1));
            if kind != ShapeKind::LeftBrace {
                p.move_to((x1 - w * 0.3, y0));
                p.quad_to((x1 - w * 0.15, y0), (x1 - w * 0.15, cy - h * 0.1));
                p.line_to((x1, cy));
                p.line_to((x1 - w * 0.15, cy + h * 0.1));
                p.quad_to((x1 - w * 0.15, y1), (x1 - w * 0.3, y1));
            }
            p
        }

        // Block Arrows
        ShapeKind::LeftArrow => poly(&[
            (x0, cy),
            (x0 + h * 0.5, y0),
            (x0 + h * 0.5, cy - h * 0.2),
            (x1, cy - h * 0.2),
            (x1, cy + h * 0.2),
            (x0 + h * 0.5, cy + h * 0.2),
            (x0 + h * 0.5, y1),
        ]),
        ShapeKind::UpArrow => poly(&[
            (cx, y0),
            (x1, y0 + w * 0.5),
            (cx + w * 0.2, y0 + w * 0.5),
            (cx + w * 0.2, y1),
            (cx - w * 0.2, y1),
            (cx - w * 0.2, y0 + w * 0.5),
            (x0, y0 + w * 0.5),
        ]),
        ShapeKind::DownArrow => poly(&[
            (cx - w * 0.2, y0),
            (cx + w * 0.2, y0),
            (cx + w * 0.2, y1 - w * 0.5),
            (x1, y1 - w * 0.5),
            (cx, y1),
            (x0, y1 - w * 0.5),
            (cx - w * 0.2, y1 - w * 0.5),
        ]),
        ShapeKind::LeftRightArrow => {
            let ah = h * 0.5;
            poly(&[
                (x0, cy),
                (x0 + ah, y0),
                (x0 + ah, cy - h * 0.2),
                (x1 - ah, cy - h * 0.2),
                (x1 - ah, y0),
                (x1, cy),
                (x1 - ah, y1),
                (x1 - ah, cy + h * 0.2),
                (x0 + ah, cy + h * 0.2),
                (x0 + ah, y1),
            ])
        }
        ShapeKind::UpDownArrow => {
            let aw = w * 0.5;
            poly(&[
                (cx, y0),
                (x1, y0 + aw),
                (cx + w * 0.2, y0 + aw),
                (cx + w * 0.2, y1 - aw),
                (x1, y1 - aw),
                (cx, y1),
                (x0, y1 - aw),
                (cx - w * 0.2, y1 - aw),
                (cx - w * 0.2, y0 + aw),
                (x0, y0 + aw),
            ])
        }
        ShapeKind::QuadArrow => {
            let mut p = BezPath::new();
            p.move_to((cx, y0));
            p.line_to((cx + w * 0.15, y0 + h * 0.15));
            p.line_to((cx + w * 0.05, y0 + h * 0.15));
            p.line_to((cx + w * 0.05, cy - h * 0.05));
            p.line_to((x1 - w * 0.15, cy - h * 0.05));
            p.line_to((x1 - w * 0.15, cy - h * 0.15));
            p.line_to((x1, cy));
            p.line_to((x1 - w * 0.15, cy + h * 0.15));
            p.line_to((x1 - w * 0.15, cy + h * 0.05));
            p.line_to((cx + w * 0.05, cy + h * 0.05));
            p.line_to((cx + w * 0.05, y1 - h * 0.15));
            p.line_to((cx + w * 0.15, y1 - h * 0.15));
            p.line_to((cx, y1));
            p.line_to((cx - w * 0.15, y1 - h * 0.15));
            p.line_to((cx - w * 0.05, y1 - h * 0.15));
            p.line_to((cx - w * 0.05, cy + h * 0.05));
            p.line_to((x0 + w * 0.15, cy + h * 0.05));
            p.line_to((x0 + w * 0.15, cy + h * 0.15));
            p.line_to((x0, cy));
            p.line_to((x0 + w * 0.15, cy - h * 0.15));
            p.line_to((x0 + w * 0.15, cy - h * 0.05));
            p.line_to((cx - w * 0.05, cy - h * 0.05));
            p.line_to((cx - w * 0.05, y0 + h * 0.15));
            p.line_to((cx - w * 0.15, y0 + h * 0.15));
            p.close_path();
            p
        }
        ShapeKind::LeftRightUpArrow | ShapeKind::BentArrow | ShapeKind::UTurnArrow | ShapeKind::LeftUpArrow | ShapeKind::BentUpArrow => poly(&[
            (x0, cy),
            (x0 + w * 0.3, y0),
            (x0 + w * 0.3, cy - h * 0.2),
            (x1 - w * 0.2, cy - h * 0.2),
            (x1 - w * 0.2, y1),
            (x1, y1),
            (x1, cy - h * 0.4),
            (x0 + w * 0.3, cy - h * 0.4),
            (x0 + w * 0.3, y0),
        ]),
        ShapeKind::StripedRightArrow => {
            let mut p = poly(&[
                (x0 + w * 0.2, cy - h * 0.2),
                (x1 - h * 0.5, cy - h * 0.2),
                (x1 - h * 0.5, y0),
                (x1, cy),
                (x1 - h * 0.5, y1),
                (x1 - h * 0.5, cy + h * 0.2),
                (x0 + w * 0.2, cy + h * 0.2),
            ]);
            p.move_to((x0, cy - h * 0.2));
            p.line_to((x0 + w * 0.1, cy - h * 0.2));
            p.line_to((x0 + w * 0.1, cy + h * 0.2));
            p.line_to((x0, cy + h * 0.2));
            p.close_path();
            p
        }
        ShapeKind::NotchedRightArrow => poly(&[
            (x0, y0 + h * 0.2),
            (x0 + w * 0.15, cy),
            (x0, y1 - h * 0.2),
            (x1 - h * 0.5, y1 - h * 0.2),
            (x1 - h * 0.5, y1),
            (x1, cy),
            (x1 - h * 0.5, y0),
            (x1 - h * 0.5, y0 + h * 0.2),
        ]),
        ShapeKind::PentagonArrow => {
            poly(&[(x0, y0 + h * 0.15), (x1 - w * 0.3, y0 + h * 0.15), (x1, cy), (x1 - w * 0.3, y1 - h * 0.15), (x0, y1 - h * 0.15)])
        }
        ShapeKind::Chevron => {
            let cut = w * 0.3;
            poly(&[(x0, y0), (x1 - cut, y0), (x1, cy), (x1 - cut, y1), (x0, y1), (x0 + cut, cy)])
        }
        ShapeKind::CircularArrow => {
            let mut p = BezPath::new();
            p.move_to((cx, y0));
            p.quad_to((x1, y0), (x1, cy));
            p.quad_to((x1, y1), (cx, y1));
            p.line_to((cx, y1 - h * 0.2));
            p.quad_to((x1 - w * 0.2, y1 - h * 0.2), (x1 - w * 0.2, cy));
            p.quad_to((x1 - w * 0.2, y0 + h * 0.2), (cx, y0 + h * 0.2));
            p.line_to((cx, y0 + h * 0.35));
            p.line_to((cx - w * 0.2, y0 + h * 0.1));
            p.line_to((cx, y0));
            p.close_path();
            p
        }
        ShapeKind::RightArrowCallout
        | ShapeKind::DownArrowCallout
        | ShapeKind::LeftArrowCallout
        | ShapeKind::UpArrowCallout
        | ShapeKind::LeftRightArrowCallout
        | ShapeKind::QuadArrowCallout => {
            poly(&[(x0, y0), (x1 - w * 0.3, y0), (x1 - w * 0.3, y0 + h * 0.2), (x1, cy), (x1 - w * 0.3, y1 - h * 0.2), (x1 - w * 0.3, y1), (x0, y1)])
        }

        // Equation Shapes
        ShapeKind::MathMinus => {
            let bar_h = h * 0.25;
            kurbo::Rect::new(x0, cy - bar_h / 2.0, x1, cy + bar_h / 2.0).to_path(0.1)
        }
        ShapeKind::MathMultiply => {
            let mut p = BezPath::new();
            let th = w.min(h) * 0.2;
            p.move_to((x0, y0 + th));
            p.line_to((x0 + th, y0));
            p.line_to((x1, y1 - th));
            p.line_to((x1 - th, y1));
            p.close_path();
            p.move_to((x1 - th, y0));
            p.line_to((x1, y0 + th));
            p.line_to((x0 + th, y1));
            p.line_to((x0, y1 - th));
            p.close_path();
            p
        }
        ShapeKind::MathDivide => {
            let bar_h = h * 0.18;
            let mut p = kurbo::Rect::new(x0, cy - bar_h / 2.0, x1, cy + bar_h / 2.0).to_path(0.1);
            let dot_r = w.min(h) * 0.1;
            p.extend(
                kurbo::Ellipse::from_rect(kurbo::Rect::from_center_size(kurbo::Point::new(cx, y0 + h * 0.2), (dot_r * 2.0, dot_r * 2.0)))
                    .to_path(0.1)
                    .elements()
                    .iter()
                    .copied(),
            );
            p.extend(
                kurbo::Ellipse::from_rect(kurbo::Rect::from_center_size(kurbo::Point::new(cx, y1 - h * 0.2), (dot_r * 2.0, dot_r * 2.0)))
                    .to_path(0.1)
                    .elements()
                    .iter()
                    .copied(),
            );
            p
        }
        ShapeKind::MathEqual => {
            let mut p = kurbo::Rect::new(x0, cy - h * 0.25, x1, cy - h * 0.05).to_path(0.1);
            p.extend(kurbo::Rect::new(x0, cy + h * 0.05, x1, cy + h * 0.25).to_path(0.1).elements().iter().copied());
            p
        }
        ShapeKind::MathNotEqual => {
            let mut p = shape_path(ShapeKind::MathEqual, r, corner);
            p.move_to((x0 + w * 0.7, y0));
            p.line_to((x0 + w * 0.3, y1));
            p
        }

        // Flowchart
        ShapeKind::FlowchartPredefinedProcess => {
            let mut p = r.to_path(0.1);
            let off = w * 0.15;
            p.move_to((x0 + off, y0));
            p.line_to((x0 + off, y1));
            p.move_to((x1 - off, y0));
            p.line_to((x1 - off, y1));
            p
        }
        ShapeKind::FlowchartInternalStorage => {
            let mut p = r.to_path(0.1);
            let off = w.min(h) * 0.2;
            p.move_to((x0 + off, y0));
            p.line_to((x0 + off, y1));
            p.move_to((x0, y0 + off));
            p.line_to((x1, y0 + off));
            p
        }
        ShapeKind::FlowchartDocument => {
            let mut p = BezPath::new();
            p.move_to((x0, y0));
            p.line_to((x1, y0));
            p.line_to((x1, y1 - h * 0.15));
            p.quad_to((x0 + w * 0.75, y1), (cx, y1 - h * 0.15));
            p.quad_to((x0 + w * 0.25, y1 - h * 0.3), (x0, y1 - h * 0.15));
            p.close_path();
            p
        }
        ShapeKind::FlowchartMultidocument => {
            let mut p = shape_path(ShapeKind::FlowchartDocument, kurbo::Rect::new(x0 + 4.0, y0 + 4.0, x1, y1), corner);
            p.extend(shape_path(ShapeKind::FlowchartDocument, kurbo::Rect::new(x0, y0, x1 - 4.0, y1 - 4.0), corner).elements().iter().copied());
            p
        }
        ShapeKind::FlowchartTerminator | ShapeKind::FlowchartDelay => {
            let rad = h * 0.5;
            kurbo::RoundedRect::from_rect(r, rad).to_path(0.1)
        }
        ShapeKind::FlowchartManualInput => poly(&[(x0, y0 + h * 0.2), (x1, y0), (x1, y1), (x0, y1)]),
        ShapeKind::FlowchartOffpageConnector => poly(&[(x0, y0), (x1, y0), (x1, y0 + h * 0.65), (cx, y1), (x0, y0 + h * 0.65)]),
        ShapeKind::FlowchartCard => {
            let cut = w.min(h) * 0.25;
            poly(&[(x0 + cut, y0), (x1, y0), (x1, y1), (x0, y1)])
        }
        ShapeKind::FlowchartPunchedTape => {
            let mut p = BezPath::new();
            p.move_to((x0, y0 + h * 0.15));
            p.quad_to((x0 + w * 0.25, y0 + h * 0.3), (cx, y0 + h * 0.15));
            p.quad_to((x0 + w * 0.75, y0), (x1, y0 + h * 0.15));
            p.line_to((x1, y1 - h * 0.15));
            p.quad_to((x0 + w * 0.75, y1), (cx, y1 - h * 0.15));
            p.quad_to((x0 + w * 0.25, y1 - h * 0.3), (x0, y1 - h * 0.15));
            p.close_path();
            p
        }
        ShapeKind::FlowchartSummingJunction => {
            let mut p = kurbo::Ellipse::from_rect(r).to_path(0.1);
            p.move_to((x0 + w * 0.15, y0 + h * 0.15));
            p.line_to((x1 - w * 0.15, y1 - h * 0.15));
            p.move_to((x1 - w * 0.15, y0 + h * 0.15));
            p.line_to((x0 + w * 0.15, y1 - h * 0.15));
            p
        }
        ShapeKind::FlowchartOr => {
            let mut p = kurbo::Ellipse::from_rect(r).to_path(0.1);
            p.move_to((cx, y0));
            p.line_to((cx, y1));
            p.move_to((x0, cy));
            p.line_to((x1, cy));
            p
        }
        ShapeKind::FlowchartCollate | ShapeKind::FlowchartMerge => poly(&[(x0, y0), (x1, y0), (cx, y1)]),
        ShapeKind::FlowchartSort => poly(&[(cx, y0), (x1, cy), (cx, y1), (x0, cy)]),
        ShapeKind::FlowchartStoredData => {
            let mut p = BezPath::new();
            p.move_to((x0 + w * 0.2, y0));
            p.line_to((x1, y0));
            p.quad_to((x1 - w * 0.2, cy), (x1, y1));
            p.line_to((x0 + w * 0.2, y1));
            p.quad_to((x0, cy), (x0 + w * 0.2, y0));
            p.close_path();
            p
        }
        ShapeKind::FlowchartSequentialData => {
            let mut p = kurbo::Ellipse::from_rect(kurbo::Rect::new(x0, y0, x1 - w * 0.2, y1)).to_path(0.1);
            p.move_to((cx, y1));
            p.line_to((x1, y1));
            p
        }
        ShapeKind::FlowchartDirectAccessStorage => {
            let mut p = BezPath::new();
            p.move_to((x0, y0));
            p.line_to((x1 - w * 0.2, y0));
            p.quad_to((x1, cy), (x1 - w * 0.2, y1));
            p.line_to((x0, y1));
            p.quad_to((x0 + w * 0.2, cy), (x0, y0));
            p.close_path();
            p
        }
        ShapeKind::FlowchartDisplay => {
            let mut p = BezPath::new();
            p.move_to((x0 + w * 0.2, y0));
            p.line_to((x1 - w * 0.2, y0));
            p.quad_to((x1, cy), (x1 - w * 0.2, y1));
            p.line_to((x0 + w * 0.2, y1));
            p.line_to((x0, cy));
            p.close_path();
            p
        }

        // Stars & Banners
        ShapeKind::Explosion1 => star(8, 0.5),
        ShapeKind::Explosion2 => star(14, 0.6),
        ShapeKind::Star4 => star(4, 0.35),
        ShapeKind::Star => star(5, 0.4),
        ShapeKind::Star6 => star(6, 0.5),
        ShapeKind::Star7 => star(7, 0.55),
        ShapeKind::Star8 => star(8, 0.55),
        ShapeKind::Star10 => star(10, 0.6),
        ShapeKind::Star12 => star(12, 0.65),
        ShapeKind::Star16 => star(16, 0.7),
        ShapeKind::Star24 => star(24, 0.75),
        ShapeKind::Star32 => star(32, 0.8),
        ShapeKind::UpRibbon => poly(&[
            (x0, y0 + h * 0.3),
            (x0 + w * 0.2, y0),
            (x1 - w * 0.2, y0),
            (x1, y0 + h * 0.3),
            (x1 - w * 0.1, cy),
            (x1, y1),
            (x1 - w * 0.2, y1 - h * 0.3),
            (x0 + w * 0.2, y1 - h * 0.3),
            (x0, y1),
            (x0 + w * 0.1, cy),
        ]),
        ShapeKind::DownRibbon => poly(&[
            (x0, y0),
            (x0 + w * 0.2, y0 + h * 0.3),
            (x1 - w * 0.2, y0 + h * 0.3),
            (x1, y0),
            (x1 - w * 0.1, cy),
            (x1, y1 - h * 0.3),
            (x1 - w * 0.2, y1),
            (x0 + w * 0.2, y1),
            (x0, y1 - h * 0.3),
            (x0 + w * 0.1, cy),
        ]),
        ShapeKind::CurvedUpRibbon | ShapeKind::CurvedDownRibbon => {
            let mut p = BezPath::new();
            p.move_to((x0, y1 - h * 0.2));
            p.quad_to((cx, y0), (x1, y1 - h * 0.2));
            p.line_to((x1, y1));
            p.quad_to((cx, y0 + h * 0.2), (x0, y1));
            p.close_path();
            p
        }
        ShapeKind::VerticalScroll => {
            let mut p = r.to_path(0.1);
            p.move_to((x0, y0 + h * 0.15));
            p.line_to((x1, y0 + h * 0.15));
            p.move_to((x0, y1 - h * 0.15));
            p.line_to((x1, y1 - h * 0.15));
            p
        }
        ShapeKind::HorizontalScroll => {
            let mut p = r.to_path(0.1);
            p.move_to((x0 + w * 0.15, y0));
            p.line_to((x0 + w * 0.15, y1));
            p.move_to((x1 - w * 0.15, y0));
            p.line_to((x1 - w * 0.15, y1));
            p
        }
        ShapeKind::Wave => {
            let mut p = BezPath::new();
            p.move_to((x0, y0 + h * 0.2));
            p.quad_to((x0 + w * 0.25, y0 + h * 0.4), (cx, y0 + h * 0.2));
            p.quad_to((x0 + w * 0.75, y0), (x1, y0 + h * 0.2));
            p.line_to((x1, y1));
            p.quad_to((x0 + w * 0.75, y1 - h * 0.2), (cx, y1));
            p.quad_to((x0 + w * 0.25, y1 + h * 0.2), (x0, y1));
            p.close_path();
            p
        }
        ShapeKind::DoubleWave => {
            let mut p = shape_path(ShapeKind::Wave, r, corner);
            p.move_to((x0, cy));
            p.line_to((x1, cy));
            p
        }

        // Callouts
        ShapeKind::RectangularCallout => poly(&[
            (x0, y0),
            (x1, y0),
            (x1, y1 - h * 0.25),
            (x0 + w * 0.4, y1 - h * 0.25),
            (x0 + w * 0.2, y1),
            (x0 + w * 0.25, y1 - h * 0.25),
            (x0, y1 - h * 0.25),
        ]),
        ShapeKind::RoundedRectangularCallout => {
            let mut p = kurbo::RoundedRect::from_rect(kurbo::Rect::new(x0, y0, x1, y1 - h * 0.25), 6.0).to_path(0.1);
            p.move_to((x0 + w * 0.4, y1 - h * 0.25));
            p.line_to((x0 + w * 0.2, y1));
            p.line_to((x0 + w * 0.25, y1 - h * 0.25));
            p
        }
        ShapeKind::OvalCallout => {
            let mut p = kurbo::Ellipse::from_rect(kurbo::Rect::new(x0, y0, x1, y1 - h * 0.25)).to_path(0.1);
            p.move_to((x0 + w * 0.4, y1 - h * 0.25));
            p.line_to((x0 + w * 0.2, y1));
            p.line_to((x0 + w * 0.3, y1 - h * 0.25));
            p
        }
        ShapeKind::LineCallout1
        | ShapeKind::LineCallout2
        | ShapeKind::LineCallout3
        | ShapeKind::LineCallout4
        | ShapeKind::LineCallout1AccentBar
        | ShapeKind::LineCallout2AccentBar
        | ShapeKind::LineCallout3AccentBar
        | ShapeKind::LineCallout4AccentBar => {
            let mut p = BezPath::new();
            p.move_to((x0, y1));
            p.line_to((x0 + w * 0.3, cy));
            p.line_to((x1, cy));
            if matches!(
                kind,
                ShapeKind::LineCallout1AccentBar
                    | ShapeKind::LineCallout2AccentBar
                    | ShapeKind::LineCallout3AccentBar
                    | ShapeKind::LineCallout4AccentBar
            ) {
                p.move_to((x0 + w * 0.3, y0));
                p.line_to((x0 + w * 0.3, y1));
            }
            p
        }
    }
}

/// Flattened subpath contours of a shape inside a `(0, 0, w, h)` box, suitable for UI preview rendering.
/// Returns a list of `(closed, points)` for each contour.
pub fn shape_preview_contours(kind: ShapeKind, w: f32, h: f32) -> Vec<(bool, Vec<(f32, f32)>)> {
    let r = kurbo::Rect::new(0.0, 0.0, w as f64, h as f64);
    let path = shape_path(kind, r, None);
    let mut contours = Vec::new();
    let mut current = Vec::new();
    let mut is_closed = false;
    kurbo::flatten(path, 0.5, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if !current.is_empty() {
                contours.push((is_closed, std::mem::take(&mut current)));
                is_closed = false;
            }
            current.push((p.x as f32, p.y as f32));
        }
        kurbo::PathEl::LineTo(p) => {
            current.push((p.x as f32, p.y as f32));
        }
        kurbo::PathEl::ClosePath => {
            is_closed = true;
            if !current.is_empty() {
                contours.push((true, std::mem::take(&mut current)));
                is_closed = false;
            }
        }
        _ => {}
    });
    if !current.is_empty() {
        contours.push((is_closed, current));
    }
    contours
}
