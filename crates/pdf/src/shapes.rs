//! Path geometry for Office shapes exported to PDF (krilla).

use krilla::geom::{Path, PathBuilder};
use wordcraft_doc::para::ShapeKind;
use wordcraft_geom::Rect;

fn ok(v: f32) -> bool {
    v.is_finite()
}

pub(crate) fn shape_path(kind: ShapeKind, r: &Rect, corner: Option<u32>) -> Option<Path> {
    let flat_ok = (kind.is_open() || kind.is_line_like()) && r.w.max(r.h) > 0.0 && r.w.min(r.h) >= 0.0;
    if !(ok(r.x) && ok(r.y) && ok(r.w) && ok(r.h)) || !(flat_ok || r.w > 0.0 && r.h > 0.0) {
        return None;
    }
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (cx, cy) = (x0 + r.w / 2.0, y0 + r.h / 2.0);
    let mut pb = PathBuilder::new();
    let poly = |pb: &mut PathBuilder, pts: &[(f32, f32)]| {
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y);
            } else {
                pb.line_to(*x, *y);
            }
        }
        pb.close();
    };
    const K: f32 = 0.552_284_8;
    let ellipse = |pb: &mut PathBuilder, cx: f32, cy: f32, rx: f32, ry: f32| {
        pb.move_to(cx + rx, cy);
        pb.cubic_to(cx + rx, cy + ry * K, cx + rx * K, cy + ry, cx, cy + ry);
        pb.cubic_to(cx - rx * K, cy + ry, cx - rx, cy + ry * K, cx - rx, cy);
        pb.cubic_to(cx - rx, cy - ry * K, cx - rx * K, cy - ry, cx, cy - ry);
        pb.cubic_to(cx + rx * K, cy - ry, cx + rx, cy - ry * K, cx + rx, cy);
        pb.close();
    };

    let (w, h) = (r.w, r.h);
    let ngon = |pb: &mut PathBuilder, sides: usize, rot: f32| {
        let mut pts = Vec::with_capacity(sides);
        let rx = w / 2.0;
        let ry = h / 2.0;
        for i in 0..sides {
            let a = rot + 2.0 * std::f32::consts::PI * (i as f32) / (sides as f32);
            pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
        }
        poly(pb, &pts);
    };

    let star = |pb: &mut PathBuilder, pts_count: usize, inner_ratio: f32| {
        let mut pts = Vec::with_capacity(pts_count * 2);
        let rx = w / 2.0;
        let ry = h / 2.0;
        for i in 0..(pts_count * 2) {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * (i as f32) / (pts_count as f32);
            let rad = if i % 2 == 0 { 1.0 } else { inner_ratio };
            pts.push((cx + rx * rad * a.cos(), cy + ry * rad * a.sin()));
        }
        poly(pb, &pts);
    };

    match kind {
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::DrawingCanvas | ShapeKind::FlowchartProcess => {
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
        }
        ShapeKind::RoundedRectangle => {
            let rad = wordcraft_doc::connector::corner_radius(corner, r.w, r.h);
            let k = rad * (1.0 - K);
            pb.move_to(x0 + rad, y0);
            pb.line_to(x1 - rad, y0);
            pb.cubic_to(x1 - k, y0, x1, y0 + k, x1, y0 + rad);
            pb.line_to(x1, y1 - rad);
            pb.cubic_to(x1, y1 - k, x1 - k, y1, x1 - rad, y1);
            pb.line_to(x0 + rad, y1);
            pb.cubic_to(x0 + k, y1, x0, y1 - k, x0, y1 - rad);
            pb.line_to(x0, y0 + rad);
            pb.cubic_to(x0, y0 + k, x0 + k, y0, x0 + rad, y0);
            pb.close();
        }
        ShapeKind::FlowchartAlternateProcess => {
            let rad = r.w.min(r.h) * 0.16;
            let k = rad * (1.0 - K);
            pb.move_to(x0 + rad, y0);
            pb.line_to(x1 - rad, y0);
            pb.cubic_to(x1 - k, y0, x1, y0 + k, x1, y0 + rad);
            pb.line_to(x1, y1 - rad);
            pb.cubic_to(x1, y1 - k, x1 - k, y1, x1 - rad, y1);
            pb.line_to(x0 + rad, y1);
            pb.cubic_to(x0 + k, y1, x0, y1 - k, x0, y1 - rad);
            pb.line_to(x0, y0 + rad);
            pb.cubic_to(x0, y0 + k, x0 + k, y0, x0 + rad, y0);
            pb.close();
        }
        ShapeKind::SingleCornerSnipped => {
            let cut = w.min(h) * 0.2;
            poly(&mut pb, &[(x0 + cut, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::TopCornersSnipped => {
            let cut = w.min(h) * 0.2;
            poly(&mut pb, &[(x0 + cut, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::DiagonalCornersSnipped => {
            let cut = w.min(h) * 0.2;
            poly(&mut pb, &[(x0 + cut, y0), (x1, y0), (x1, y1 - cut), (x1 - cut, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::SnippedAndRounded => {
            let cut = w.min(h) * 0.2;
            poly(&mut pb, &[(x0 + cut, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1), (x0, y1)])
        }
        ShapeKind::RoundSingleCorner | ShapeKind::RoundSameSideCorner | ShapeKind::RoundDiagonalCorner => {
            let cut = w.min(h) * 0.2;
            poly(&mut pb, &[(x0 + cut, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0 + cut)])
        }
        ShapeKind::Ellipse | ShapeKind::FlowchartConnector => ellipse(&mut pb, cx, cy, r.w / 2.0, r.h / 2.0),
        ShapeKind::Triangle | ShapeKind::FlowchartExtract => poly(&mut pb, &[(cx, y0), (x1, y1), (x0, y1)]),
        ShapeKind::RightTriangle => poly(&mut pb, &[(x0, y0), (x1, y1), (x0, y1)]),
        ShapeKind::Parallelogram | ShapeKind::FlowchartData => {
            let off = w * 0.2;
            poly(&mut pb, &[(x0 + off, y0), (x1, y0), (x1 - off, y1), (x0, y1)])
        }
        ShapeKind::Trapezoid | ShapeKind::FlowchartManualOperation => {
            let off = w * 0.2;
            poly(&mut pb, &[(x0 + off, y0), (x1 - off, y0), (x1, y1), (x0, y1)])
        }
        ShapeKind::Diamond | ShapeKind::FlowchartDecision => poly(&mut pb, &[(cx, y0), (x1, cy), (cx, y1), (x0, cy)]),
        ShapeKind::Pentagon => ngon(&mut pb, 5, -std::f32::consts::FRAC_PI_2),
        ShapeKind::Hexagon | ShapeKind::FlowchartPreparation => ngon(&mut pb, 6, 0.0),
        ShapeKind::Heptagon => ngon(&mut pb, 7, -std::f32::consts::FRAC_PI_2),
        ShapeKind::Octagon => ngon(&mut pb, 8, std::f32::consts::PI / 8.0),
        ShapeKind::Decagon => ngon(&mut pb, 10, -std::f32::consts::FRAC_PI_2),
        ShapeKind::Dodecagon => ngon(&mut pb, 12, 0.0),

        ShapeKind::Pie | ShapeKind::Chord => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
        }
        ShapeKind::Teardrop => {
            pb.move_to(cx, y0);
            pb.cubic_to(x1, y0, x1, y1, cx, y1);
            pb.cubic_to(x0, y1, x0, y0, cx, y0);
            pb.close();
        }
        ShapeKind::Frame | ShapeKind::Bevel => {
            let th = w.min(h) * 0.15;
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
            poly(&mut pb, &[(x0 + th, y0 + th), (x1 - th, y0 + th), (x1 - th, y1 - th), (x0 + th, y1 - th)]);
        }
        ShapeKind::HalfFrame => {
            let th = w.min(h) * 0.15;
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y0 + th), (x0 + th, y0 + th), (x0 + th, y1), (x0, y1)]);
        }
        ShapeKind::LShape => {
            let th = w.min(h) * 0.25;
            poly(&mut pb, &[(x0, y0), (x0 + th, y0), (x0 + th, y1 - th), (x1, y1 - th), (x1, y1), (x0, y1)]);
        }
        ShapeKind::DiagonalStripe => {
            let th = w.min(h) * 0.3;
            poly(&mut pb, &[(x0 + th, y0), (x1, y1 - th), (x1, y1), (x1 - th, y1), (x0, y0 + th), (x0, y0)]);
        }
        ShapeKind::Cross | ShapeKind::MathPlus => {
            let xw = w * 0.3;
            let yh = h * 0.3;
            poly(
                &mut pb,
                &[
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
                ],
            );
        }
        ShapeKind::Plaque => {
            let cut = w.min(h) * 0.2;
            poly(
                &mut pb,
                &[(x0 + cut, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1 - cut), (x1 - cut, y1), (x0 + cut, y1), (x0, y1 - cut), (x0, y0 + cut)],
            );
        }
        ShapeKind::Cylinder | ShapeKind::FlowchartMagneticDisk => {
            let cap = h * 0.2;
            pb.move_to(x0, y0 + cap / 2.0);
            pb.line_to(x1, y0 + cap / 2.0);
            pb.line_to(x1, y1 - cap / 2.0);
            pb.cubic_to(x1, y1 + cap / 2.0, x0, y1 + cap / 2.0, x0, y1 - cap / 2.0);
            pb.close();
        }
        ShapeKind::Cube => {
            let off = w.min(h) * 0.25;
            poly(&mut pb, &[(x0, y0 + off), (x1 - off, y0 + off), (x1 - off, y1), (x0, y1)]);
        }
        ShapeKind::Donut | ShapeKind::NoSymbol => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
            ellipse(&mut pb, cx, cy, w * 0.3, h * 0.3);
        }
        ShapeKind::BlockArc => {
            pb.move_to(x0, cy);
            pb.cubic_to(x0, y0, cx, y0, cx, y0);
            pb.line_to(cx, y0 + h * 0.2);
            pb.cubic_to(x0 + w * 0.2, y0 + h * 0.2, x0 + w * 0.2, cy, x0 + w * 0.2, cy);
            pb.close();
        }
        ShapeKind::FoldedCorner => {
            let cut = w.min(h) * 0.25;
            poly(&mut pb, &[(x0, y0), (x1 - cut, y0), (x1, y0 + cut), (x1, y1), (x0, y1)]);
        }
        ShapeKind::SmileyFace => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
        }
        ShapeKind::Heart => {
            pb.move_to(cx, y0 + h * 0.3);
            pb.cubic_to(cx, y0, x0, y0, x0, y0 + h * 0.3);
            pb.cubic_to(x0, y0 + h * 0.6, cx - w * 0.1, y0 + h * 0.75, cx, y1);
            pb.cubic_to(cx + w * 0.1, y0 + h * 0.75, x1, y0 + h * 0.6, x1, y0 + h * 0.3);
            pb.cubic_to(x1, y0, cx, y0, cx, y0 + h * 0.3);
            pb.close();
        }
        ShapeKind::LightningBolt => poly(
            &mut pb,
            &[
                (cx + w * 0.1, y0),
                (x0 + w * 0.1, cy + h * 0.05),
                (cx, cy + h * 0.05),
                (cx - w * 0.1, y1),
                (x1 - w * 0.1, cy - h * 0.05),
                (cx, cy - h * 0.05),
            ],
        ),
        ShapeKind::Sun => star(&mut pb, 12, 0.75),
        ShapeKind::Moon => {
            pb.move_to(cx, y0);
            pb.cubic_to(x1, cy, cx, y1, cx, y1);
            pb.cubic_to(cx + w * 0.15, cy, cx, y0, cx, y0);
            pb.close();
        }
        ShapeKind::Cloud | ShapeKind::CloudCallout => {
            ellipse(&mut pb, cx, cy, w * 0.4, h * 0.3);
        }

        // Lines
        ShapeKind::Line | ShapeKind::StraightConnector => {
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
        }
        ShapeKind::Arrow => {
            poly(
                &mut pb,
                &[
                    (x0, cy - h * 0.2),
                    (x1 - h * 0.5, cy - h * 0.2),
                    (x1 - h * 0.5, y0),
                    (x1, cy),
                    (x1 - h * 0.5, y1),
                    (x1 - h * 0.5, cy + h * 0.2),
                    (x0, cy + h * 0.2),
                ],
            );
        }
        ShapeKind::DoubleArrow => {
            let ah = h * 0.4;
            poly(
                &mut pb,
                &[
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
                ],
            );
        }
        ShapeKind::ElbowConnector | ShapeKind::ElbowArrow | ShapeKind::ElbowDoubleArrow => {
            pb.move_to(x0, y0);
            pb.line_to(cx, y0);
            pb.line_to(cx, y1);
            pb.line_to(x1, y1);
        }
        ShapeKind::CurvedConnector | ShapeKind::CurvedArrow | ShapeKind::CurvedDoubleArrow | ShapeKind::Curve | ShapeKind::Arc => {
            pb.move_to(x0, y1);
            pb.cubic_to(x0, y0, x1, y0, x1, y0);
        }
        ShapeKind::Freeform | ShapeKind::Scribble => {
            pb.move_to(x0, cy);
            pb.cubic_to(x0 + w * 0.25, y0, cx, cy, cx, cy);
            pb.cubic_to(x0 + w * 0.75, y1, x1, cy, x1, cy);
        }
        ShapeKind::DoubleBracket | ShapeKind::LeftBracket | ShapeKind::RightBracket => {
            pb.move_to(x0 + w * 0.3, y0);
            pb.line_to(x0, y0);
            pb.line_to(x0, y1);
            pb.line_to(x0 + w * 0.3, y1);
            if kind != ShapeKind::LeftBracket {
                pb.move_to(x1 - w * 0.3, y0);
                pb.line_to(x1, y0);
                pb.line_to(x1, y1);
                pb.line_to(x1 - w * 0.3, y1);
            }
        }
        ShapeKind::DoubleBrace | ShapeKind::LeftBrace | ShapeKind::RightBrace => {
            pb.move_to(x0 + w * 0.3, y0);
            pb.line_to(x0 + w * 0.15, cy);
            pb.line_to(x0 + w * 0.3, y1);
            if kind != ShapeKind::LeftBrace {
                pb.move_to(x1 - w * 0.3, y0);
                pb.line_to(x1 - w * 0.15, cy);
                pb.line_to(x1 - w * 0.3, y1);
            }
        }

        // Block arrows
        ShapeKind::LeftArrow => poly(
            &mut pb,
            &[
                (x0, cy),
                (x0 + h * 0.5, y0),
                (x0 + h * 0.5, cy - h * 0.2),
                (x1, cy - h * 0.2),
                (x1, cy + h * 0.2),
                (x0 + h * 0.5, cy + h * 0.2),
                (x0 + h * 0.5, y1),
            ],
        ),
        ShapeKind::UpArrow => poly(
            &mut pb,
            &[
                (cx, y0),
                (x1, y0 + w * 0.5),
                (cx + w * 0.2, y0 + w * 0.5),
                (cx + w * 0.2, y1),
                (cx - w * 0.2, y1),
                (cx - w * 0.2, y0 + w * 0.5),
                (x0, y0 + w * 0.5),
            ],
        ),
        ShapeKind::DownArrow => poly(
            &mut pb,
            &[
                (cx - w * 0.2, y0),
                (cx + w * 0.2, y0),
                (cx + w * 0.2, y1 - w * 0.5),
                (x1, y1 - w * 0.5),
                (cx, y1),
                (x0, y1 - w * 0.5),
                (cx - w * 0.2, y1 - w * 0.5),
            ],
        ),
        ShapeKind::LeftRightArrow => {
            let ah = h * 0.5;
            poly(
                &mut pb,
                &[
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
                ],
            );
        }
        ShapeKind::UpDownArrow => {
            let aw = w * 0.5;
            poly(
                &mut pb,
                &[
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
                ],
            );
        }
        ShapeKind::QuadArrow => {
            poly(
                &mut pb,
                &[
                    (cx, y0),
                    (cx + w * 0.15, y0 + h * 0.15),
                    (cx + w * 0.05, y0 + h * 0.15),
                    (cx + w * 0.05, cy - h * 0.05),
                    (x1 - w * 0.15, cy - h * 0.05),
                    (x1, cy),
                    (x1 - w * 0.15, cy + h * 0.05),
                    (cx + w * 0.05, cy + h * 0.05),
                    (cx + w * 0.05, y1 - h * 0.15),
                    (cx, y1),
                    (cx - w * 0.05, y1 - h * 0.15),
                    (cx - w * 0.05, cy + h * 0.05),
                    (x0 + w * 0.15, cy + h * 0.05),
                    (x0, cy),
                    (x0 + w * 0.15, cy - h * 0.05),
                    (cx - w * 0.05, cy - h * 0.05),
                    (cx - w * 0.05, y0 + h * 0.15),
                ],
            );
        }
        ShapeKind::LeftRightUpArrow | ShapeKind::BentArrow | ShapeKind::UTurnArrow | ShapeKind::LeftUpArrow | ShapeKind::BentUpArrow => {
            poly(
                &mut pb,
                &[
                    (x0, cy),
                    (x0 + w * 0.3, y0),
                    (x0 + w * 0.3, cy - h * 0.2),
                    (x1 - w * 0.2, cy - h * 0.2),
                    (x1 - w * 0.2, y1),
                    (x1, y1),
                    (x1, cy - h * 0.4),
                    (x0 + w * 0.3, cy - h * 0.4),
                    (x0 + w * 0.3, y0),
                ],
            );
        }
        ShapeKind::StripedRightArrow | ShapeKind::NotchedRightArrow => {
            poly(
                &mut pb,
                &[
                    (x0, y0 + h * 0.2),
                    (x0 + w * 0.15, cy),
                    (x0, y1 - h * 0.2),
                    (x1 - h * 0.5, y1 - h * 0.2),
                    (x1 - h * 0.5, y1),
                    (x1, cy),
                    (x1 - h * 0.5, y0),
                    (x1 - h * 0.5, y0 + h * 0.2),
                ],
            );
        }
        ShapeKind::PentagonArrow => {
            poly(&mut pb, &[(x0, y0 + h * 0.15), (x1 - w * 0.3, y0 + h * 0.15), (x1, cy), (x1 - w * 0.3, y1 - h * 0.15), (x0, y1 - h * 0.15)])
        }
        ShapeKind::Chevron => {
            let cut = w * 0.3;
            poly(&mut pb, &[(x0, y0), (x1 - cut, y0), (x1, cy), (x1 - cut, y1), (x0, y1), (x0 + cut, cy)]);
        }
        ShapeKind::CircularArrow => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
        }
        ShapeKind::RightArrowCallout
        | ShapeKind::DownArrowCallout
        | ShapeKind::LeftArrowCallout
        | ShapeKind::UpArrowCallout
        | ShapeKind::LeftRightArrowCallout
        | ShapeKind::QuadArrowCallout => {
            poly(
                &mut pb,
                &[(x0, y0), (x1 - w * 0.3, y0), (x1 - w * 0.3, y0 + h * 0.2), (x1, cy), (x1 - w * 0.3, y1 - h * 0.2), (x1 - w * 0.3, y1), (x0, y1)],
            );
        }

        // Equation Shapes
        ShapeKind::MathMinus => {
            let bar_h = h * 0.25;
            poly(&mut pb, &[(x0, cy - bar_h / 2.0), (x1, cy - bar_h / 2.0), (x1, cy + bar_h / 2.0), (x0, cy + bar_h / 2.0)]);
        }
        ShapeKind::MathMultiply => {
            let th = w.min(h) * 0.2;
            poly(&mut pb, &[(x0, y0 + th), (x0 + th, y0), (x1, y1 - th), (x1 - th, y1)]);
        }
        ShapeKind::MathDivide => {
            let bar_h = h * 0.18;
            poly(&mut pb, &[(x0, cy - bar_h / 2.0), (x1, cy - bar_h / 2.0), (x1, cy + bar_h / 2.0), (x0, cy + bar_h / 2.0)]);
        }
        ShapeKind::MathEqual | ShapeKind::MathNotEqual => {
            poly(&mut pb, &[(x0, cy - h * 0.25), (x1, cy - h * 0.25), (x1, cy - h * 0.05), (x0, cy - h * 0.05)]);
            poly(&mut pb, &[(x0, cy + h * 0.05), (x1, cy + h * 0.05), (x1, cy + h * 0.25), (x0, cy + h * 0.25)]);
        }

        // Flowchart
        ShapeKind::FlowchartPredefinedProcess | ShapeKind::FlowchartInternalStorage => {
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
        }
        ShapeKind::FlowchartDocument | ShapeKind::FlowchartMultidocument => {
            pb.move_to(x0, y0);
            pb.line_to(x1, y0);
            pb.line_to(x1, y1 - h * 0.15);
            pb.cubic_to(x0 + w * 0.75, y1, x0 + w * 0.25, y1 - h * 0.3, x0, y1 - h * 0.15);
            pb.close();
        }
        ShapeKind::FlowchartTerminator | ShapeKind::FlowchartDelay => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
        }
        ShapeKind::FlowchartManualInput => poly(&mut pb, &[(x0, y0 + h * 0.2), (x1, y0), (x1, y1), (x0, y1)]),
        ShapeKind::FlowchartOffpageConnector => poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y0 + h * 0.65), (cx, y1), (x0, y0 + h * 0.65)]),
        ShapeKind::FlowchartCard => {
            let cut = w.min(h) * 0.25;
            poly(&mut pb, &[(x0 + cut, y0), (x1, y0), (x1, y1), (x0, y1)]);
        }
        ShapeKind::FlowchartPunchedTape => {
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
        }
        ShapeKind::FlowchartSummingJunction => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
            pb.move_to(x0 + w * 0.15, y0 + h * 0.15);
            pb.line_to(x1 - w * 0.15, y1 - h * 0.15);
            pb.move_to(x1 - w * 0.15, y0 + h * 0.15);
            pb.line_to(x0 + w * 0.15, y1 - h * 0.15);
        }
        ShapeKind::FlowchartOr => {
            ellipse(&mut pb, cx, cy, w / 2.0, h / 2.0);
            pb.move_to(cx, y0);
            pb.line_to(cx, y1);
            pb.move_to(x0, cy);
            pb.line_to(x1, cy);
        }
        ShapeKind::FlowchartCollate | ShapeKind::FlowchartMerge => poly(&mut pb, &[(x0, y0), (x1, y0), (cx, y1)]),
        ShapeKind::FlowchartSort => poly(&mut pb, &[(cx, y0), (x1, cy), (cx, y1), (x0, cy)]),
        ShapeKind::FlowchartStoredData
        | ShapeKind::FlowchartSequentialData
        | ShapeKind::FlowchartDirectAccessStorage
        | ShapeKind::FlowchartDisplay => {
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
        }

        // Stars & Banners
        ShapeKind::Explosion1 => star(&mut pb, 8, 0.5),
        ShapeKind::Explosion2 => star(&mut pb, 14, 0.6),
        ShapeKind::Star4 => star(&mut pb, 4, 0.35),
        ShapeKind::Star => star(&mut pb, 5, 0.4),
        ShapeKind::Star6 => star(&mut pb, 6, 0.5),
        ShapeKind::Star7 => star(&mut pb, 7, 0.55),
        ShapeKind::Star8 => star(&mut pb, 8, 0.55),
        ShapeKind::Star10 => star(&mut pb, 10, 0.6),
        ShapeKind::Star12 => star(&mut pb, 12, 0.65),
        ShapeKind::Star16 => star(&mut pb, 16, 0.7),
        ShapeKind::Star24 => star(&mut pb, 24, 0.75),
        ShapeKind::Star32 => star(&mut pb, 32, 0.8),
        ShapeKind::UpRibbon | ShapeKind::DownRibbon => poly(
            &mut pb,
            &[
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
            ],
        ),
        ShapeKind::CurvedUpRibbon
        | ShapeKind::CurvedDownRibbon
        | ShapeKind::VerticalScroll
        | ShapeKind::HorizontalScroll
        | ShapeKind::Wave
        | ShapeKind::DoubleWave => {
            poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
        }

        // Callouts
        ShapeKind::RectangularCallout | ShapeKind::RoundedRectangularCallout | ShapeKind::OvalCallout => poly(
            &mut pb,
            &[
                (x0, y0),
                (x1, y0),
                (x1, y1 - h * 0.25),
                (x0 + w * 0.4, y1 - h * 0.25),
                (x0 + w * 0.2, y1),
                (x0 + w * 0.25, y1 - h * 0.25),
                (x0, y1 - h * 0.25),
            ],
        ),
        ShapeKind::LineCallout1
        | ShapeKind::LineCallout2
        | ShapeKind::LineCallout3
        | ShapeKind::LineCallout4
        | ShapeKind::LineCallout1AccentBar
        | ShapeKind::LineCallout2AccentBar
        | ShapeKind::LineCallout3AccentBar
        | ShapeKind::LineCallout4AccentBar => {
            pb.move_to(x0, y1);
            pb.line_to(x0 + w * 0.3, cy);
            pb.line_to(x1, cy);
        }
    }
    pb.finish()
}
