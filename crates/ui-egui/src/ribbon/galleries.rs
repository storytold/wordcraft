//! The ribbon's galleries: Styles, Style Sets, Table Styles (on the framework `Gallery`) and the
//! Shapes and WordArt grids of the menus (on `TileGrid`). Previews are images from the real engine.

use craft::behavior::{DisplayOrder, ItemSource};
use craft::keymap::CommandRef;
use craft::menu::{Gallery, GalleryFace, GalleryItem, GalleryMore};
use craft::theme::Mode;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Trigger,
};
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::IconSpec;
use craft::ui::primitives::textures::Revision;
use craft::ui::runtime::commands::resolve;
use craft::ui::runtime::env::Env;
use craft::ui::tile_grid::{CaptionPlace, GridZoom, Tile, TileGrid};
use egui::{Color32, ColorImage, Pos2, Ui};
use serde_json::json;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Table, para_block};

use crate::WordApp;
use crate::previews::{SnippetOptions, dim, picture_key, preview_revision, snippet_blocks, style_revision, style_snippet};

/// The basic shapes of Insert › Shapes: tooltip, `insert.shape` kind and the registered icon.
const SHAPE_TILES: [(&str, &str, &str); 9] = [
    ("Rectangle", "rectangle", "shape.rectangle"),
    ("Rounded Rectangle", "roundedRectangle", "shape.roundedRectangle"),
    ("Oval", "ellipse", "shape.ellipse"),
    ("Triangle", "triangle", "shape.triangle"),
    ("Diamond", "diamond", "shape.diamond"),
    ("Line", "line", "shape.line"),
    ("Arrow", "arrow", "shape.arrow"),
    ("Star", "star", "shape.star"),
    ("Heart", "heart", "shape.heart"),
];

/// Shapes per row of the Insert › Shapes grid.
const SHAPE_COLUMNS: usize = 5;
/// WordArt styles per row.
const WORDART_COLUMNS: usize = 4;
/// Style sets shown in the Design tab's gallery, in order.
const STYLE_SETS: [&str; 7] = ["default", "basic", "lines", "shaded", "casual", "centered", "minimalist"];
/// Width of the rendered style sample in page points, the same class as the snippet sizes beside it: the
/// first letters of the sample. The tile scales the picture into its own extent.
const STYLE_TILE_W: f32 = 68.0;

/// The interface mode, read once per gallery.
fn interface_mode(ui: &Ui) -> Mode {
    Env::of(ui).view().mode()
}

/// Makes one tile's image.
type Maker<'a> = Box<dyn Fn() -> ColorImage + 'a>;

/// A transparent 1 px picture, for a tile whose layout produced nothing.
fn blank_image() -> ColorImage {
    ColorImage::filled([1, 1], Color32::TRANSPARENT)
}

/// `img` cut to its left `width` pixels.
fn crop_left(img: &ColorImage, width: usize) -> ColorImage {
    let [w, h] = img.size;
    let keep = width.clamp(1, w.max(1));
    if w == 0 || keep >= w {
        return img.clone();
    }
    let mut pixels = Vec::with_capacity(keep.saturating_mul(h));
    for row in img.pixels.chunks(w) {
        pixels.extend_from_slice(row.get(..keep).unwrap_or(row));
    }
    ColorImage::new([keep, h], pixels)
}

/// A Styles gallery tile: the sample letters in style `id`, the first line starting at the tile's edge.
pub(crate) fn style_tile_image(doc: &Document, id: &str, ppp: f32, dark_page: bool, dark: bool) -> ColorImage {
    // Sample letters in the interface's script (`АаБбВвГг` in Russian).
    let mut p = Paragraph::with_text(tl!("AaBbCcDd"), CharProps::default()).styled(id);
    p.props.space_before = Some(0.0);
    p.props.align = Some(wordcraft_doc::Align::Left);
    // The sample letters are a left-to-right script, in the Arabic interface too (its
    // catalog keeps `AaBbCcDd`), so a right-to-left style's tile keeps them at the left
    // edge, where they start (#4944). A sample translated into a right-to-left script
    // would be cut at its start here and need a solution of its own.
    p.props.bidi = Some(false);
    // Keep hanging indents (bullets) but start the first line at the tile's left edge.
    let first = doc.styles.resolve_para(&p.props).indent_first;
    p.props.indent_left = Some(if first.is_finite() { (-first).max(0.0) } else { 0.0 });
    p.props.borders = None;
    p.props.shading = None;
    let Some(img) = style_snippet(doc, p, 220.0, 30.0, ppp, 1.0, dark_page) else { return blank_image() };
    let mut img = crop_left(&img, (STYLE_TILE_W * ppp).round().max(1.0) as usize);
    if dark && !dark_page {
        dim(&mut img);
    }
    img
}

