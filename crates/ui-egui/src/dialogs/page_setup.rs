use craft::pages::ui::PageSetupDialog;
use craft::pages::{DocumentGrid, GridKind, MAX_GRID_COUNT, Margins, Orientation, PageSetup, PageVerticalAlign};
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::runtime::env::{Label, UnitKind};
use wordcraft_doc::section::{DocGridType, PageVAlign};

use super::host::{DialogHooks, Settle, open_with, run_as_one_step};
use super::{Value, WordApp};
use egui::Ui;
use serde_json::json;

/// Labels of Page Setup's paper size (#1433), in one place in case Word's wording differs.
pub(crate) const PAPER_LABELS: PaperLabels = PaperLabels { more: "More Paper Sizes…" };

pub(crate) struct PaperLabels {
    /// The Size menu's last item, opening Page Setup.
    pub more: &'static str,
}

/// The commands Page Setup's OK runs for the paper, orientation and margins (#1433), all in points.
/// Refuses when the margins don't fit the paper.
/// Otherwise it orders them so that none is refused on the way: the size first
/// when the section's margins fit it, else the margins first, else margins of 0 in between.
/// `layout.size` keeps the section's orientation; `layout.orientation` then turns the page and
/// its margins to what the dialog shows. A size equal to the opened one (either way round,
/// within 0.01 pt) doesn't run `layout.size`.
pub(super) fn page_layout_commands(
    app: &WordApp,
    (paper, was): ((f32, f32), (f32, f32)),
    landscape: bool,
    m: (f32, f32, f32, f32),
) -> Result<Vec<(String, Value)>, &'static str> {
    use wordcraft_engine::cmd::page::{MARGINS_DONT_FIT, margins_fit, sect};
    let cur = sect(&app.session);
    if !margins_fit(paper, m, cur.gutter) {
        return Err(MARGINS_DONT_FIT);
    }
    let turned = |on: bool, (a, b): (f32, f32)| if on { (a.max(b), a.min(b)) } else { (a.min(b), a.max(b)) };
    let ((w0, h0), (w1, h1)) = (turned(false, paper), turned(false, was));
    let new_size = (w0 - w1).abs() >= 0.01 || (h0 - h1).abs() >= 0.01;
    let size = ("layout.size".to_string(), json!({"width": paper.0, "height": paper.1}));
    let orientation = ("layout.orientation".to_string(), json!({"value": if landscape { "landscape" } else { "portrait" }}));
    let margins = ("layout.margins".to_string(), json!({"top": m.0, "bottom": m.1, "left": m.2, "right": m.3}));
    let old_margins = (cur.margin_top, cur.margin_bottom, cur.margin_left, cur.margin_right);
    Ok(if !new_size || margins_fit(turned(cur.landscape, paper), old_margins, cur.gutter) {
        let mut v = if new_size { vec![size] } else { Vec::new() };
        v.extend([orientation, margins]);
        v
    } else if margins_fit(turned(landscape, (cur.page_w, cur.page_h)), m, cur.gutter) {
        vec![orientation, margins, size]
    } else {
        vec![orientation, ("layout.margins".to_string(), json!({"top": 0.0, "bottom": 0.0, "left": 0.0, "right": 0.0})), size, margins]
    })
}

/// The dialog's vertical alignment for the section's.
fn valign_in(v: PageVAlign) -> PageVerticalAlign {
    match v {
        PageVAlign::Top => PageVerticalAlign::Top,
        PageVAlign::Center => PageVerticalAlign::Center,
        PageVAlign::Justify => PageVerticalAlign::Justify,
        PageVAlign::Bottom => PageVerticalAlign::Bottom,
    }
}

/// `layout.verticalAlign`'s name for a value.
fn valign_name(v: PageVerticalAlign) -> &'static str {
    match v {
        PageVerticalAlign::Center => "center",
        PageVerticalAlign::Justify => "justify",
        PageVerticalAlign::Bottom => "bottom",
        _ => "top",
    }
}

/// `layout.documentGrid`'s name for a kind.
fn grid_name(kind: GridKind) -> &'static str {
    match kind {
        GridKind::Lines => "lines",
        GridKind::LinesAndChars => "linesAndChars",
        GridKind::SnapToChars => "snapToChars",
        _ => "none",
    }
}

/// The setup of the section holding the caret, in points.
fn read_setup(app: &WordApp) -> PageSetup {
    let sp = wordcraft_engine::cmd::page::sect(&app.session);
    let kind = match sp.doc_grid.map(|g| g.kind) {
        Some(DocGridType::Lines) => GridKind::Lines,
        Some(DocGridType::LinesAndChars) => GridKind::LinesAndChars,
        Some(DocGridType::SnapToChars) => GridKind::SnapToChars,
        _ => GridKind::None,
    };
    let size = app.session.doc.styles.resolve_char(Some("Normal"), &Default::default()).size;
    let grid = DocumentGrid::new(kind, sp.grid_lines_per_page().min(MAX_GRID_COUNT), sp.grid_chars_per_line(size).min(MAX_GRID_COUNT));
    PageSetup {
        width: f64::from(sp.page_w),
        height: f64::from(sp.page_h),
        orientation: if sp.landscape { Orientation::Landscape } else { Orientation::Portrait },
        margins: Margins {
            top: f64::from(sp.margin_top),
            bottom: f64::from(sp.margin_bottom),
            left: f64::from(sp.margin_left),
            right: f64::from(sp.margin_right),
        },
        gutter: f64::from(sp.gutter),
        mirrored: app.session.doc.settings.mirror_margins,
        pages: None,
        vertical_align: valign_in(sp.valign),
        different_first: sp.title_page,
        different_odd_even: app.session.doc.settings.even_odd_headers,
        right_to_left: sp.rtl,
        grid: Some(grid),
    }
}

