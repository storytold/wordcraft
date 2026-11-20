//! The Equation tab (shown while editing an equation) and the Insert › Equation gallery:
//! ready-made equations, input format and conversion, symbol sets, and the structure galleries
//! (fractions, scripts, radicals, integrals, large operators, brackets, functions, accents,
//! limits and logs, operators, matrices). Tiles and button pictures are rendered by the math
//! engine itself.

use craft::behavior::{DisplayOrder, IconId, ItemSource, Selection};
use craft::keymap::CommandRef;
use craft::menu::ribbon::{Gallery, GalleryFace, GalleryItem, GalleryMore, RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, SplitButton, SplitMenu, ToggleButton};
use craft::ui::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, Gap, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::runtime::env::Label;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{Color32, Ui};
use serde_json::json;
use wordcraft_doc::math::{MNode, MRun, parse_linear};
use wordcraft_engine::math_gallery::{BUILT_INS, STRUCTURES, SYMBOL_SETS};

use crate::WordApp;
use crate::previews::{MathSource, math_key, math_picture, math_thumbnails};
use crate::ribbon::common::menu_button;

/// What each structure button shows.
pub(crate) fn button_picture(id: &str) -> &'static str {
    match id {
        "fraction" => "x/y",
        "script" => "e^x",
        "radical" => "√(n&x)",
        "integral" => "∫_(−x)^x",
        "largeOperator" => "∑_(i=0)^n",
        "bracket" => "{()}",
        "function" => "sin⁡θ",
        "accent" => "ä",
        "limitLog" => "lim┬(n→∞)",
        "operator" => "≜",
        "matrix" => "[■(1&0@0&1)]",
        _ => "x",
    }
}

/// The ink math pictures are rendered in: the theme's text colour.
fn ink(ui: &Ui) -> Color32 {
    ui.visuals().text_color()
}

/// The keys `0..n` as a collection source.
pub(crate) struct Indexes(pub usize);

impl DisplayOrder<usize> for Indexes {
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

impl ItemSource<usize> for Indexes {}

/// A grid of `count` tiles in `columns` columns, `current` marked; returns the tile the user activated this
/// pass (a click or Enter). Arrow keys only move the lead, so a keyboard user can browse the grid without
/// running a tile.
pub(crate) fn pick_tile<'r>(
    ui: &mut Ui,
    label: &str,
    columns: usize,
    caption: CaptionPlace,
    current: Option<usize>,
    count: usize,
    mut tile: impl FnMut(usize) -> Tile<'r> + 'r,
) -> Option<usize> {
    let selection = Selection::<usize>::new(current.into_iter().collect(), current, current);
    let grid = TileGrid::<usize>::new(label.to_string())
        .select(SelectMode::Single)
        .selection(Bind::given(selection))
        .activate(ActivateOn::Click)
        .arrows_select(false)
        .caption(caption)
        .card_columns(columns)
        // Shown in a menu whose body is the one scroll (production scrolls the whole menu, not each grid).
        .natural_height(true);
    let out = grid.show_items(ui, &Indexes(count), &mut |key, _| tile(*key));
    let mut picked = None;
    for event in out.response.events {
        if let CollectionEvent::Activate(key) = event {
            picked = Some(key);
        }
    }
    picked
}

/// Builds a readable fallback for a math preview from the interface fonts' glyph coverage:
/// covered characters and whitespace stay, while a missing glyph becomes its UnicodeMath name
/// or a `U+XXXX` codepoint.
fn math_placeholder(ui: &Ui, linear: &str) -> String {
    // Coverage depends on the family, not the size.
    let font = egui::TextStyle::Body.resolve(ui.style());
    math_placeholder_with(linear, |c| ui.fonts_mut(|fonts| fonts.has_glyph(&font, c)))
}

/// [`math_placeholder`] given which characters the interface fonts cover. A character they lack
/// becomes its UnicodeMath name (`┬` → `\below`), so the same template has a different
/// placeholder in a build without craft-fonts; the i18n audit recognizes the exact strings
/// (`placeholder_linears`) as math input rather than interface wording.
pub(crate) fn math_placeholder_with(linear: &str, covered: impl Fn(char) -> bool) -> String {
    let mut result = String::new();
    for c in linear.chars() {
        if c.is_whitespace() || covered(c) {
            result.push(c);
        } else if let Some(name) = wordcraft_doc::math_symbols::name_of(c) {
            result.push('\\');
            result.push_str(name);
        } else {
            use std::fmt::Write as _;
            let _ = write!(result, "U+{:04X}", c as u32);
        }
    }
    result
}

/// The Equation tab's own big button picture.
const EQUATION_PICTURE: &str = "∑_(i=1)^n";