/// A Style Sets tile: a title, a heading and a paragraph in the set `name`.
fn style_set_image(name: &str, ppp: f32, dark: bool) -> ColorImage {
    let mut s = wordcraft_engine::Session::new(Document::new());
    let _ = s.run("design.styleSet", &json!({"name": name}));
    let blocks = vec![
        Block::Para(Paragraph::with_text(crate::i18n::tc("preview", "Title"), CharProps::default()).styled("Title")),
        Block::Para(Paragraph::with_text(tl!("Heading 1"), CharProps::default()).styled("Heading1")),
        Block::Para(Paragraph::with_text(tl!("Body text in a short paragraph to show spacing."), CharProps::default())),
    ];
    let options = SnippetOptions { width: 120.0, height: 110.0, pixels_per_point: ppp * 0.42, margin: 6.0, paper: None, dark: false };
    let Some(mut img) = snippet_blocks(&s.doc, blocks, options) else { return blank_image() };
    if dark {
        dim(&mut img);
    }
    img
}

/// A Table Styles tile: a 5×4 table in `style`.
fn table_style_image(doc: &Document, style: &str, ppp: f32, dark: bool) -> ColorImage {
    let mut tb = Table::new(5, 4, 100.0);
    tb.props.style = Some(style.to_string());
    for row in &mut tb.rows {
        for c in &mut row.cells {
            c.blocks = vec![para_block(Paragraph::with_text(" ", CharProps { size: Some(5.0), ..Default::default() }))];
        }
    }
    let options = SnippetOptions { width: 104.0, height: 70.0, pixels_per_point: ppp * 0.48, margin: 4.0, paper: None, dark: false };
    let Some(mut img) = snippet_blocks(doc, vec![Block::Table(tb), Block::Para(Paragraph::new())], options) else { return blank_image() };
    if dark {
        dim(&mut img);
    }
    img
}

