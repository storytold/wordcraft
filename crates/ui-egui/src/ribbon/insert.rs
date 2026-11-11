use super::common::{mi, mi_icon, stack, top_row};
use super::language::mi_check;
use super::{Ui, WordApp, big, group, json, menu_button, small};

/// The basic shapes of Insert › Shapes: tooltip and `insert.shape` kind.
pub(super) const SHAPE_TILES: [(&str, &str); 9] = [
    ("Rectangle", "rectangle"),
    ("Rounded Rectangle", "roundedRectangle"),
    ("Oval", "ellipse"),
    ("Triangle", "triangle"),
    ("Diamond", "diamond"),
    ("Line", "line"),
    ("Arrow", "arrow"),
    ("Star", "star"),
    ("Heart", "heart"),
];

/// Shapes per row of the Insert › Shapes grid.
const SHAPE_TILES_PER_ROW: usize = 5;

/// The basic shapes as a grid of drawn tiles (#4349), each named by its tooltip; a click inserts it.
pub(super) fn shape_gallery(ui: &mut Ui, app: &mut WordApp) {
    use crate::{icons, theme::Tokens};
    use egui::{Rect, Sense, vec2};
    let t = Tokens::get(ui.ctx());
    let enabled = crate::widgets::enabled(app, "insert.shape");
    ui.add_enabled_ui(enabled, |ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for row in SHAPE_TILES.chunks(SHAPE_TILES_PER_ROW) {
            ui.horizontal(|ui| {
                for (label, kind) in row {
                    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
                    let name = crate::tl!(label);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
                    crate::widgets::bg(ui, r, &resp, false, &t);
                    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(24.0, 24.0)), &format!("shape.{kind}"), t.icon, t.accent);
                    if resp.on_hover_text(name).clicked() {
                        crate::widgets::run_from_ribbon(app, "insert.shape", json!({"kind": kind}));
                        ui.close();
                    }
                }
            });
        }
    });
}

