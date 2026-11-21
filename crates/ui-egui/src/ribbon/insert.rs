use super::common::menu_button;
use super::{WordApp, json};
use craft::behavior::{DisplayOrder, IconId, ItemSource, SizeHint};
use craft::keymap::CommandRef;
use craft::menu::{RibbonGroup, RibbonRow};
use craft::pages::ui::TableSizePicker;
use craft::ui::buttons::{Arrangement, Button, IconButton};
use craft::ui::contract::{ActivateOn, BodyCx, CollectionEvent, Gap, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::layout_components::Stack;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::{HAlign, IconSpec, TextRun};
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Ui;

/// The symbol grid's cells: positions are their own keys.
struct SymbolCells(usize);

impl DisplayOrder<usize> for SymbolCells {
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

impl ItemSource<usize> for SymbolCells {
    fn size_hint(&self, _at: usize) -> SizeHint {
        SizeHint::Aspect(1.0)
    }
}

/// Render the Insert ribbon tab controls.
///
// PLAN: each menu button's `about` is the command its menu runs; the base's `MenuButtonOptions` ids were not in the bundle.
pub(super) fn insert(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("Pages", crate::tl!("Pages"), IconId::new("coverPage").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(
                ui,
                Button::new(crate::tl!("Cover Page")).icon(IconId::new("coverPage")).tooltip(crate::tl!("Cover Page")),
                crate::tl!("Cover Page"),
                Some("insert.coverPage"),
                |ui, _| {
                    MenuItem::command(CommandRef::with_params("insert.coverPage", json!({}))).label(crate::tl!("Studio cover")).show(ui);
                },
            );
            Button::command(CommandRef::with_params("insert.blankPage", json!({})))
                .label(crate::tl!("Blank Page"))
                .icon(IconId::new("blankPage"))
                .show(ui);
            Button::command(CommandRef::with_params("insert.pageBreak", json!({})))
                .label(crate::tl!("Page Break"))
                .icon(IconId::new("pageBreak"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Tables", crate::tl!("Tables"), IconId::new("table").into()), |ui| {
        let closed = menu_button(
            ui,
            Button::new(crate::tl!("Table")).icon(IconId::new("table")).tooltip(crate::tl!("Add a Table")).arrangement(Arrangement::Stacked),
            crate::tl!("Add a Table"),
            Some("insert.table"),
            |ui, cx| {
                let out = TableSizePicker::new(crate::tl!("Insert Table")).show(ui);
                if let Some((rows, cols)) = out.picked {
                    let _ = app.run("insert.table", json!({"rows": rows, "cols": cols}));
                    app.canvas.table_preview = None;
                    cx.request_close();
                } else {
                    // The page shows the hovered table until a square is picked.
                    app.canvas.table_preview = out.hovered;
                }
                Divider::horizontal().show(ui);
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "insertTable"}))).label(crate::tl!("Insert Table…")).show(ui);
                let drawing = app.canvas.table_tool == Some(crate::table_pen::TableTool::Draw);
                MenuItem::command(CommandRef::with_params("table.draw", json!({}))).label(crate::tl!("Draw Table")).check(drawing).show(ui);
                MenuItem::command(CommandRef::with_params("table.fromText", json!({}))).label(crate::tl!("Convert Text to Table…")).show(ui);
                Menu::submenu_row(ui, crate::tl!("Quick Tables"), |ui, _| {
                    for (label, kind) in [("Tabular List", "tabular"), ("Matrix", "matrix"), ("Calendar", "calendar")] {
                        MenuItem::command(CommandRef::with_params("table.quick", json!({"kind": kind}))).label(crate::tl!(label)).show(ui);
                    }
                });
            },
        )
        .closed;
        // A closed menu leaves no preview behind: its body no longer runs to clear it.
        if closed.is_some() {
            app.canvas.table_preview = None;
        }
    });
    row.group(RibbonGroup::new("Illustrations", crate::tl!("Illustrations"), IconId::new("picture").into()), |ui| {
        Button::command(CommandRef::with_params("insert.picture", json!({})))
            .label(crate::tl!("Pictures"))
            .icon(IconId::new("picture"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        menu_button(
            ui,
            Button::new(crate::tl!("Shapes")).icon(IconId::new("shapes")).tooltip(crate::tl!("Shapes")).arrangement(Arrangement::Stacked),
            crate::tl!("Shapes"),
            Some("insert.shape"),
            |ui, cx| {
                super::galleries::shapes(ui, cx, app);
                Divider::horizontal().show(ui);
                MenuItem::command(CommandRef::with_params("insert.canvas", json!({})))
                    .label(crate::tl!("New Drawing Canvas"))
                    .icon(IconId::new("canvas"))
                    .show(ui);
                // Connectors: glue their ends to shapes by dragging them (shape.connect).
                for (label, kind, arrow, glyph) in [
                    ("Straight Connector", "straightConnector", "none", "shape.straightConnector"),
                    ("Straight Arrow Connector", "straightConnector", "end", "shape.straightArrowConnector"),
                    ("Elbow Connector", "elbowConnector", "none", "shape.elbowConnector"),
                    ("Elbow Arrow Connector", "elbowConnector", "end", "shape.elbowArrowConnector"),
                    ("Curved Connector", "curvedConnector", "none", "shape.curvedConnector"),
                    ("Curved Arrow Connector", "curvedConnector", "end", "shape.curvedArrowConnector"),
                ] {
                    MenuItem::command(CommandRef::with_params("insert.shape", json!({"kind": kind, "arrow": arrow})))
                        .label(crate::tl!(label))
                        .icon(crate::icons::id(glyph))
                        .show(ui);
                }
                let _ = cx;
            },
        );
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (key, label, id) in
                [("icons", "Icons", "insert.icon"), ("smartArt", "Diagram", "insert.smartArt"), ("chart", "Chart", "insert.chart")]
            {
                Button::command(CommandRef::with_params(id, json!({}))).label(crate::tl!(label)).icon(crate::icons::id(key)).show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("Links", crate::tl!("Links"), IconId::new("wordcraft.link").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::with_params("ui.dialog", json!({"name": "link"})))
                .label(crate::tl!("Link"))
                .icon(IconId::new("wordcraft.link"))
                .show(ui);
            Button::command(CommandRef::with_params("ui.dialog", json!({"name": "bookmark"})))
                .label(crate::tl!("Bookmark"))
                .icon(IconId::new("wordcraft.bookmark"))
                .show(ui);
            Button::command(CommandRef::with_params("insert.crossReference", json!({})))
                .label(crate::tl!("Cross-reference"))
                .icon(IconId::new("crossRef"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Comments", crate::tl!("Comments"), IconId::new("newComment").into()), |ui| {
        Button::command(CommandRef::with_params("review.newComment", json!({})))
            .label(crate::tl!("Comment"))
            .icon(IconId::new("newComment"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("Header & Footer", crate::tl!("Header & Footer"), IconId::new("header").into()), |ui| {
        menu_button(
            ui,
            Button::new(crate::tl!("Header")).icon(IconId::new("header")).tooltip(crate::tl!("Header")).arrangement(Arrangement::Stacked),
            crate::tl!("Header"),
            Some("insert.header"),
            |ui, _| {
                for (label, preset) in [("Blank", "blank"), ("Blank (Three Columns)", "blankThree"), ("Document Title", "title")] {
                    MenuItem::command(CommandRef::with_params("insert.header", json!({"preset": preset}))).label(crate::tl!(label)).show(ui);
                }
                Divider::horizontal().show(ui);
                for (label, id) in [("Edit Header", "insert.editHeader"), ("Remove Header", "insert.removeHeader")] {
                    MenuItem::command(CommandRef::with_params(id, json!({}))).label(crate::tl!(label)).show(ui);
                }
            },
        );
        menu_button(
            ui,
            Button::new(crate::tl!("Footer")).icon(IconId::new("footer")).tooltip(crate::tl!("Footer")).arrangement(Arrangement::Stacked),
            crate::tl!("Footer"),
            Some("insert.footer"),
            |ui, _| {
                for (label, preset) in [("Blank", "blank"), ("Blank (Three Columns)", "blankThree"), ("Page Number", "pageNumber")] {
                    MenuItem::command(CommandRef::with_params("insert.footer", json!({"preset": preset}))).label(crate::tl!(label)).show(ui);
                }
                Divider::horizontal().show(ui);
                for (label, id) in [("Edit Footer", "insert.editFooter"), ("Remove Footer", "insert.removeFooter")] {
                    MenuItem::command(CommandRef::with_params(id, json!({}))).label(crate::tl!(label)).show(ui);
                }
            },
        );
        menu_button(
            ui,
            Button::new(crate::tl!("Page\nNumber"))
                .icon(IconId::new("pageNumber"))
                .tooltip(crate::tl!("Page Number"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Page Number"),
            Some("insert.pageNumber"),
            |ui, _| {
                for (label, params) in [
                    ("Top of Page", json!({"position": "top", "align": "right"})),
                    ("Bottom of Page", json!({"position": "bottom", "align": "center"})),
                    ("Page X of Y", json!({"position": "bottom", "format": "x of y"})),
                    ("Current Position", json!({"position": "current"})),
                ] {
                    MenuItem::command(CommandRef::with_params("insert.pageNumber", params)).label(crate::tl!(label)).show(ui);
                }
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "pageNumbers"})))
                    .label(crate::tl!("Format Page Numbers…"))
                    .show(ui);
            },
        );
    });
    row.group(RibbonGroup::new("Text", crate::tl!("Text"), IconId::new("textBox").into()), |ui| {
        Button::command(CommandRef::with_params("insert.textBox", json!({})))
            .label(crate::tl!("Text\nBox"))
            .icon(IconId::new("textBox"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(
                ui,
                Button::new(crate::tl!("Quick Parts")).icon(IconId::new("quickParts")).tooltip(crate::tl!("Quick Parts")),
                crate::tl!("Quick Parts"),
                Some("insert.quickParts"),
                |ui, _| {
                    // This session's building blocks, then the document's own Quick Parts.
                    let mut parts: Vec<String> = app.session.building_blocks.keys().cloned().collect();
                    let mut seen: std::collections::HashSet<String> = parts.iter().cloned().collect();
                    let glossary = app.session.doc.glossary.iter().filter(|e| e.gallery == wordcraft_doc::QUICK_PARTS_GALLERY);
                    parts.extend(glossary.filter(|e| seen.insert(e.name.clone())).map(|e| e.name.clone()));
                    for name in parts {
                        MenuItem::command(CommandRef::with_params("insert.quickParts", json!({"insert": name}))).label(name).show(ui);
                    }
                    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "field"}))).label(crate::tl!("Field…")).show(ui);
                },
            );
            menu_button(
                ui,
                Button::new(crate::tl!("Decorative Text")).icon(IconId::new("wordArt")).tooltip(crate::tl!("Decorative Text")),
                crate::tl!("Decorative Text"),
                Some("insert.wordArt"),
                |ui, cx| {
                    super::galleries::wordart(ui, cx, app, "insert.wordArt");
                },
            );
            menu_button(
                ui,
                Button::new(crate::tl!("Drop Cap")).icon(IconId::new("dropCap")).tooltip(crate::tl!("Drop Cap")),
                crate::tl!("Drop Cap"),
                Some("insert.dropCap"),
                |ui, _| drop_cap_items(ui, app),
            );
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (key, tip, id, params) in [
                ("signature", "Signature Line", "insert.signatureLine", json!({})),
                ("dateTime", "Date & Time", "ui.dialog", json!({"name": "dateTime"})),
                ("object", "Object", "insert.object", json!({})),
            ] {
                IconButton::command(IconSpec::from(crate::icons::id(key)), CommandRef::with_params(id, params))
                    .label(crate::tl!(tip))
                    .tooltip(crate::tl!(tip))
                    .show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("Symbols", crate::tl!("Symbols"), IconId::new("symbol").into()), |ui| {
        crate::equation_tab::insert_button(ui, app);
        menu_button(
            ui,
            Button::new(crate::tl!("Symbol")).icon(IconId::new("symbol")).tooltip(crate::tl!("Symbol")).arrangement(Arrangement::Stacked),
            crate::tl!("Symbol"),
            Some("insert.symbol"),
            |ui, cx| {
                symbol_grid(ui, cx, app);
                Divider::horizontal().show(ui);
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "symbol"}))).label(crate::tl!("More Symbols…")).show(ui);
            },
        );
    });
}

/// The Unicode names (Unicode Standard, UnicodeData.txt) of the characters the grid offers first. No Unicode-names
/// crate is installed, and the one on crates.io is under Unicode-DFS-2016, which the licence allowlist lacks.
const SYMBOL_NAMES: &[(char, &str)] = &[
    ('\u{A9}', "COPYRIGHT SIGN"),
    ('\u{AE}', "REGISTERED SIGN"),
    ('\u{2122}', "TRADE MARK SIGN"),
    ('\u{A7}', "SECTION SIGN"),
    ('\u{B6}', "PILCROW SIGN"),
    ('\u{20AC}', "EURO SIGN"),
    ('\u{A3}', "POUND SIGN"),
    ('\u{A5}', "YEN SIGN"),
    ('\u{B0}', "DEGREE SIGN"),
    ('\u{B1}', "PLUS-MINUS SIGN"),
    ('\u{2260}', "NOT EQUAL TO"),
    ('\u{2264}', "LESS-THAN OR EQUAL TO"),
    ('\u{2265}', "GREATER-THAN OR EQUAL TO"),
    ('\u{F7}', "DIVISION SIGN"),
    ('\u{D7}', "MULTIPLICATION SIGN"),
    ('\u{221E}', "INFINITY"),
    ('\u{B5}', "MICRO SIGN"),
    ('\u{3B1}', "GREEK SMALL LETTER ALPHA"),
    ('\u{3B2}', "GREEK SMALL LETTER BETA"),
    ('\u{3C0}', "GREEK SMALL LETTER PI"),
    ('\u{3A9}', "GREEK CAPITAL LETTER OMEGA"),
    ('\u{2211}', "N-ARY SUMMATION"),
    ('\u{221A}', "SQUARE ROOT"),
    ('\u{2192}', "RIGHTWARDS ARROW"),
    ('\u{2190}', "LEFTWARDS ARROW"),
    ('\u{2713}', "CHECK MARK"),
    ('\u{2605}', "BLACK STAR"),
    ('\u{2665}', "BLACK HEART SUIT"),
    ('\u{2014}', "EM DASH"),
    ('\u{2026}', "HORIZONTAL ELLIPSIS"),
];

/// A symbol's hover text: its Unicode name and code point (the code point alone, with its block, for a character
/// not named above), then its font when it has one.
pub(super) fn symbol_tip(glyph: &str, font: &str) -> String {
    let mut chars = glyph.chars();
    let tip = match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let name = SYMBOL_NAMES.iter().find(|(n, _)| *n == c).map(|(_, name)| *name);
            let block = crate::dialogs_insert::subset_of(c);
            match (name, block) {
                (Some(name), _) => format!("{name} (U+{:04X})", c as u32),
                (None, Some(block)) => format!("U+{:04X} ({block})", c as u32),
                (None, None) => format!("U+{:04X}", c as u32),
            }
        }
        _ => glyph.to_string(),
    };
    if font.is_empty() { tip } else { format!("{tip}, {font}") }
}