/// Gallery items of `entries` (id, name), each drawn by its maker and keyed by its kind, id and density.
fn image_items<'a>(kind: &str, entries: &[(String, String)], makers: &'a [Maker<'a>], ppp: f32, revision: Revision) -> Vec<GalleryItem<'a, String>> {
    entries
        .iter()
        .zip(makers)
        .map(|((id, name), make)| GalleryItem {
            key: id.clone(),
            label: name.clone().into(),
            face: GalleryFace::image(picture_key(&(kind, id, ppp.to_bits())), revision, &**make),
        })
        .collect()
}

/// The Home tab's Styles gallery; `current` is the selection's style id. A tile's menu has Update to Match
/// Selection and Modify….
pub(crate) fn styles(ui: &mut Ui, app: &mut WordApp, current: Option<&str>) {
    let mode = interface_mode(ui);
    let dark = mode == Mode::Dark;
    let ppp = ui.ctx().pixels_per_point();
    let revision = style_revision(app, mode);
    let dark_page = app.session.view.dark_mode;
    let entries: Vec<(String, String)> = app.session.doc.styles.gallery().iter().map(|s| (s.id.clone(), crate::i18n::style_name(s))).collect();
    let current = current.unwrap_or("Normal").to_string();
    let out = {
        let doc = &app.session.doc;
        let makers: Vec<Maker<'_>> =
            entries.iter().map(|(id, _)| Box::new(move || style_tile_image(doc, id, ppp, dark_page, dark)) as Maker<'_>).collect();
        let items = image_items("style", &entries, &makers, ppp, revision);
        // Each tile shows the sample's first letters at their size: a large thumbnail as wide as `STYLE_TILE_W` crops
        // them (68 by 48 points), so the sample is not scaled down, with the style's name under it.
        Gallery::new(tl!("Styles"), &items, Some(&current))
            .tiles(6, 3)
            .thumb(ThumbSize::Large)
            .aspect(ThumbAspect::new(17, 12))
            .caption(CaptionPlace::Below)
            .framed()
            .more(GalleryMore::Command(CommandRef::with_params("view.stylesPane", json!({"value": true}))))
            .show(ui)
    };
    if let Some(id) = out.picked {
        let _ = app.run("para.style", json!({"style": id}));
    }
    // The menu's target stays in memory until the menu closes, as the gallery's request is made once.
    let menu_id = ui.scope_id().with("style_tile_menu");
    if let Some(request) = out.menu {
        ui.data_mut(|d| d.insert_temp(menu_id, (request.key, request.at)));
    }
    let Some((style, at)) = ui.data(|d| d.get_temp::<(String, Pos2)>(menu_id)) else { return };
    let mut open = true;
    let (mut updated, mut modify) = (false, false);
    let _ =
        Menu::new(tl!("Style")).opened_by(&out.response, Trigger::Manual).anchor(Anchor::Point(at)).open(Bind::bound(&mut open)).show(ui, |ui, _| {
            updated = MenuItem::command(CommandRef::with_params("styles.updateToMatch", json!({"style": style}))).show(ui).ran;
            modify = MenuItem::new(tl!("Modify…")).show(ui).ran;
        });
    if modify {
        let _ = crate::dialogs::style_forms::modify_style(app, &style);
    }
    if !open || updated || modify {
        ui.data_mut(|d| d.remove::<(String, Pos2)>(menu_id));
    }
}

/// The Design tab's style set gallery.
pub(crate) fn style_sets(ui: &mut Ui, app: &mut WordApp) {
    let dark = interface_mode(ui) == Mode::Dark;
    let ppp = ui.ctx().pixels_per_point();
    // The tiles draw a fresh document: no styles hash, but the fonts they are set in are an input.
    let revision = preview_revision(0, wordcraft_fonts::FontDb::global().generation(), crate::i18n::current().code(), false, dark);
    let entries: Vec<(String, String)> = STYLE_SETS.iter().map(|name| ((*name).to_string(), format!("Style set: {name}"))).collect();
    let makers: Vec<Maker<'_>> = entries.iter().map(|(name, _)| Box::new(move || style_set_image(name, ppp, dark)) as Maker<'_>).collect();
    let items = image_items("set", &entries, &makers, ppp, revision);
    // Each tile is a small page (title, heading, paragraph) at the large thumbnail size, about as wide as high as the
    // image `style_set_image` renders, so the title and heading read as they did in the old ribbon. All seven show at
    // full width, so no buttons stand beside them, as in the old ribbon.
    let out =
        Gallery::new(tl!("Style Sets"), &items, None).tiles(7, 4).thumb(ThumbSize::Large).aspect(ThumbAspect::new(12, 11)).spare_buttons().show(ui);
    if let Some(name) = out.picked {
        let _ = app.run("design.styleSet", json!({"name": name}));
    }
}

/// Table Design's gallery of table styles; a modified table style redraws, as the revision covers the style sheet.
pub(crate) fn table_styles(ui: &mut Ui, app: &mut WordApp) {
    let mode = interface_mode(ui);
    let dark = mode == Mode::Dark;
    let ppp = ui.ctx().pixels_per_point();
    let revision = style_revision(app, mode);
    let mut styles: Vec<(bool, String, String)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && !s.hidden)
        .map(|s| (s.builtin, s.id.clone(), crate::i18n::style_name(s)))
        .collect();
    styles.sort_by_key(|(builtin, _, _)| *builtin);
    let entries: Vec<(String, String)> = styles.into_iter().map(|(_, id, name)| (id, name)).collect();
    let out = {
        let doc = &app.session.doc;
        let makers: Vec<Maker<'_>> = entries.iter().map(|(id, _)| Box::new(move || table_style_image(doc, id, ppp, dark)) as Maker<'_>).collect();
        let items = image_items("table", &entries, &makers, ppp, revision);
        // Each tile shows a 5×4 table sample at the large thumbnail size in the 3:2 landscape
        // ratio `table_style_image` renders (112×74 pt including margins), matching the sibling
        // `Styles` and `Style Sets` galleries and v0.18.1's ribbon previews.
        Gallery::new(tl!("Table Styles"), &items, None).tiles(7, 4).thumb(ThumbSize::Large).aspect(ThumbAspect::new(3, 2)).show(ui)
    };
    if let Some(style) = out.picked {
        let _ = app.run("table.style", json!({"style": style}));
    }
}

/// Tiles keyed by position.
struct Indexed(usize);