/// Page Setup on the dialog stack: the framework dialog over the setup, with OK held until the
/// commands ran, so a refusal shows in the dialog.
pub(crate) struct PageSetupHost {
    /// What the dialog shows, in points.
    pub(super) setup: PageSetup,
    /// The setup as opened: only what differs from it is applied.
    was: PageSetup,
    unit: UnitKind,
    held: HeldAnswer<PageSetup>,
}

impl StackDialog for PageSetupHost {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("pageSetup")
    }

    fn key(&self) -> Option<&str> {
        Some("pageSetup")
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<()> {
        if self.held.is_accepted() {
            return self.held.finish().map(|_| ());
        }
        if !self.held.is_waiting() {
            self.held.file(self.setup.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.held.finish().is_some() {
            return Some(DialogResult::Ok(()));
        }
        let out = PageSetupDialog::new(Bind::bound(&mut self.setup)).unit(self.unit).layout().direction().held(&mut self.held).show(ui);
        out.closed.map(|_| DialogResult::Cancel)
    }
}

/// The commands for the setup OK filed, as one undo step.
fn apply(app: &mut WordApp, setup: &PageSetup, was: &PageSetup) -> Result<(), String> {
    use wordcraft_engine::cmd::page::sect;
    let size = |s: &PageSetup| (s.width as f32, s.height as f32);
    let m = &setup.margins;
    let landscape = setup.orientation == Orientation::Landscape;
    let mut commands = page_layout_commands(app, (size(setup), size(was)), landscape, (m.top as f32, m.bottom as f32, m.left as f32, m.right as f32))
        .map_err(|e| tl!(e).to_string())?;
    let cur = sect(&app.session);
    // Mirror margins and the gutter go with the margins (#1181).
    if let Some((_, v)) = commands.iter_mut().rev().find(|(id, _)| id.as_str() == "layout.margins") {
        v["mirror"] = json!(setup.mirrored);
        if (setup.gutter as f32 - cur.gutter).abs() > f32::EPSILON {
            v["gutter"] = json!(setup.gutter);
        }
    }
    if setup.grid != was.grid
        && let Some(grid) = &setup.grid
    {
        let mut v = json!({"type": grid_name(grid.kind)});
        if Some(grid.lines_per_page) != was.grid.as_ref().map(|g| g.lines_per_page) {
            v["linesPerPage"] = json!(grid.lines_per_page.min(MAX_GRID_COUNT));
        }
        if Some(grid.chars_per_line) != was.grid.as_ref().map(|g| g.chars_per_line) {
            v["charsPerLine"] = json!(grid.chars_per_line.min(MAX_GRID_COUNT));
        }
        commands.push(("layout.documentGrid".to_string(), v));
    }
    if setup.right_to_left != cur.rtl {
        commands.push(("layout.sectionRtl".to_string(), json!({"value": setup.right_to_left})));
    }
    if setup.vertical_align != valign_in(cur.valign) {
        commands.push(("layout.verticalAlign".to_string(), json!({"value": valign_name(setup.vertical_align)})));
    }
    if setup.different_odd_even != app.session.doc.settings.even_odd_headers {
        commands.push(("layout.differentOddEven".to_string(), json!({"value": setup.different_odd_even})));
    }
    if setup.different_first != cur.title_page {
        commands.push(("layout.differentFirstPage".to_string(), json!({"value": setup.different_first})));
    }
    run_as_one_step(app, commands)
}

/// Takes the filed setup, runs it and answers the dialog with the outcome.
fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(host) = app.dialog_host.get_mut_by_id::<PageSetupHost>(id) else {
        return Settle::Done;
    };
    let Some(setup) = host.held.take() else {
        return Settle::Keep;
    };
    let was = host.was.clone();
    let result = apply(app, &setup, &was).map_err(Label::from);
    if let Some(host) = app.dialog_host.get_mut_by_id::<PageSetupHost>(id) {
        host.held.answer(result);
    }
    Settle::Keep
}

/// Opens Page Setup on the section holding the caret.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let setup = read_setup(app);
    let host = PageSetupHost { was: setup.clone(), setup, unit: crate::options::unit_kind(app.session.units()), held: HeldAnswer::new() };
    Some(open_with(app, parent, host, DialogHooks { prepare: None, settle }))
}
