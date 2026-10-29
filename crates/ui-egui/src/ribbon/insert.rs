use super::common::{mi, mi_icon, stack};
use super::language::mi_check;
use super::{Ui, WordApp, big, group, json, menu_button, small};

pub(super) fn insert(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Pages", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("coverPage", Some("Cover Page"), "Cover Page", false, "insert.coverPage"),
                |ui, app| {
                    mi(ui, app, "Studio cover", "insert.coverPage", json!({}));
                },
            );
            small(ui, app, "blankPage", Some("Blank Page"), "Blank Page", crate::widgets::ButtonAction::new("insert.blankPage", json!({}), false));
            small(ui, app, "pageBreak", Some("Page Break"), "Page Break", crate::widgets::ButtonAction::new("insert.pageBreak", json!({}), false));
        });
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
                mi(ui, app, "Tabular List", "table.quick", json!({"kind": "tabular"}));
                mi(ui, app, "Matrix", "table.quick", json!({"kind": "matrix"}));
                mi(ui, app, "Calendar", "table.quick", json!({"kind": "calendar"}));
            });
        });
    });
    group(ui, "Illustrations", None, app, |ui, app| {
        big(ui, app, "picture", "Pictures", "insert.picture", json!({}), false);
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("shapes", Some("Shapes"), "Shapes", true, "insert.shape"), |ui, app| {
            for (l, k) in [
                ("Rectangle", "rectangle"),
                ("Rounded Rectangle", "roundedRectangle"),
                ("Oval", "ellipse"),
                ("Triangle", "triangle"),
                ("Diamond", "diamond"),
                ("Line", "line"),
                ("Arrow", "arrow"),
                ("Star", "star"),
                ("Heart", "heart"),
            ] {
                mi_icon(ui, app, &format!("shape.{k}"), l, "insert.shape", json!({"kind": k}));
            }
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
        stack(ui, |ui| {
            small(ui, app, "icons", Some("Icons"), "Icons", crate::widgets::ButtonAction::new("insert.icon", json!({}), false));
            small(ui, app, "smartArt", Some("SmartArt"), "SmartArt", crate::widgets::ButtonAction::new("insert.smartArt", json!({}), false));
            small(ui, app, "chart", Some("Chart"), "Chart", crate::widgets::ButtonAction::new("insert.chart", json!({}), false));
        });
    });
    group(ui, "Links", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "link", Some("Link"), "Link", crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "link"}), false));
            small(
                ui,
                app,
                "bookmark",
                Some("Bookmark"),
                "Bookmark",
                crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "bookmark"}), false),
            );
            small(
                ui,
                app,
                "crossRef",
                Some("Cross-reference"),
                "Cross-reference",
                crate::widgets::ButtonAction::new("insert.crossReference", json!({}), false),
            );
        });
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "Comment", "review.newComment", json!({}), false);
    });
    group(ui, "Header & Footer", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("header", Some("Header"), "Header", true, "insert.header"), |ui, app| {
            mi(ui, app, "Blank", "insert.header", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.header", json!({"preset": "blankThree"}));
            mi(ui, app, "Document Title", "insert.header", json!({"preset": "title"}));
            ui.separator();
            mi(ui, app, "Edit Header", "insert.editHeader", json!({}));
            mi(ui, app, "Remove Header", "insert.removeHeader", json!({}));
        });
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("footer", Some("Footer"), "Footer", true, "insert.footer"), |ui, app| {
            mi(ui, app, "Blank", "insert.footer", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.footer", json!({"preset": "blankThree"}));
            mi(ui, app, "Page Number", "insert.footer", json!({"preset": "pageNumber"}));
            ui.separator();
            mi(ui, app, "Edit Footer", "insert.editFooter", json!({}));
            mi(ui, app, "Remove Footer", "insert.removeFooter", json!({}));
        });
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("pageNumber", Some("Page\nNumber"), "Page Number", true, "insert.pageNumber"),
            |ui, app| {
                mi(ui, app, "Top of Page", "insert.pageNumber", json!({"position": "top", "align": "right"}));
                mi(ui, app, "Bottom of Page", "insert.pageNumber", json!({"position": "bottom", "align": "center"}));
                mi(ui, app, "Page X of Y", "insert.pageNumber", json!({"position": "bottom", "format": "x of y"}));
                mi(ui, app, "Current Position", "insert.pageNumber", json!({"position": "current"}));
                mi(ui, app, "Format Page Numbers…", "ui.dialog", json!({"name": "pageNumbers"}));
            },
        );
    });
    group(ui, "Text", None, app, |ui, app| {
        big(ui, app, "textBox", "Text\nBox", "insert.textBox", json!({}), false);
        stack(ui, |ui| {
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
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("wordArt", Some("WordArt"), "WordArt", false, ""), |ui, app| {
                crate::wordart_menus::style_gallery(ui, app, "insert.wordArt")
            });
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("dropCap", Some("Drop Cap"), "Drop Cap", false, "insert.dropCap"),
                drop_cap_items,
            );
        });
        stack(ui, |ui| {
            small(ui, app, "signature", None, "Signature Line", crate::widgets::ButtonAction::new("insert.signatureLine", json!({}), false));
            small(ui, app, "dateTime", None, "Date & Time", crate::widgets::ButtonAction::new("insert.dateTime", json!({}), false));
            small(ui, app, "object", None, "Object", crate::widgets::ButtonAction::new("insert.object", json!({}), false));
        });
    });
    group(ui, "Symbols", None, app, |ui, app| {
        crate::equation_tab::insert_button(ui, app);
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("symbol", Some("Symbol"), "Symbol", true, "insert.symbol"), |ui, app| {
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
            egui::Grid::new("syms").show(ui, |ui| {
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
    });
}

/// Insert › Drop Cap's menu (#2601): None, Dropped and In margin (the current one checked),
/// then the Drop Cap dialog.
fn drop_cap_items(ui: &mut Ui, app: &mut WordApp) {
    let current = wordcraft_engine::cmd::insert::drop_cap_state(&app.session).position;
    for (label, v) in [("None", "none"), ("Dropped", "dropped"), ("In margin", "margin")] {
        mi_check(ui, app, label, current == v, "insert.dropCap", json!({"position": v}));
    }
    ui.separator();
    mi(ui, app, "Drop Cap Options…", "ui.dialog", json!({"name": "dropCap"}));
}