pub(super) fn insert(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Pages", None, app, |ui, app| {
        let cover = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("coverPage", Some("Cover Page"), "Cover Page", big_btn, "insert.coverPage"),
                |ui, app| {
                    mi(ui, app, "Studio cover", "insert.coverPage", json!({}));
                },
            );
        };
        let blank = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "blankPage", "Blank Page", "insert.blankPage", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "blankPage",
                    Some("Blank Page"),
                    "Blank Page",
                    crate::widgets::ButtonAction::new("insert.blankPage", json!({}), false),
                );
            }
        };
        let page_break = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "pageBreak", "Page Break", "insert.pageBreak", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "pageBreak",
                    Some("Page Break"),
                    "Page Break",
                    crate::widgets::ButtonAction::new("insert.pageBreak", json!({}), false),
                );
            }
        };
        // Measured: one row below 1600 px (1280 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Insert", "Pages", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| cover(ui, app, false));
                crate::widgets::row(ui, |ui| blank(ui, app, false));
                crate::widgets::row(ui, |ui| page_break(ui, app, false));
            });
        } else {
            top_row(ui, |ui| {
                cover(ui, app, true);
                blank(ui, app, true);
                page_break(ui, app, true);
            });
        }
    });
    group(ui, "Tables", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("table", Some("Table"), "Add a Table", true, "insert.table"), |ui, app| {
            crate::dialogs::table_grid_picker(ui, app);
            ui.separator();
            mi(ui, app, "Insert Table…", "ui.dialog", json!({"name": "insertTable"}));
            let drawing = app.canvas.table_tool == Some(crate::table_pen::TableTool::Draw);
            mi_check(ui, app, "Draw Table", drawing, "table.draw", json!({}));
            mi(ui, app, "Convert Text to Table…", "table.fromText", json!({}));
            ui.menu_button(tl!("Quick Tables"), |ui| {
                crate::rtl::root(ui, |ui| {
                    mi(ui, app, "Tabular List", "table.quick", json!({"kind": "tabular"}));
                    mi(ui, app, "Matrix", "table.quick", json!({"kind": "matrix"}));
                    mi(ui, app, "Calendar", "table.quick", json!({"kind": "calendar"}));
                })
            });
        });
    });
    group(ui, "Media", None, app, |ui, app| {
        let pictures = |ui: &mut Ui, app: &mut WordApp| {
            big(ui, app, "picture", "Pictures", "insert.picture", json!({}), false);
        };
        let shapes = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("shapes", Some("Shapes"), "Shapes", true, "insert.shape"), |ui, app| {
                shape_gallery(ui, app);
                ui.separator();
                mi_icon(ui, app, "canvas", "New Drawing Canvas", "insert.canvas", json!({}));
                // Connectors: glue their ends to shapes by dragging them (shape.connect).
                for (l, k, arrow) in [
                    ("Straight Connector", "straightConnector", "none"),
                    ("Straight Arrow Connector", "straightConnector", "end"),
                    ("Elbow Connector", "elbowConnector", "none"),
                    ("Elbow Arrow Connector", "elbowConnector", "end"),
                    ("Curved Connector", "curvedConnector", "none"),
                    ("Curved Arrow Connector", "curvedConnector", "end"),
                ] {
                    let icon = if arrow == "end" { k.replace("Connector", "ArrowConnector") } else { k.to_string() };
                    mi_icon(ui, app, &format!("shape.{icon}"), l, "insert.shape", json!({"kind": k, "arrow": arrow}));
                }
            });
        };
        let smalls = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "icons", Some("Icons"), "Icons", crate::widgets::ButtonAction::new("insert.icon", json!({}), false));
            small(ui, app, "smartArt", Some("SmartArt"), "SmartArt", crate::widgets::ButtonAction::new("insert.smartArt", json!({}), false));
            small(ui, app, "chart", Some("Chart"), "Chart", crate::widgets::ButtonAction::new("insert.chart", json!({}), false));
        };
        // Measured: three rows from 1100 px (880 pt) to 1900 px, one row outside.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Insert", "Media", 1) >= 3 {
            pictures(ui, app);
            shapes(ui, app);
            stack(ui, |ui| smalls(ui, app));
        } else {
            top_row(ui, |ui| {
                pictures(ui, app);
                shapes(ui, app);
                smalls(ui, app);
            });
        }
    });
    group(ui, "Links", None, app, |ui, app| {
        let link = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "link", "Link", "ui.dialog", json!({"name": "link"}), false);
            } else {
                small(ui, app, "link", Some("Link"), "Link", crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "link"}), false));
            }
        };
        let bookmark = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "bookmark", "Bookmark", "ui.dialog", json!({"name": "bookmark"}), false);
            } else {
                small(
                    ui,
                    app,
                    "bookmark",
                    Some("Bookmark"),
                    "Bookmark",
                    crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "bookmark"}), false),
                );
            }
        };
        let cross_ref = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "crossRef", "Cross-reference", "insert.crossReference", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "crossRef",
                    Some("Cross-reference"),
                    "Cross-reference",
                    crate::widgets::ButtonAction::new("insert.crossReference", json!({}), false),
                );
            }
        };
        // Measured: one row below 1600 px (1280 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Insert", "Links", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| link(ui, app, false));
                crate::widgets::row(ui, |ui| bookmark(ui, app, false));
                crate::widgets::row(ui, |ui| cross_ref(ui, app, false));
            });
        } else {
            top_row(ui, |ui| {
                link(ui, app, true);
                bookmark(ui, app, true);
                cross_ref(ui, app, true);
            });
        }
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "Comment", "review.newComment", json!({}), false);
    });
    group(ui, "Header & Footer", None, app, |ui, app| {
        let header = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("header", Some("Header"), "Header", big_btn, "insert.header"), |ui, app| {
                mi(ui, app, "Blank", "insert.header", json!({"preset": "blank"}));
                mi(ui, app, "Blank (Three Columns)", "insert.header", json!({"preset": "blankThree"}));
                mi(ui, app, "Document Title", "insert.header", json!({"preset": "title"}));
                ui.separator();
                mi(ui, app, "Edit Header", "insert.editHeader", json!({}));
                mi(ui, app, "Remove Header", "insert.removeHeader", json!({}));
            });
        };
        let footer = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("footer", Some("Footer"), "Footer", big_btn, "insert.footer"), |ui, app| {
                mi(ui, app, "Blank", "insert.footer", json!({"preset": "blank"}));
                mi(ui, app, "Blank (Three Columns)", "insert.footer", json!({"preset": "blankThree"}));
                mi(ui, app, "Page Number", "insert.footer", json!({"preset": "pageNumber"}));
                ui.separator();
                mi(ui, app, "Edit Footer", "insert.editFooter", json!({}));
                mi(ui, app, "Remove Footer", "insert.removeFooter", json!({}));
            });
        };
        let page_number = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "pageNumber",
                    Some(if big_btn { "Page\nNumber" } else { "Page Number" }),
                    "Page Number",
                    big_btn,
                    "insert.pageNumber",
                ),
                |ui, app| {
                    mi(ui, app, "Top of Page", "insert.pageNumber", json!({"position": "top", "align": "right"}));
                    mi(ui, app, "Bottom of Page", "insert.pageNumber", json!({"position": "bottom", "align": "center"}));
                    mi(ui, app, "Page X of Y", "insert.pageNumber", json!({"position": "bottom", "format": "x of y"}));
                    mi(ui, app, "Current Position", "insert.pageNumber", json!({"position": "current"}));
                    mi(ui, app, "Format Page Numbers…", "ui.dialog", json!({"name": "pageNumbers"}));
                },
            );
        };
        // Measured: three rows from 1161 px (928.8 pt) to 1900 px, one row outside.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Insert", "Header & Footer", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| header(ui, app, false));
                crate::widgets::row(ui, |ui| footer(ui, app, false));
                crate::widgets::row(ui, |ui| page_number(ui, app, false));
            });
        } else {
            top_row(ui, |ui| {
                header(ui, app, true);
                footer(ui, app, true);
                page_number(ui, app, true);
            });
        }
    });
    group(ui, "Text", None, app, |ui, app| {
        let quick_parts = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("quickParts", Some("Quick Parts"), "Quick Parts", false, "insert.quickParts"),
                |ui, app| {
                    // This session's building blocks, then the document's own Quick Parts.
                    let mut parts: Vec<String> = app.session.building_blocks.keys().cloned().collect();
                    let mut seen: std::collections::HashSet<String> = parts.iter().cloned().collect();
                    let glossary = app.session.doc.glossary.iter().filter(|e| e.gallery == wordcraft_doc::QUICK_PARTS_GALLERY);
                    parts.extend(glossary.filter(|e| seen.insert(e.name.clone())).map(|e| e.name.clone()));
                    for name in parts {
                        if ui.button(&name).clicked() {
                            let _ = app.run("insert.quickParts", json!({"insert": name}));
                            ui.close();
                        }
                    }
                    mi(ui, app, "Field…", "ui.dialog", json!({"name": "field"}));
                },
            );
        };
        let word_art = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("wordArt", Some("WordArt"), "WordArt", false, ""), |ui, app| {
                crate::wordart_menus::style_gallery(ui, app, "insert.wordArt")
            });
        };
        let drop_cap = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("dropCap", Some("Drop Cap"), "Drop Cap", false, "insert.dropCap"),
                drop_cap_items,
            );
        };
        let extras = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "signature", None, "Signature Line", crate::widgets::ButtonAction::new("insert.signatureLine", json!({}), false));
            small(ui, app, "dateTime", None, "Date & Time", crate::widgets::ButtonAction::new("insert.dateTime", json!({}), false));
            small(ui, app, "object", None, "Object", crate::widgets::ButtonAction::new("insert.object", json!({}), false));
        };
        // Measured: one row below 1300 px (1040 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Insert", "Text", 1) >= 3 {
            big(ui, app, "textBox", "Text\nBox", "insert.textBox", json!({}), false);
            stack(ui, |ui| {
                quick_parts(ui, app);
                word_art(ui, app);
                drop_cap(ui, app);
            });
            stack(ui, |ui| extras(ui, app));
        } else {
            top_row(ui, |ui| {
                big(ui, app, "textBox", "Text\nBox", "insert.textBox", json!({}), false);
                quick_parts(ui, app);
                word_art(ui, app);
                drop_cap(ui, app);
                extras(ui, app);
            });
        }
    });
    group(ui, "Symbols", None, app, |ui, app| {
        let symbol = |ui: &mut Ui, app: &mut WordApp, big: bool| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("symbol", Some("Symbol"), "Symbol", big, "insert.symbol"), |ui, app| {
                // Recently used symbols first (their font on hover), then common ones.
                let mut items: Vec<(String, String)> = app.ui.recent_symbols.iter().map(|r| (r.ch.clone(), r.font.clone())).collect();
                for c in [
                    "©", "®", "™", "§", "¶", "€", "£", "¥", "°", "±", "≠", "≤", "≥", "÷", "×", "∞", "µ", "α", "β", "π", "Ω", "∑", "√", "→", "←", "✓",
                    "★", "♥", "—", "…",
                ] {
                    if !items.iter().any(|(ch, f)| ch == c && f.is_empty()) {
                        items.push((c.to_string(), String::new()));
                    }
                }
                items.truncate(30);
                crate::rtl::grid(ui, egui::Grid::new("syms"), |ui| {
                    for (i, (c, font)) in items.iter().enumerate() {
                        let b = ui.button(egui::RichText::new(c).size(16.0));
                        let b = if font.is_empty() { b } else { b.on_hover_text(font) };
                        if b.clicked() {
                            let params = if font.is_empty() { json!({"char": c}) } else { json!({"char": c, "font": font}) };
                            if app.run("insert.symbol", params).is_ok() {
                                crate::dialogs_insert::remember_symbol(app, c, font);
                            }
                            ui.close();
                        }
                        if i % 6 == 5 {
                            ui.end_row();
                        }
                    }
                });
                ui.separator();
                mi(ui, app, "More Symbols…", "ui.dialog", json!({"name": "symbol"}));
            });
        };
        // Measured: Symbols is one row below 1120 pt and two rows from there (1400 px at 125 %).
        if crate::ribbon_fit::rows_for(ui.ctx(), "Insert", "Symbols", 1) >= 2 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| {
                    small(ui, app, "equation", Some("Equation"), "Equation", crate::widgets::ButtonAction::new("insert.equation", json!({}), false));
                });
                crate::widgets::row(ui, |ui| symbol(ui, app, false));
            });
        } else {
            crate::equation_tab::insert_button(ui, app);
            symbol(ui, app, true);
        }
    });
}

/// What a Drop Cap menu choice sends: the position, and `keep` so each selected paragraph that has
/// a drop cap keeps its own lines, distance and font (None takes it all away).
pub(super) fn drop_cap_payload(position: &str) -> serde_json::Value {
    if position == "none" { json!({"position": position}) } else { json!({"position": position, "keep": true}) }
}

/// Insert › Drop Cap's menu (#2601): None, Dropped and In margin (the current one checked),
/// then the Drop Cap dialog.
fn drop_cap_items(ui: &mut Ui, app: &mut WordApp) {
    let state = wordcraft_engine::cmd::insert::drop_cap_state(&app.session);
    for (label, v) in [("None", "none"), ("Dropped", "dropped"), ("In margin", "margin")] {
        mi_check(ui, app, label, state.position == v, "insert.dropCap", drop_cap_payload(v));
    }
    ui.separator();
    mi(ui, app, "Drop Cap Options…", "ui.dialog", json!({"name": "dropCap"}));
}
