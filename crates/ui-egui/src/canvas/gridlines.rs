//! The gridlines of a page: the drawing grid over the text area (View › Gridlines) and the
//! outlines of table cells that have no printed border (View Gridlines for tables).

use craft::behavior::kurbo::{Point, Rect as DocRect, Vec2};
use craft::behavior::{Input, Metrics};
use craft::pages::ui::PageBody;
use craft::ui::canvas_controls::{CanvasGrid, GridCrowding};
use craft::ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Response, Ui};

use super::story::{editing_header_footer, text_box_host};
use super::{DocLayout, Page, Placed, StoryRef, WordApp};

/// Where a page's drawing grid starts and runs, and its major step per axis, all in the page's own
/// points.
pub(in crate::canvas) struct PageGrid {
    /// The text area's top left: the grid counts from the margins.
    pub origin: Point,
    /// The text area.
    pub bounds: DocRect,
    /// The step across and down; `f64::INFINITY` across when the section has no character grid.
    pub spacing: Vec2,
}

/// A step that is not a positive finite number falls back to the spec's 1/8 inch.
fn step(spacing: f32) -> f64 {
    if spacing.is_finite() && spacing > 0.0 { f64::from(spacing) } else { f64::from(wordcraft_doc::DEFAULT_GRID) }
}

/// The drawing grid of `page`, when View › Gridlines is on in Print Layout. A section with a
/// document grid (`w:docGrid`) shows lines at its line pitch and, for a character grid, columns at
/// its character pitch; without one the document's own spacing applies both ways.
pub(in crate::canvas) fn page_grid(app: &WordApp, page: &Page) -> Option<PageGrid> {
    if !(app.session.view.gridlines && app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode) {
        return None;
    }
    let settings = &app.session.doc.settings;
    let sections = app.session.doc.sections();
    let sect = sections.get(page.section).map(|(_, s)| *s);
    let (across, down) = match sect.and_then(|s| s.grid_line_pitch().map(|lp| (s, lp))) {
        Some((s, lp)) => {
            let size = app.session.doc.styles.resolve_char(Some("Normal"), &Default::default()).size;
            (s.grid_char_pitch(size).map_or(f64::INFINITY, step), step(lp))
        }
        None => (step(settings.grid_h), step(settings.grid_v)),
    };
    let body = page.body;
    let (x0, y0) = (f64::from(body.x), f64::from(body.y));
    let (x1, y1) = (f64::from(body.right()), f64::from(body.bottom()));
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) || x1 < x0 || y1 < y0 {
        return None;
    }
    Some(PageGrid { origin: Point::new(x0, y0), bounds: DocRect::new(x0, y0, x1, y1), spacing: Vec2::new(across, down) })
}

/// The colour of the table gridlines.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(in crate::canvas) struct TableGridlinesStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: StyleColor,
}

/// The table cells' outlines of the visible pages, stroked where no printed border draws them. Display only.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub(in crate::canvas) struct TableGridlines<'a> {
    #[common]
    common: Common,
    /// The segments in screen points.
    lines: &'a [[Pos2; 2]],
}

impl<'a> TableGridlines<'a> {
    /// The gridlines named `label` over `lines`, screen segments.
    pub fn new(label: impl Into<Label>, lines: &'a [[Pos2; 2]]) -> Self {
        TableGridlines { common: Common::new(), lines }.label(label)
    }
}

