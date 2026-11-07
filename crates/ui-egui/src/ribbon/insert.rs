use super::common::{mi, stack};
use super::language::mi_check;
use super::{Ui, WordApp, big, group, json, menu_button, small};
use egui::{Pos2, Stroke};
use wordcraft_doc::para::ShapeKind;

use crate::theme::{Tokens, regular};

/// The basic shapes of Insert › Shapes: the labels the gallery gives them, for screen readers
/// and the grid test (each is a [`ShapeKind::label`]).
#[cfg(test)]
pub(super) const SHAPE_TILES: [(&str, &str); 9] = [
    ("Rectangle", "rectangle"),
    ("Rounded Rectangle", "roundedRectangle"),
    ("Oval", "ellipse"),
    ("Isosceles Triangle", "triangle"),
    ("Diamond", "diamond"),
    ("Line", "line"),
    ("Arrow", "arrow"),
    ("5-Point Star", "star"),
    ("Heart", "heart"),
];

/// Insert › Shapes (#4349): Word's gallery, a section per family, each shape a drawn tile. A
/// click arms the shape tool (`ui.shapeTool`): the next drag on the page draws it at that size,
/// and a plain click drops it at a default size (as Word's crosshair does).
pub(super) fn shape_gallery(ui: &mut Ui, app: &mut WordApp) {
    ui.set_width(260.0);
    egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
        shape_section(
            ui,
            app,
            "Recently Used Shapes",
            &[
                ShapeKind::Rectangle,
                ShapeKind::RoundedRectangle,
                ShapeKind::Ellipse,
                ShapeKind::Line,
                ShapeKind::Arrow,
                ShapeKind::Triangle,
                ShapeKind::Diamond,
                ShapeKind::Heart,
                ShapeKind::Star,
            ],
        );
        shape_section(
            ui,
            app,
            "Lines",
            &[
                ShapeKind::Line,
                ShapeKind::Arrow,
                ShapeKind::DoubleArrow,
                ShapeKind::StraightConnector,
                ShapeKind::ElbowConnector,
                ShapeKind::ElbowArrow,
                ShapeKind::ElbowDoubleArrow,
                ShapeKind::CurvedConnector,
                ShapeKind::CurvedArrow,
                ShapeKind::CurvedDoubleArrow,
                ShapeKind::Curve,
                ShapeKind::Freeform,
                ShapeKind::Scribble,
            ],
        );
        shape_section(
            ui,
            app,
            "Rectangles",
            &[
                ShapeKind::Rectangle,
                ShapeKind::RoundedRectangle,
                ShapeKind::SingleCornerSnipped,
                ShapeKind::TopCornersSnipped,
                ShapeKind::DiagonalCornersSnipped,
                ShapeKind::SnippedAndRounded,
                ShapeKind::RoundSingleCorner,
                ShapeKind::RoundSameSideCorner,
                ShapeKind::RoundDiagonalCorner,
            ],
        );
        shape_section(
            ui,
            app,
            "Basic Shapes",
            &[
                ShapeKind::TextBox,
                ShapeKind::Ellipse,
                ShapeKind::Triangle,
                ShapeKind::RightTriangle,
                ShapeKind::Parallelogram,
                ShapeKind::Trapezoid,
                ShapeKind::Diamond,
                ShapeKind::Pentagon,
                ShapeKind::Hexagon,
                ShapeKind::Heptagon,
                ShapeKind::Octagon,
                ShapeKind::Decagon,
                ShapeKind::Dodecagon,
                ShapeKind::Pie,
                ShapeKind::Chord,
                ShapeKind::Teardrop,
                ShapeKind::Frame,
                ShapeKind::HalfFrame,
                ShapeKind::LShape,
                ShapeKind::DiagonalStripe,
                ShapeKind::Cross,
                ShapeKind::Plaque,
                ShapeKind::Cylinder,
                ShapeKind::Cube,
                ShapeKind::Bevel,
                ShapeKind::Donut,
                ShapeKind::NoSymbol,
                ShapeKind::BlockArc,
                ShapeKind::FoldedCorner,
                ShapeKind::SmileyFace,
                ShapeKind::Heart,
                ShapeKind::LightningBolt,
                ShapeKind::Sun,
                ShapeKind::Moon,
                ShapeKind::Cloud,
                ShapeKind::Arc,
                ShapeKind::DoubleBracket,
                ShapeKind::DoubleBrace,
                ShapeKind::LeftBracket,
                ShapeKind::RightBracket,
                ShapeKind::LeftBrace,
                ShapeKind::RightBrace,
            ],
        );
        shape_section(
            ui,
            app,
            "Block Arrows",
            &[
                ShapeKind::Arrow,
                ShapeKind::ArrowLeft,
                ShapeKind::ArrowUp,
                ShapeKind::ArrowDown,
                ShapeKind::LeftRightArrow,
                ShapeKind::UpDownArrow,
                ShapeKind::QuadArrow,
                ShapeKind::LeftRightUpArrow,
                ShapeKind::BentArrow,
                ShapeKind::UTurnArrow,
                ShapeKind::LeftUpArrow,
                ShapeKind::BentUpArrow,
                ShapeKind::StripedRightArrow,
                ShapeKind::NotchedRightArrow,
                ShapeKind::PentagonArrow,
                ShapeKind::Chevron,
                ShapeKind::RightArrowCallout,
                ShapeKind::DownArrowCallout,
                ShapeKind::LeftArrowCallout,
                ShapeKind::UpArrowCallout,
                ShapeKind::LeftRightArrowCallout,
                ShapeKind::QuadArrowCallout,
                ShapeKind::CircularArrow,
            ],
        );
        shape_section(
            ui,
            app,
            "Equation Shapes",
            &[
                ShapeKind::MathPlus,
                ShapeKind::MathMinus,
                ShapeKind::MathMultiply,
                ShapeKind::MathDivide,
                ShapeKind::MathEqual,
                ShapeKind::MathNotEqual,
            ],
        );
        shape_section(
            ui,
            app,
            "Flowchart",
            &[
                ShapeKind::FlowchartProcess,
                ShapeKind::FlowchartAlternateProcess,
                ShapeKind::FlowchartDecision,
                ShapeKind::FlowchartData,
                ShapeKind::FlowchartPredefinedProcess,
                ShapeKind::FlowchartInternalStorage,
                ShapeKind::FlowchartDocument,
                ShapeKind::FlowchartMultidocument,
                ShapeKind::FlowchartTerminator,
                ShapeKind::FlowchartPreparation,
                ShapeKind::FlowchartManualInput,
                ShapeKind::FlowchartManualOperation,
                ShapeKind::FlowchartConnector,
                ShapeKind::FlowchartOffpageConnector,
                ShapeKind::FlowchartCard,
                ShapeKind::FlowchartPunchedTape,
                ShapeKind::FlowchartSummingJunction,
                ShapeKind::FlowchartOr,
                ShapeKind::FlowchartCollate,
                ShapeKind::FlowchartSort,
                ShapeKind::FlowchartExtract,
                ShapeKind::FlowchartMerge,
                ShapeKind::FlowchartStoredData,
                ShapeKind::FlowchartDelay,
                ShapeKind::FlowchartSequentialData,
                ShapeKind::FlowchartMagneticDisk,
                ShapeKind::FlowchartDirectAccessStorage,
                ShapeKind::FlowchartDisplay,
            ],
        );
        shape_section(
            ui,
            app,
            "Stars and Banners",
            &[
                ShapeKind::Explosion1,
                ShapeKind::Explosion2,
                ShapeKind::Star4,
                ShapeKind::Star,
                ShapeKind::Star6,
                ShapeKind::Star7,
                ShapeKind::Star8,
                ShapeKind::Star10,
                ShapeKind::Star12,
                ShapeKind::Star16,
                ShapeKind::Star24,
                ShapeKind::Star32,
                ShapeKind::UpRibbon,
                ShapeKind::DownRibbon,
                ShapeKind::CurvedUpRibbon,
                ShapeKind::CurvedDownRibbon,
                ShapeKind::VerticalScroll,
                ShapeKind::HorizontalScroll,
                ShapeKind::Wave,
                ShapeKind::DoubleWave,
            ],
        );
        shape_section(
            ui,
            app,
            "Callouts",
            &[
                ShapeKind::RectangularCallout,
                ShapeKind::RoundedRectangularCallout,
                ShapeKind::OvalCallout,
                ShapeKind::CloudCallout,
                ShapeKind::LineCallout1,
                ShapeKind::LineCallout2,
                ShapeKind::LineCallout3,
                ShapeKind::LineCallout4,
                ShapeKind::LineCallout1AccentBar,
                ShapeKind::LineCallout2AccentBar,
                ShapeKind::LineCallout3AccentBar,
                ShapeKind::LineCallout4AccentBar,
            ],
        );
        ui.separator();
        super::common::mi_icon(ui, app, "canvas", "New Drawing Canvas", "insert.canvas", json!({}));
    });
}