impl DisplayOrder<usize> for Indexed {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Indexed {}

/// The position of the tile a grid reports picked this pass.
fn picked(events: &[CollectionEvent<usize, craft::ui::tile_grid::TilePartEvent>]) -> Option<usize> {
    events.iter().find_map(|event| match event {
        CollectionEvent::Activate(at) => Some(*at),
        _ => None,
    })
}

/// The basic shapes as a grid of icon tiles, each named by its label; a click inserts it. Disabled while
/// `insert.shape` is.
// PLAN: takes the menu body's `cx` (S-L371) beside `ui`; callers are ribbon-w16 (Insert › Shapes).
pub(crate) fn shapes(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    let enabled = resolve(ui.ctx(), "insert.shape").enabled;
    let out = TileGrid::new(tl!("Shapes"))
        .select(SelectMode::None)
        .activate(ActivateOn::Click)
        .card_columns(SHAPE_COLUMNS)
        .content_cells()
        // Every shape shows, as in production; the menu's body is the one scroll.
        .natural_height(true)
        .caption(CaptionPlace::None)
        .enabled(enabled)
        .show_items(ui, &Indexed(SHAPE_TILES.len()), &mut |at, _| {
            let (label, _, icon) = SHAPE_TILES.get(*at).copied().unwrap_or(("", "", "shape.rectangle"));
            Tile::new(crate::tl!(label)).icon(IconSpec::svg(crate::icons::id(icon)))
        });
    if let Some((_, kind, _)) = picked(&out.response.events).and_then(|at| SHAPE_TILES.get(at))
        && app.run("insert.shape", json!({"kind": kind})).is_ok()
    {
        cx.request_close();
    }
}

/// Our WordArt styles as a grid of drawn tiles; a pick runs `command` with `{"style": id}`.
// PLAN: takes the menu body's `cx` (S-L371) beside `ui`; callers are ribbon-w15, w16 and w18.
pub(crate) fn wordart(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp, command: &str) {
    let theme = app.session.doc.settings.theme_colors.clone();
    let styles = wordcraft_doc::wordart::ART_STYLES;
    let out = TileGrid::new(tl!("Decorative Text Styles"))
        .select(SelectMode::None)
        .activate(ActivateOn::Click)
        .card_columns(WORDART_COLUMNS)
        .thumb_size(ThumbSize::Small)
        .zoom(GridZoom::Fixed(2.0))
        // Every style shows, as in production; the menu's body is the one scroll.
        .natural_height(true)
        .caption(CaptionPlace::None)
        .show_items(ui, &Indexed(styles.len()), &mut |at, _| {
            let (id, label) = styles.get(*at).copied().unwrap_or(("", ""));
            let theme = &theme;
            Tile::new(crate::tl!(label)).thumb(ThumbAspect::new(10, 9), move |cx, r| crate::wordart_menus::paint_style(cx, r, id, theme))
        });
    if let Some((id, _)) = picked(&out.response.events).and_then(|at| styles.get(at))
        && app.run(command, json!({"style": id})).is_ok()
    {
        cx.request_close();
        // The WordArt's text is selected: the page takes the keyboard so typing replaces it at once (#4061).
        app.canvas.focus_page(ui.ctx());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Relative luminance (WCAG) of an opaque sRGB colour, in `0.0..=1.0`.
    fn rel_luma(c: Color32) -> f64 {
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        let lin = |u: u8| {
            let s = f64::from(u) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    /// WCAG contrast ratio between two opaque colours.
    fn contrast_ratio(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (rel_luma(a), rel_luma(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The background paper pixel: the most frequent colour in the tile.
    fn paper_color(img: &ColorImage) -> Color32 {
        let mut counts = std::collections::HashMap::<Color32, usize>::new();
        for &px in &img.pixels {
            *counts.entry(px).or_insert(0) += 1;
        }
        counts.into_iter().max_by_key(|&(_, n)| n).expect("non-empty tile").0
    }

    /// #7309: Styles gallery tiles keep opaque paper in light and dark modes (dimmed in dark mode,
    /// matching #663 and sibling galleries) so dark style ink remains legible, and leave Switch
    /// Modes' dark page untouched by interface dimming.
    #[test]
    fn style_tile_keeps_opaque_paper_and_legible_contrast_in_dark_mode() {
        let doc = Document::new();
        let light = style_tile_image(&doc, "Normal", 1.0, false, false);
        let dark = style_tile_image(&doc, "Normal", 1.0, false, true);
        let dark_page_light_ui = style_tile_image(&doc, "Normal", 1.0, true, false);
        let dark_page_dark_ui = style_tile_image(&doc, "Normal", 1.0, true, true);

        assert!(light.pixels.iter().all(|p| p.a() == 255), "light-mode style tile keeps opaque paper");
        assert!(dark.pixels.iter().all(|p| p.a() == 255), "dark-mode style tile keeps opaque paper (not a translucent veil)");
        assert!(dark_page_dark_ui.pixels.iter().all(|p| p.a() == 255), "dark-page style tile stays opaque");

        let lightest = |img: &ColorImage| img.pixels.iter().copied().max_by(|a, b| rel_luma(*a).total_cmp(&rel_luma(*b))).expect("non-empty tile");
        let darkest = |img: &ColorImage| img.pixels.iter().copied().min_by(|a, b| rel_luma(*a).total_cmp(&rel_luma(*b))).expect("non-empty tile");

        let light_paper = paper_color(&light);
        let light_ink = darkest(&light);
        let dark_paper = paper_color(&dark);
        let dark_ink = darkest(&dark);
        let dark_page_paper = paper_color(&dark_page_dark_ui);
        let dark_page_ink = lightest(&dark_page_dark_ui);

        assert!(rel_luma(light_paper) > rel_luma(dark_paper), "dark mode dims white paper below light mode");
        assert!(rel_luma(dark_paper) > rel_luma(dark_ink), "dark-mode tile ink stays darker than its dimmed paper");
        assert!(
            contrast_ratio(light_paper, light_ink) >= 4.5,
            "light-mode Normal tile needs >= 4.5:1 contrast (got {:.2}:1)",
            contrast_ratio(light_paper, light_ink)
        );
        assert!(
            contrast_ratio(dark_paper, dark_ink) >= 4.5,
            "dark-mode Normal tile needs >= 4.5:1 contrast (got {:.2}:1)",
            contrast_ratio(dark_paper, dark_ink)
        );
        assert_eq!(dark_page_light_ui.pixels, dark_page_dark_ui.pixels, "Switch Modes' dark page is not dimmed again by dark mode");
        assert!(rel_luma(dark_page_paper) < rel_luma(dark_paper), "Switch Modes renders onto a dark page rather than dimmed white paper");
        assert!(rel_luma(dark_page_ink) > rel_luma(dark_page_paper), "Switch Modes renders light ink onto the dark page");
        assert!(
            contrast_ratio(dark_page_paper, dark_page_ink) >= 4.5,
            "dark-page Normal tile needs >= 4.5:1 contrast (got {:.2}:1)",
            contrast_ratio(dark_page_paper, dark_page_ink)
        );
    }

    /// #2: a style whose hanging indent is larger than its left indent previewed with its first line
    /// starting off the tile's left edge; the first line now starts at the edge.
    // PLAN: the sheet puts this test in previews/tests.rs; `ribbon::galleries` is private to `ribbon`, so
    // it lives beside the maker.
    #[test]
    fn a_hanging_indent_style_previews_from_the_tile_edge() {
        let mut doc = Document::new();
        let mut style =
            wordcraft_doc::styles::Style { id: "Hanging".into(), name: "Hanging".into(), quick: true, priority: Some(0), ..Default::default() };
        style.para.indent_left = Some(0.0);
        style.para.indent_first = Some(-36.0);
        Arc::make_mut(&mut doc.styles).upsert(style);
        let preview = style_tile_image(&doc, "Hanging", 1.0, false, false);
        let [w, h] = preview.size;
        let inked: Vec<bool> = (0..w).map(|x| (0..h).any(|y| preview.pixels.get(y * w + x).is_some_and(|p| p.a() > 0 && p.r() < 160))).collect();
        let first = inked.iter().position(|&i| i).expect("the preview has text");
        let last = inked.iter().rposition(|&i| i).unwrap_or(first);
        // "AaBbCcDd" begins at the left margin (1 point) and is whole: it is about 60 points wide.
        assert!(first < 6, "the first line starts off the tile's left edge (ink from column {first})");
        assert!(last - first > 40, "the first line is cut off by the tile's left edge (ink {first}..{last})");
    }

    /// Builds a quick paragraph style with `name` as both id and display name.
    fn gallery_style(name: &str) -> wordcraft_doc::styles::Style {
        wordcraft_doc::styles::Style { id: name.into(), name: name.into(), quick: true, priority: Some(0), ..Default::default() }
    }

    /// Returns whether column `x` of `img` contains any dark ink pixel.
    fn column_has_ink(img: &ColorImage, x: usize) -> bool {
        let [w, h] = img.size;
        (0..h).any(|y| img.pixels.get(y * w + x).is_some_and(|p| p.a() > 0 && p.r() < 160))
    }

    /// Asserts that two rendered style tiles have identical dimensions and pixels.
    fn assert_same_tile(rtl: &ColorImage, ltr: &ColorImage) {
        assert_eq!(rtl.size, ltr.size, "right-to-left tile dimensions differ from its left-to-right twin");
        let differ = rtl.pixels.iter().zip(&ltr.pixels).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{differ} pixels differ from the left-to-right twin");
    }

    /// #4944: a right-to-left style's sample was laid out at the right end of its picture, and the
    /// tile crops the left end, so the tile was blank. The sample now stands at the left edge.
    #[test]
    fn a_right_to_left_style_previews_its_sample_text() {
        let mut doc = Document::new();
        let mut style = gallery_style("Right to left");
        style.para.bidi = Some(true);
        Arc::make_mut(&mut doc.styles).upsert(style);
        let preview = style_tile_image(&doc, "Right to left", 1.0, false, false);
        let [w, _] = preview.size;
        let inked: Vec<bool> = (0..w).map(|x| column_has_ink(&preview, x)).collect();
        let first = inked.iter().position(|&i| i).expect("the preview has text");
        let last = inked.iter().rposition(|&i| i).unwrap_or(first);
        assert!(first < 6, "the sample starts off the tile's left edge (ink from column {first})");
        assert!(last - first > 40, "the sample is cut off in the tile (ink {first}..{last})");
    }

    /// #4944: the sample reads left to right whatever the style's direction, so with large type,
    /// where it is wider than its tile, a right-to-left tile must be the same picture as its
    /// left-to-right twin. Ink alone would not do: a sample cut at its start has ink too.
    #[test]
    fn a_large_right_to_left_style_previews_like_its_left_to_right_twin() {
        let mut ltr_doc = Document::new();
        let mut ltr_style = gallery_style("Large");
        ltr_style.chr.size = Some(28.0);
        Arc::make_mut(&mut ltr_doc.styles).upsert(ltr_style);

        let mut rtl_doc = Document::new();
        let mut rtl_style = gallery_style("Large");
        rtl_style.chr.size = Some(28.0);
        rtl_style.para.bidi = Some(true);
        Arc::make_mut(&mut rtl_doc.styles).upsert(rtl_style);

        // The uncropped sample must run past the tile width, or this says nothing about which end is kept.
        let mut p = Paragraph::with_text(tl!("AaBbCcDd"), CharProps::default()).styled("Large");
        p.props.space_before = Some(0.0);
        p.props.align = Some(wordcraft_doc::Align::Left);
        let full = style_snippet(&ltr_doc, p, 220.0, 30.0, 1.0, 1.0, false).expect("the uncropped preview renders");
        let cut = STYLE_TILE_W.round() as usize;
        let past_tile = (cut..full.size[0]).any(|x| column_has_ink(&full, x));
        assert!(past_tile, "the sample ends inside the tile width ({cut} px)");

        let ltr = style_tile_image(&ltr_doc, "Large", 1.0, false, false);
        let rtl = style_tile_image(&rtl_doc, "Large", 1.0, false, false);
        assert_same_tile(&rtl, &ltr);
    }

    /// #4944, the issue's steps: Normal updated to match a right-to-left paragraph. A large style
    /// that only inherits the direction from Normal must show the same tile as in a document
    /// without any right-to-left.
    #[test]
    fn a_style_inheriting_right_to_left_from_normal_previews_like_its_left_to_right_twin() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
        for (command, params) in [("text.insert", json!({"text": "Some text"})), ("para.rtl", json!({})), ("styles.updateToMatch", json!({}))] {
            let _ = app.run(command, params);
        }
        let mut right_to_left = app.session.doc.clone();
        let normal = wordcraft_doc::props::ParaProps { style: Some("Normal".into()), ..Default::default() };
        assert!(right_to_left.styles.resolve_para(&normal).bidi, "Update to Match made Normal right to left");

        let mut ltr_doc = Document::new();
        for doc in [&mut right_to_left, &mut ltr_doc] {
            let mut style = gallery_style("Large");
            style.based_on = Some("Normal".into());
            style.chr.size = Some(28.0);
            Arc::make_mut(&mut doc.styles).upsert(style);
        }
        let rtl = style_tile_image(&right_to_left, "Large", 1.0, false, false);
        let ltr = style_tile_image(&ltr_doc, "Large", 1.0, false, false);
        assert_same_tile(&rtl, &ltr);
    }
}