/// Every linear text the Equation tab can paint as a placeholder while a preview loads: the big
/// buttons' pictures, every template and built-in equation, and every symbol tile.
#[cfg(test)]
pub(crate) fn placeholder_linears() -> impl Iterator<Item = String> {
    let pictures = STRUCTURES.iter().map(|g| button_picture(g.id).to_string());
    let templates = wordcraft_engine::math_gallery::templates().map(|t| t.linear.to_string());
    let symbols = SYMBOL_SETS.iter().flat_map(|(_, chars)| chars.chars()).map(|c| c.to_string());
    std::iter::once(EQUATION_PICTURE.to_string())
        .chain(pictures)
        .chain(templates)
        .chain(BUILT_INS.iter().map(|(_, _, linear)| (*linear).to_string()))
        .chain(symbols)
}

/// One button whose picture is an equation over its label; the button opens `body` as a menu.
fn math_menu_button(ui: &mut Ui, picture: &str, label: &str, body: impl FnMut(&mut Ui, &mut BodyCx<'_>)) {
    let thumbs = math_thumbnails(ui.ctx());
    let source = MathSource::new(parse_linear(picture), false, 12.0, [5, 4], ink(ui));
    let face = Button::new(tl!(label).to_string())
        .picture(source.thumb_aspect(), math_picture(tl!(label).to_string(), math_key(&format!("btn:{picture}")), &thumbs, &source))
        .arrangement(Arrangement::Stacked);
    let _ = menu_button(ui, face, tl!(label).to_string(), None, body);
}

/// The picture source of one symbol, as a literal run.
fn symbol_source(c: char, ink: Color32) -> MathSource {
    let nodes = vec![MNode::Run(MRun { text: c.to_string(), lit: true, ..Default::default() })];
    MathSource::new(nodes, false, 11.0, [1, 1], ink)
}

/// A grid of rendered math tiles, `(name, picture key)` each over its own source, shaped as the sources are;
/// returns the tile picked.
fn math_grid(ui: &mut Ui, label: &str, columns: usize, caption: CaptionPlace, tiles: &[(String, String)], sources: &[MathSource]) -> Option<usize> {
    let aspect = sources.first().map(MathSource::thumb_aspect)?;
    let thumbs = math_thumbnails(ui.ctx());
    pick_tile(ui, label, columns, caption, None, tiles.len(), |at| match (tiles.get(at), sources.get(at)) {
        (Some((name, key)), Some(source)) => Tile::new(name.clone()).picture(aspect, math_picture(name.clone(), math_key(key), &thumbs, source)),
        _ => Tile::new(String::new()),
    })
}

/// A structure gallery's menu: a heading and a grid of tiles per section.
fn structure_menu(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp, gallery_id: &str) {
    let Some(g) = STRUCTURES.iter().find(|g| g.id == gallery_id) else { return };
    let ink = ink(ui);
    for (name, templates) in g.sections {
        MenuItem::heading(tl!(name).to_string()).show(ui);
        let wide = name.starts_with("Common") || templates.iter().all(|t| !t.linear.contains('⬚')) && templates.len() <= 3;
        let (shape, columns, pt) = if wide { ([79, 30], 2, 13.0) } else { ([15, 14], 5, 14.0) };
        let sources: Vec<MathSource> = templates.iter().map(|t| MathSource::new(t.nodes(), true, pt, shape, ink)).collect();
        let tiles: Vec<(String, String)> = templates.iter().map(|t| (tl!(t.label).to_string(), format!("tpl:{}:{pt}", t.id))).collect();
        if let Some(tpl) = math_grid(ui, name, columns, CaptionPlace::None, &tiles, &sources).and_then(|i| templates.get(i)) {
            let _ = app.run("equation.insertStructure", json!({"id": tpl.id}));
            app.canvas.focus_page(ui.ctx());
            cx.request_close();
        }
    }
}

/// The ready-made equations (Insert › Equation and Equation › Equation).
fn builtin_menu(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    MenuItem::heading(tl!("Built-In").to_string()).show(ui);
    let ink = ink(ui);
    let sources: Vec<MathSource> = BUILT_INS.iter().map(|(_, _, lin)| MathSource::new(parse_linear(lin), true, 11.0, [175, 32], ink)).collect();
    let tiles: Vec<(String, String)> = BUILT_INS.iter().map(|(id, label, _)| (tl!(label).to_string(), format!("builtin:{id}"))).collect();
    if let Some((id, _, _)) = math_grid(ui, "Built-In", 1, CaptionPlace::Below, &tiles, &sources).and_then(|i| BUILT_INS.get(i)) {
        let _ = app.run("insert.equation", json!({"builtin": id}));
        app.canvas.focus_page(ui.ctx());
        cx.request_close();
    }
    craft::ui::display::Divider::horizontal().show(ui);
    MenuItem::command(CommandRef::with_params("insert.equation", json!({}))).label(tl!("Insert New Equation").to_string()).show(ui);
}

/// Insert › Symbols › Equation: a split button (insert a new equation, or pick a ready-made one).
pub fn insert_button(ui: &mut Ui, app: &mut WordApp) {
    SplitButton::<bool>::command("insert.equation", SplitMenu::Body(Box::new(|ui, cx| builtin_menu(ui, cx, app))))
        .label(tl!("Equation").to_string())
        .icon(IconId::new("equation"))
        .arrangement(Arrangement::Stacked)
        .show(ui);
}

/// The Symbols group: a strip of the current set's symbols, and a menu with every set.
fn symbols(ui: &mut Ui, app: &mut WordApp) {
    let set = app.equation_symbol_set.min(SYMBOL_SETS.len().saturating_sub(1));
    let Some((_, chars)) = SYMBOL_SETS.get(set) else { return };
    let all: Vec<char> = chars.chars().collect();
    let ink = ink(ui);
    let thumbs = math_thumbnails(ui.ctx());
    let sources: Vec<MathSource> = all.iter().map(|c| symbol_source(*c, ink)).collect();
    let items: Vec<GalleryItem<'_, char>> = all
        .iter()
        .zip(&sources)
        .map(|(c, source)| GalleryItem {
            key: *c,
            label: Label::from(symbol_tooltip(*c, &math_placeholder(ui, &c.to_string()))),
            face: GalleryFace::rendered(math_key(&format!("sym:{c}")), &thumbs, source),
        })
        .collect();
    let mut sets = |ui: &mut Ui| {
        let mut chosen = None;
        Row::new().wrap(true).show(ui, |ui, _| {
            for (i, (name, _)) in SYMBOL_SETS.iter().enumerate() {
                if ToggleButton::new(tl!(name).to_string(), Bind::given(i == set)).show(ui).response.clicked() {
                    chosen = Some(i);
                }
            }
        });
        if let Some(i) = chosen {
            app.equation_symbol_set = i;
        }
        craft::ui::display::Divider::horizontal().show(ui);
        let shown = app.equation_symbol_set.min(SYMBOL_SETS.len().saturating_sub(1));
        let Some((_, chars)) = SYMBOL_SETS.get(shown) else { return };
        let chars: Vec<char> = chars.chars().collect();
        let sources: Vec<MathSource> = chars.iter().map(|c| symbol_source(*c, ink)).collect();
        let tiles: Vec<(String, String)> =
            chars.iter().map(|c| (symbol_tooltip(*c, &math_placeholder(ui, &c.to_string())), format!("sym:{c}"))).collect();
        if let Some(c) = math_grid(ui, "All symbol sets", 15, CaptionPlace::None, &tiles, &sources).and_then(|i| chars.get(i)) {
            let _ = app.run("equation.insertSymbol", json!({"char": c.to_string()}));
            app.canvas.focus_page(ui.ctx());
        }
    };
    let out = Gallery::new(tl!("Symbols").to_string(), &items, None).tiles(30, 10).more(GalleryMore::Body(&mut sets)).show(ui);
    if let Some(c) = out.picked {
        let _ = app.run("equation.insertSymbol", json!({"char": c.to_string()}));
        app.canvas.focus_page(ui.ctx());
    }
}

/// Append the symbol name or codepoint only when the display is the glyph itself.
fn symbol_tooltip(c: char, display: &str) -> String {
    if display != c.to_string() {
        // Missing glyphs already display their linear name or codepoint.
        return display.to_owned();
    }
    wordcraft_doc::math_symbols::name_of(c).map(|n| format!("{display}  \\{n}")).unwrap_or_else(|| format!("{display}  U+{:04X}", c as u32))
}

/// The Equation tab.
pub fn show(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    // The equation under the caret: its display mode and linear text give the Display, Inline and Number states.
    // Read, not run: a command in every frame would consume an input method's pending replacement join.
    let info = wordcraft_engine::cmd::equation::at_caret(&app.session);
    let display = info.is_some_and(|(display, _)| display);
    let numbered = info.is_some_and(|(_, linear)| linear.contains('#'));
    let latex = app.session.math_latex;
    let normal_text = app.session.math_normal_text;
    row.group(RibbonGroup::new("tools", tl!("Tools").to_string(), IconId::new("equation").into()), |ui| {
        math_menu_button(ui, EQUATION_PICTURE, "Equation", |ui, cx| builtin_menu(ui, cx, app));
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            ToggleButton::command(CommandRef::with_params("equation.inputFormat", json!({"format": "unicode"})))
                .label(tl!("Unicode").to_string())
                .icon(IconId::new("equation"))
                .tooltip(tl!("Type equations in the linear format (UnicodeMath)").to_string())
                .selected(!latex)
                .show(ui);
            ToggleButton::command(CommandRef::with_params("equation.inputFormat", json!({"format": "latex"})))
                .label(tl!("LaTeX").to_string())
                .icon(IconId::new("formula"))
                .tooltip(tl!("Type equations in LaTeX").to_string())
                .selected(latex)
                .show(ui);
            let face = Button::new(tl!("Convert").to_string()).icon(IconId::new("replace"));
            let _ = menu_button(ui, face, tl!("Convert").to_string(), None, |ui, _| {
                for (label, to, all) in [
                    ("Current - Professional", "professional", false),
                    ("Current - Linear", "linear", false),
                    ("All - Professional", "professional", true),
                    ("All - Linear", "linear", true),
                ] {
                    MenuItem::command(CommandRef::with_params("equation.convert", json!({"to": to, "all": all})))
                        .label(tl!(label).to_string())
                        .show(ui);
                }
            });
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            ToggleButton::command(CommandRef::new("equation.normalText"))
                .label(tl!("Normal Text").to_string())
                .icon(IconId::new("case"))
                .tooltip(tl!("Type normal (non-math) text in the equation").to_string())
                .selected(normal_text)
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("symbols", tl!("Symbols").to_string(), IconId::new("symbol").into()), |ui| symbols(ui, app));
    row.group(RibbonGroup::new("structures", tl!("Structures").to_string(), IconId::new("equation").into()), |ui| {
        for g in STRUCTURES {
            let id = g.id;
            math_menu_button(ui, button_picture(id), g.label, |ui, cx| structure_menu(ui, cx, app, id));
        }
    });
    row.group(RibbonGroup::new("equationOptions", tl!("Equation Options").to_string(), IconId::new("equation").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            ToggleButton::command(CommandRef::with_params("equation.display", json!({"value": true})))
                .label(tl!("Display").to_string())
                .icon(IconId::new("alignCenter"))
                .tooltip(tl!("Show the equation on a line of its own").to_string())
                .selected(display)
                .show(ui);
            ToggleButton::command(CommandRef::with_params("equation.display", json!({"value": false})))
                .label(tl!("Inline").to_string())
                .icon(IconId::new("alignLeft"))
                .tooltip(tl!("Show the equation in the line of text").to_string())
                .selected(!display)
                .show(ui);
            ToggleButton::command(CommandRef::new("equation.number"))
                .label(tl!("Number").to_string())
                .icon(IconId::new("numbering"))
                .tooltip(tl!("Number the equation; the number sits at the right margin (or type # then the number)").to_string())
                .selected(numbered)
                .show(ui);
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            let face = Button::new(tl!("Justification").to_string()).icon(IconId::new("justify"));
            let _ = menu_button(ui, face, tl!("Justification").to_string(), None, |ui, _| {
                for (label, jc) in [("Left", "left"), ("Right", "right"), ("Centered", "center"), ("Centered as Group", "centerGroup")] {
                    MenuItem::command(CommandRef::with_params("equation.justify", json!({"jc": jc}))).label(tl!(label).to_string()).show(ui);
                }
            });
            Button::command(CommandRef::new("equation.exit"))
                .label(tl!("Close Equation").to_string())
                .tooltip(tl!("Leave the equation (Esc)").to_string())
                .show(ui);
        });
    });
}