/// One gallery section: its title, then its shapes as a ten-wide grid of drawn tiles.
fn shape_section(ui: &mut Ui, app: &mut WordApp, title: &str, shapes: &[ShapeKind]) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(tl!(title)).font(regular(11.0)).color(t.text_dim));
    egui::Grid::new(title).spacing(egui::vec2(2.0, 2.0)).show(ui, |ui| {
        for (i, &k) in shapes.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
            let name = tl!(k.label());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
            if resp.hovered() {
                ui.painter().rect_filled(r, 2.0, t.hover);
            }
            let stroke_c = if resp.hovered() { t.accent_text } else { t.text };
            let inner = r.shrink(4.0);
            for (closed, pts) in wordcraft_render::shape_preview_contours(k, inner.width(), inner.height()) {
                let screen_pts: Vec<Pos2> = pts.into_iter().map(|(x, y)| egui::pos2(inner.min.x + x, inner.min.y + y)).collect();
                if closed {
                    ui.painter().add(egui::Shape::closed_line(screen_pts, Stroke::new(1.0, stroke_c)));
                } else {
                    ui.painter().add(egui::Shape::line(screen_pts, Stroke::new(1.2, stroke_c)));
                }
            }
            if resp.on_hover_text(name).clicked() {
                // Arms the shape tool: the next canvas drag draws it.
                let kind_val = serde_json::to_value(k).unwrap_or_else(|_| json!("rectangle"));
                let _ = app.run("ui.shapeTool", json!({ "kind": kind_val }));
                ui.close();
            }
            if i % 10 == 9 {
                ui.end_row();
            }
        }
    });
}

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
        crate::widgets::menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("shapes", Some("Shapes"), "Shapes", true, "insert.shape"),
            shape_gallery,
        );
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