impl Component for TableGridlines<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = TableGridlinesStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TableGridlinesStyleKey) -> TableGridlinesStyle {
        TableGridlinesStyle { metrics: Metrics::new(), line: cx.color(cx.tokens.color.overlay.grid) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for line in self.lines {
            draw.stroke(&PaintShape::Path(line.to_vec()), StrokeWidth::Thin, cx.style.line);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Draws the drawing grid of every visible page, thinned out when zoomed far out, and the table
/// gridlines.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody<'_>) {
    let all = app.session.view.table_gridlines;
    let story = text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    let editing = editing_header_footer(app, layout).then_some(story);
    let mut lines: Vec<[Pos2; 2]> = Vec::new();
    for placed in body.pages {
        let Some(page) = layout.pages.get(placed.page) else { continue };
        if let Some(grid) = page_grid(app, page) {
            let to_doc = |p: Point| placed.to_doc(&body.view, p);
            let origin = to_doc(grid.origin);
            let (a, b) = (to_doc(Point::new(grid.bounds.x0, grid.bounds.y0)), to_doc(Point::new(grid.bounds.x1, grid.bounds.y1)));
            // `CanvasGrid` counts its step in document units: a page length goes through the same map as the points.
            let step = |along: f64, unit: Point| {
                if along.is_finite() {
                    let (z, s) = (to_doc(Point::ZERO), to_doc(Point::new(unit.x * along, unit.y * along)));
                    (s.x - z.x).hypot(s.y - z.y)
                } else {
                    f64::INFINITY
                }
            };
            let spacing = Vec2::new(step(grid.spacing.x, Point::new(1.0, 0.0)), step(grid.spacing.y, Point::new(0.0, 1.0)));
            CanvasGrid::new(tl!("Grid").to_string())
                .spacing(spacing)
                .origin(origin)
                .bounds(DocRect::new(a.x, a.y, b.x, b.y))
                .crowding(GridCrowding::Thin)
                .show_at(ui, body.rect);
        }
        if all || editing.is_some() {
            let at = |p: &wordcraft_geom::Point| placed.to_screen(Point::new(f64::from(p.x), f64::from(p.y)));
            lines.extend(gridline_segments(page, all, editing).iter().map(|(a, b)| [at(a), at(b)]));
        }
    }
    if !lines.is_empty() {
        TableGridlines::new(tl!("Table gridlines").to_string(), &lines).show_at(ui, body.rect);
    }
}

/// Global View Gridlines outlines every table (#1628). While editing a header/footer, also
/// outline that story's cells (#2197), without adding printable borders or changing the view flag.
pub(in crate::canvas) fn gridline_cells(page: &Page, all: bool, editing: Option<StoryRef>) -> impl Iterator<Item = wordcraft_geom::Rect> + '_ {
    // Automatic header/footer guides need not scan a potentially large body placement list.
    let body = if all { page.items.as_slice() } else { &[] };
    body.iter().chain(&page.header).chain(&page.footer).filter_map(move |it| match it {
        Placed::Cell { rect, covered: false, story, .. } if all || Some(*story) == editing => Some(*rect),
        _ => None,
    })
}

/// The table gridline segments to draw on a page, in document points: the edges of the
/// [`gridline_cells`] minus the parts a printed border already draws. Word shows gridlines only
/// where a cell has no border; drawn over a border, the thin blue guide tinted every bordered
/// table blue and made its lines look uneven when zoomed out (#5967).
pub(in crate::canvas) fn gridline_segments(page: &Page, all: bool, editing: Option<StoryRef>) -> Vec<(wordcraft_geom::Point, wordcraft_geom::Point)> {
    use wordcraft_geom::Point;
    // A rule this close to a cell edge (points) is drawn on it.
    const ON_EDGE: f32 = 0.25;
    // Rules by the line they run along (horizontal or not, coordinate in quarter points), so
    // each cell edge looks only at the rules on its own line. Body rules cannot cover header or
    // footer cells, so they are left out when only those are outlined, as in `gridline_cells`.
    let key = |c: f32| (c * 4.0).round() as i64;
    let body = if all { page.items.as_slice() } else { &[] };
    let mut rules: std::collections::HashMap<(bool, i64), Vec<(f32, f32, f32)>> = std::collections::HashMap::new();
    for it in body.iter().chain(&page.header).chain(&page.footer) {
        let Placed::Rule { x0, y0, x1, y1, border } = it else { continue };
        if !border.is_visible() {
            continue;
        }
        let (horizontal, c0, c1, s0, s1) = if (y0 - y1).abs() <= ON_EDGE { (true, *y0, *y1, *x0, *x1) } else { (false, *x0, *x1, *y0, *y1) };
        if (c0 - c1).abs() <= ON_EDGE && c0.is_finite() {
            rules.entry((horizontal, key(c0))).or_default().push((c0, s0.min(s1), s0.max(s1)));
        }
    }
    // The rules covering the line at `at`, sorted and merged into disjoint spans. Cells share their
    // edges, so each distinct line is sorted once (not once per cell edge) and every edge then
    // binary-searches its start.
    let mut merged: std::collections::HashMap<(bool, u32), Vec<(f32, f32)>> = std::collections::HashMap::new();
    // The parts of [a, b] along a line at `at` that no rule on the same line covers; `horizontal`
    // picks rules running along x at y = `at`, otherwise along y at x = `at`.
    let mut uncovered = |horizontal: bool, at: f32, a: f32, b: f32| -> Vec<(f32, f32)> {
        let covers = merged.entry((horizontal, at.to_bits())).or_insert_with(|| {
            let k = key(at);
            let mut covers: Vec<(f32, f32)> = (k - 1..=k + 1)
                .filter_map(|k| rules.get(&(horizontal, k)))
                .flatten()
                .filter(|(c, _, _)| (c - at).abs() <= ON_EDGE)
                .map(|&(_, s0, s1)| (s0, s1))
                .collect();
            covers.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut spans: Vec<(f32, f32)> = Vec::with_capacity(covers.len());
            for (c0, c1) in covers {
                match spans.last_mut() {
                    Some(last) if c0 <= last.1 + ON_EDGE => last.1 = last.1.max(c1),
                    _ => spans.push((c0, c1)),
                }
            }
            spans
        });
        let mut out = Vec::new();
        let mut from = a;
        for &(c0, c1) in &covers[covers.partition_point(|s| s.1 < a)..] {
            if c0 > from + ON_EDGE {
                out.push((from, c0.min(b)));
            }
            from = from.max(c1);
            if from >= b {
                break;
            }
        }
        if from + ON_EDGE < b {
            out.push((from, b));
        }
        out
    };
    let mut segs = Vec::new();
    for r in gridline_cells(page, all, editing) {
        let (l, t, rt, bt) = (r.x, r.y, r.x + r.w, r.y + r.h);
        for y in [t, bt] {
            segs.extend(uncovered(true, y, l, rt).into_iter().map(|(a, b)| (Point::new(a, y), Point::new(b, y))));
        }
        for x in [l, rt] {
            segs.extend(uncovered(false, x, t, bt).into_iter().map(|(a, b)| (Point::new(x, a), Point::new(x, b))));
        }
    }
    segs
}