#[cfg(test)]
mod placeholder_tests {
    /// Named and unnamed missing glyphs appear once; covered glyphs retain their suffix.
    #[test]
    fn symbol_tooltips_do_not_repeat_missing_glyph_fallbacks() {
        for c in ['α', '\u{0378}'] {
            let display = super::math_placeholder_with(&c.to_string(), |_| false);
            assert_eq!(super::symbol_tooltip(c, &display), display);
        }
        assert_eq!(super::symbol_tooltip('α', "α"), "α  \\alpha");
        assert_eq!(super::symbol_tooltip('\u{0378}', "\u{0378}"), "\u{0378}  U+0378");
    }

    /// `∁` falls back to `\complement` in the placeholder while covered `x+y` is unchanged.
    #[test]
    fn a_missing_math_glyph_has_a_readable_linear_placeholder() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            assert!(!ui.fonts_mut(|fonts| fonts.has_glyph(&font, '∁')));
            assert_eq!(super::math_placeholder(ui, "x+\u{2201}"), "x+\\complement");
            assert_eq!(super::math_placeholder(ui, "x+y"), "x+y");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn a_missing_math_glyph_falls_back_to_its_linear_name() {
        // Without craft-fonts the interface fonts may have no `┬`: the Equation button's picture
        // then reads `lim\below(n→∞)`. That is still math input, not interface wording.
        let covered = |c: char| c.is_ascii() || matches!(c, '∞' | '→');
        assert_eq!(super::math_placeholder_with("lim┬(n→∞)", covered), "lim\\below(n→∞)");
        assert!(super::placeholder_linears().any(|linear| linear == "lim┬(n→∞)"));
    }
}