/// The recent and common characters as a compact grid; a click inserts the character, remembers it and closes
/// the menu.
fn symbol_grid(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    // Recently used symbols first (their font on hover), then common ones.
    let mut items: Vec<(String, String)> = app.ui.recent_symbols.iter().map(|r| (r.ch.clone(), r.font.clone())).collect();
    for c in [
        "©", "®", "™", "§", "¶", "€", "£", "¥", "°", "±", "≠", "≤", "≥", "÷", "×", "∞", "µ", "α", "β", "π", "Ω", "∑", "√", "→", "←", "✓", "★", "♥",
        "—", "…",
    ] {
        if !items.iter().any(|(ch, f)| ch == c && f.is_empty()) {
            items.push((c.to_string(), String::new()));
        }
    }
    items.truncate(30);
    let out = TileGrid::<usize>::new(crate::tl!("Symbols"))
        .select(SelectMode::None)
        .activate(ActivateOn::Click)
        .caption(CaptionPlace::None)
        .card_columns(6)
        // Small cells (#7165): a glyph needs no more room than the small thumbnail step.
        .thumb_size(ThumbSize::Small)
        // At most thirty symbols, all shown; the menu's body is the one scroll.
        .natural_height(true)
        .show_items(ui, &SymbolCells(items.len()), &mut |key, _| {
            let (c, font) = items.get(*key).cloned().unwrap_or_default();
            let (glyph, tip) = (c.clone(), symbol_tip(&c, &font));
            let tile = Tile::new(c).thumb(ThumbAspect::SQUARE, move |cx, rect| {
                let run = TextRun::new(glyph.as_str()).align(HAlign::Center);
                cx.with_draw(|d| {
                    d.text_on_centre_line(rect, &run);
                });
            });
            tile.tooltip(tip)
        });
    for event in &out.response.events {
        if let CollectionEvent::Activate(key) = event
            && let Some((c, font)) = items.get(*key)
        {
            let params = if font.is_empty() { json!({"char": c}) } else { json!({"char": c, "font": font}) };
            if app.run("insert.symbol", params).is_ok() {
                crate::dialogs_insert::remember_symbol(app, c, font);
                cx.request_close();
            }
        }
    }
}

/// What a Drop Cap menu choice sends: the position, and `keep` so each selected paragraph that has
/// a drop cap keeps its own lines, distance and font (None takes it all away).
pub(super) fn drop_cap_payload(position: &str) -> serde_json::Value {
    if position == "none" { json!({"position": position}) } else { json!({"position": position, "keep": true}) }
}

/// Insert › Drop Cap's menu (#2601): None, Dropped and In margin (the current one marked),
/// then the Drop Cap dialog.
fn drop_cap_items(ui: &mut Ui, app: &mut WordApp) {
    let state = wordcraft_engine::cmd::insert::drop_cap_state(&app.session);
    for (label, v) in [("None", "none"), ("Dropped", "dropped"), ("In margin", "margin")] {
        MenuItem::command(CommandRef::with_params("insert.dropCap", drop_cap_payload(v)))
            .label(crate::tl!(label))
            .radio_mark(state.position == v)
            .show(ui);
    }
    Divider::horizontal().show(ui);
    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "dropCap"}))).label(crate::tl!("Drop Cap Options…")).show(ui);
}
