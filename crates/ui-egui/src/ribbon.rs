//! The ribbon: tab strip and every tab's groups. Each control runs a command by id.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::theme::{Tokens, medium, regular, semibold};
use crate::widgets::{CONTENT_H, LABEL_H, big, color_grid, combo, group, menu_button, small, split};
use crate::{WordApp, icons};
use crate::i18n::{Language, tr};

pub const TABS: [&str; 11] = ["File", "Home", "Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Help"];

fn tl(app: &WordApp, s: &str) -> String { tr(app.ui_language(), s) }

fn in_table(app: &WordApp) -> bool {
    app.session.sel.focus.path.cell().is_some()
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // Tab strip.
    egui::Panel::top("tabs")
        .exact_size(30.0)
        .frame(egui::Frame::NONE.fill(t.tab_strip).inner_margin(egui::Margin { left: 8, right: 10, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            ui.with_layout(if app.is_rtl() { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) }, |ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                let mut tabs: Vec<&str> = TABS.to_vec();
                if in_table(app) {
                    tabs.push("Table Design");
                    tabs.push("Table Layout");
                }
                for tab in tabs {
                    let contextual = tab.starts_with("Table ");
                    let display_tab = tl(app, tab);
                    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(display_tab.clone(), medium(12.5), t.text).size().x) + 18.0;
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                    let active = app.ui.tab == tab && !app.ui.backstage;
                    if resp.hovered() && !active {
                        ui.painter().rect_filled(r.shrink2(vec2(0.0, 4.0)), 4.0, t.hover);
                    }
                    let color = if contextual || active { t.accent_text } else { t.text };
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, display_tab, if active { semibold(12.5) } else { medium(12.5) }, color);
                    if active {
                        let u = Rect::from_center_size(pos2(r.center().x, r.max.y - 2.0), vec2(w - 16.0, 3.0));
                        ui.painter().rect_filled(u, 2.0, t.accent);
                    }
                    if resp.clicked() {
                        let _ = app.run("ui.tab", json!({"tab": tab}));
                    }
                    if resp.double_clicked() && tab != "File" {
                        let _ = app.run("ui.collapseRibbon", json!({}));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Share, Editing mode, Comments.
                    let (r, resp) = ui.allocate_exact_size(vec2(74.0, 24.0), Sense::click());
                    ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.accent_text } else { t.accent });
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(pos2(r.min.x + 14.0, r.center().y), vec2(14.0, 14.0)),
                        "share",
                        egui::Color32::WHITE,
                        egui::Color32::WHITE,
                    );
                    ui.painter().text(pos2(r.min.x + 26.0, r.center().y), Align2::LEFT_CENTER, "Share", medium(12.0), egui::Color32::WHITE);
                    if resp.on_hover_text("Export a copy to share (PDF, Word document)").clicked() {
                        let _ = app.run("ui.backstage", json!({"value": true, "page": "export"}));
                    }
                    ui.add_space(6.0);
                    let track = app.session.doc.settings.track_changes;
                    ui.menu_button(egui::RichText::new(if track { format!("✎ {} ▾", tl(app, "Reviewing")) } else { format!("✎ {} ▾", tl(app, "Editing")) }).font(regular(12.0)), |ui| {
                        if ui.selectable_label(!track, tl(app, "Editing — edit the document directly")).clicked() {
                            let _ = app.run("review.trackChanges", json!({"value": false}));
                            ui.close();
                        }
                        if ui.selectable_label(track, tl(app, "Reviewing — edits become suggestions")).clicked() {
                            let _ = app.run("review.trackChanges", json!({"value": true}));
                            ui.close();
                        }
                    });
                    ui.add_space(4.0);
                    if ui.button(egui::RichText::new(format!("💬 {}", tl(app, "Comments"))).font(regular(12.0))).clicked() {
                        let _ = app.run("view.commentsPane", json!({}));
                    }
                });
            });
        });
    if app.ui.ribbon_collapsed {
        return;
    }
    egui::Panel::top("ribbon")
        .exact_size(CONTENT_H + LABEL_H + 10.0)
        .frame(
            egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 }).stroke(Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.with_layout(if app.is_rtl() { egui::Layout::right_to_left(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) }, |ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    match app.ui.tab.as_str() {
                        "Home" => home(app, ui),
                        "Insert" => insert(app, ui),
                        "Draw" => draw(app, ui),
                        "Design" => design(app, ui),
                        "Layout" => layout(app, ui),
                        "References" => references(app, ui),
                        "Mailings" => mailings(app, ui),
                        "Review" => review(app, ui),
                        "View" => view(app, ui),
                        "Help" => help(app, ui),
                        "Table Design" => table_design(app, ui),
                        "Table Layout" => table_layout(app, ui),
                        _ => home(app, ui),
                    }
                });
            });
        });
}

fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui);
    });
}

fn mi(ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: Value) {
    let label = tl(app, label);
    let sc = crate::widgets::shortcut_text(app, id);
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(sc).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        let _ = app.run(id, params);
        ui.close();
    }
}

fn home(app: &mut WordApp, ui: &mut Ui) {
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    group(ui, "Clipboard", None, app, |ui, app| {
        menu_button(ui, app, "paste", Some("Paste"), "Paste (⌘V)", true, |ui, app| {
            mi(ui, app, "Paste", "edit.paste", json!({}));
            mi(ui, app, "Keep Text Only", "edit.pasteText", json!({}));
            mi(ui, app, "מזג עיצוב", "edit.pasteMerge", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "cut", Some("Cut"), "Cut", "edit.cut", json!({}), false);
            let r = small(ui, app, "copy", Some("Copy"), "Copy", "edit.copy", json!({}), false);
            if r.clicked() {
                ui.ctx().copy_text(app.session.clipboard_text.clone());
            }
            let active = app.session.painter.is_some();
            small(ui, app, "painter", Some("Format"), "מברשת עיצוב", "edit.formatPainter", json!({}), active);
        });
    });
    group(ui, "Font", Some("format.fontDialog"), app, |ui, app| {
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                let font = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
                let fams = app.previews.families();
                let prev = app.previews.font_preview_fn();
                if let Some(f) = combo(ui, "font", 150.0, &font, &fams, Some(&*prev)) {
                    let _ = app.run("format.font", json!({"name": f}));
                }
                let size = st
                    .get("size")
                    .and_then(Value::as_f64)
                    .map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s}") })
                    .unwrap_or_default();
                let sizes: Vec<String> =
                    wordcraft_engine::cmd::format::SIZES.iter().map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).collect();
                if let Some(v) = combo(ui, "size", 52.0, &size, &sizes, None)
                    && let Ok(x) = v.trim().parse::<f64>()
                {
                    let _ = app.run("format.size", json!({"size": x}));
                }
                ui.add_space(3.0);
                small(ui, app, "grow", None, "Increase Font Size", "format.growFont", json!({}), false);
                small(ui, app, "shrink", None, "Decrease Font Size", "format.shrinkFont", json!({}), false);
                menu_button(ui, app, "case", None, "שינוי רישיות", false, |ui, app| {
                    mi(ui, app, "Sentence case.", "format.changeCase", json!({"mode": "sentence"}));
                    mi(ui, app, "lowercase", "format.changeCase", json!({"mode": "lower"}));
                    mi(ui, app, "UPPERCASE", "format.changeCase", json!({"mode": "upper"}));
                    mi(ui, app, "Capitalize Each Word", "format.changeCase", json!({"mode": "title"}));
                    mi(ui, app, "tOGGLE cASE", "format.changeCase", json!({"mode": "toggle"}));
                });
                small(ui, app, "clear", None, "נקה את כל העיצוב", "format.clear", json!({}), false);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                small(ui, app, "bold", None, "Bold", "format.bold", json!({}), flag("bold"));
                small(ui, app, "italic", None, "Italic", "format.italic", json!({}), flag("italic"));
                split(ui, app, "underline", "Underline", "format.underline", json!({}), flag("underline"), None, |ui, app| {
                    for (l, s) in [
                        ("Single", "single"),
                        ("Double", "double"),
                        ("Thick", "thick"),
                        ("Dotted", "dotted"),
                        ("Dashed", "dash"),
                        ("Dot-dash", "dotDash"),
                        ("Wave", "wave"),
                        ("מילים בלבד", "words"),
                    ] {
                        mi(ui, app, l, "format.underline", json!({"style": s}));
                    }
                });
                small(ui, app, "strike", None, "Strikethrough", "format.strikethrough", json!({}), flag("strike"));
                small(ui, app, "subscript", None, "Subscript", "format.subscript", json!({}), flag("subscript"));
                small(ui, app, "superscript", None, "Superscript", "format.superscript", json!({}), flag("superscript"));
                ui.add_space(4.0);
                menu_button(ui, app, "effects", None, "אפקטי טקסט וטיפוגרפיה", false, |ui, app| {
                    mi(ui, app, "Outline", "format.outline", json!({}));
                    mi(ui, app, "Shadow", "format.shadow", json!({}));
                    mi(ui, app, "Small Caps", "format.smallCaps", json!({}));
                    mi(ui, app, "כל האותיות גדולות", "format.allCaps", json!({}));
                    mi(ui, app, "קו חוצה כפול", "format.doubleStrikethrough", json!({}));
                });
                let hl = app.canvas.last_highlight.clone();
                split(ui, app, "highlight", "Text Highlight Color", "format.highlight", json!({"color": hl}), false, None, |ui, app| {
                    let grid = wordcraft_doc::props::Highlight::ALL;
                    egui::Grid::new("hl").spacing(vec2(3.0, 3.0)).show(ui, |ui| {
                        for (i, h) in grid.iter().enumerate().skip(1) {
                            let c = h.rgb().unwrap_or(wordcraft_doc::Rgb::WHITE);
                            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                            ui.painter().rect_filled(r, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
                            if resp.on_hover_text(h.name()).clicked() {
                                app.canvas.last_highlight = h.ooxml().to_string();
                                let _ = app.run("format.highlight", json!({"color": h.ooxml()}));
                                ui.close();
                            }
                            if i % 5 == 0 {
                                ui.end_row();
                            }
                        }
                    });
                    mi(ui, app, "ללא צבע", "format.highlight", json!({"color": "none"}));
                });
                let fc = app.canvas.last_font_color.clone();
                let sw = wordcraft_doc::Rgb::parse(&fc).map(crate::theme::c32);
                split(ui, app, "fontcolor", "Font Color", "format.color", json!({"color": fc}), false, sw, |ui, app| {
                    mi(ui, app, "Automatic", "format.color", json!({"color": "auto"}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    if let Some(hex) = color_grid(ui, &theme) {
                        app.canvas.last_font_color = hex.clone();
                        let _ = app.run("format.color", json!({"color": hex}));
                        ui.close();
                    }
                });
            });
        });
    });
    group(ui, "Paragraph", Some("para.dialog"), app, |ui, app| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
        let align = rp.as_ref().map(|r| r.align).unwrap_or_default();
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                split(ui, app, "bullets", "Bullets", "para.bullets", json!({}), false, None, |ui, app| {
                    ui.label(egui::RichText::new("ספריית תבליטים").small().weak());
                    ui.horizontal(|ui| {
                        for c in ["•", "○", "▪", "◆", "➢", "✓", "–"] {
                            if ui.button(egui::RichText::new(c).size(16.0)).clicked() {
                                let _ = app.run("para.bullets", json!({"kind": c}));
                                ui.close();
                            }
                        }
                    });
                    mi(ui, app, "None", "para.bullets", json!({"off": true}));
                });
                split(ui, app, "numbering", "Numbering", "para.numbering", json!({}), false, None, |ui, app| {
                    for (l, k) in [
                        ("1. 2. 3.", "numbered"),
                        ("1) 2) 3)", "numberedParen"),
                        ("I. II. III.", "outline"),
                        ("A. B. C.", "upperLetter"),
                        ("a) b) c)", "lowerLetter"),
                        ("i. ii. iii.", "lowerRoman"),
                    ] {
                        mi(ui, app, l, "para.numbering", json!({"kind": k}));
                    }
                    ui.separator();
                    mi(ui, app, "התחל ב־1", "para.restartNumbering", json!({}));
                    mi(ui, app, "None", "para.numbering", json!({"off": true}));
                });
                split(ui, app, "multilevel", "רשימה מרובת רמות", "para.multilevel", json!({}), false, None, |ui, app| {
                    mi(ui, app, "1. 1.1. 1.1.1.", "para.multilevel", json!({"kind": "legal"}));
                    mi(ui, app, "I. A. 1. a.", "para.multilevel", json!({"kind": "outline"}));
                    ui.separator();
                    for lv in 0..5u64 {
                        mi(ui, app, &format!("Change to Level {}", lv + 1), "para.listLevel", json!({"level": lv}));
                    }
                });
                ui.add_space(4.0);
                small(ui, app, "outdent", None, "Decrease Indent", "para.outdent", json!({}), false);
                small(ui, app, "indent", None, "Increase Indent", "para.indent", json!({}), false);
                ui.add_space(4.0);
                small(ui, app, "sort", None, "Sort", "para.sort", json!({}), false);
                let marks = app.session.view.marks;
                small(ui, app, "pilcrow", None, "הצג/הסתר ¶", "view.marks", json!({}), marks);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                use wordcraft_doc::Align as A;
                small(ui, app, "alignLeft", None, "יישור לשמאל", "para.alignLeft", json!({}), align == A::Left);
                small(ui, app, "alignCenter", None, "Center", "para.alignCenter", json!({}), align == A::Center);
                small(ui, app, "alignRight", None, "יישור לימין", "para.alignRight", json!({}), align == A::Right);
                small(ui, app, "justify", None, "Justify", "para.justify", json!({}), align == A::Justify);
                ui.add_space(4.0);
                menu_button(ui, app, "lineSpacing", None, "מרווח שורות ופסקאות", false, |ui, app| {
                    for v in [1.0, 1.15, 1.5, 2.0, 2.5, 3.0] {
                        mi(ui, app, &format!("{v}"), "para.lineSpacing", json!({"value": v}));
                    }
                    ui.separator();
                    mi(ui, app, "הוסף מרווח לפני פסקה", "para.addSpaceBefore", json!({}));
                    mi(ui, app, "הסר מרווח אחרי פסקה", "para.removeSpaceAfter", json!({}));
                    mi(ui, app, "אפשרויות מרווח שורות…", "para.dialog", json!({}));
                });
                split(ui, app, "shading", "Shading", "para.shading", json!({"color": app.canvas.last_shading.clone()}), false, None, |ui, app| {
                    mi(ui, app, "ללא צבע", "para.shading", json!({"color": null}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    if let Some(hex) = color_grid(ui, &theme) {
                        app.canvas.last_shading = hex.clone();
                        let _ = app.run("para.shading", json!({"color": hex}));
                        ui.close();
                    }
                });
                split(ui, app, "borders", "Borders", "para.borders", json!({"kind": "bottom"}), false, None, |ui, app| {
                    for (l, k) in [
                        ("גבול תחתון", "bottom"),
                        ("גבול עליון", "top"),
                        ("גבול שמאלי", "left"),
                        ("גבול ימני", "right"),
                        ("ללא גבול", "none"),
                        ("כל הגבולות", "all"),
                        ("גבולות חיצוניים", "outside"),
                        ("Inside Borders", "inside"),
                    ] {
                        mi(ui, app, l, "para.borders", json!({"kind": k}));
                    }
                    ui.separator();
                    mi(ui, app, "קו אופקי", "insert.horizontalLine", json!({}));
                });
            });
        });
    });
    group(ui, "Styles", Some("styles.pane"), app, |ui, app| {
        crate::previews::style_gallery(app, ui, &st);
    });
    group(ui, "Editing", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "find", Some("Find"), "Find", "ui.dialog", json!({"name": "find"}), false).clicked();
            small(ui, app, "replace", Some("Replace"), "Replace", "ui.dialog", json!({"name": "replace"}), false);
            menu_button(ui, app, "select", Some("Select"), "Select", false, |ui, app| {
                mi(ui, app, "בחר הכול", "select.all", json!({}));
                mi(ui, app, "בחר פסקה", "select.paragraph", json!({}));
                mi(ui, app, "בחר משפט", "select.sentence", json!({}));
            });
        });
    });
    group(ui, "Voice", None, app, |ui, app| {
        big(ui, app, "dictate", "Dictate", "tools.dictate", json!({}), false);
    });
    group(ui, "Editor", None, app, |ui, app| {
        big(ui, app, "editor", "Editor", "review.spelling", json!({}), false);
    });
}

fn insert(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Pages", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "coverPage", Some("עמוד שער"), "עמוד שער", false, |ui, app| {
                mi(ui, app, "Studio cover", "insert.coverPage", json!({}));
            });
            small(ui, app, "blankPage", Some("עמוד ריק"), "עמוד ריק", "insert.blankPage", json!({}), false);
            small(ui, app, "pageBreak", Some("מעבר עמוד"), "מעבר עמוד", "insert.pageBreak", json!({}), false);
        });
    });
    group(ui, "Tables", None, app, |ui, app| {
        menu_button(ui, app, "table", Some("Table"), "הוסף טבלה", true, |ui, app| {
            crate::dialogs::table_grid_picker(ui, app);
            ui.separator();
            mi(ui, app, "הוסף טבלה…", "ui.dialog", json!({"name": "insertTable"}));
            mi(ui, app, "המר טקסט לטבלה…", "table.fromText", json!({}));
            ui.menu_button("טבלאות מהירות", |ui| {
                mi(ui, app, "רשימה טבלאית", "table.quick", json!({"kind": "tabular"}));
                mi(ui, app, "Matrix", "table.quick", json!({"kind": "matrix"}));
                mi(ui, app, "Calendar", "table.quick", json!({"kind": "calendar"}));
            });
        });
    });
    group(ui, "Illustrations", None, app, |ui, app| {
        big(ui, app, "picture", "Pictures", "insert.picture", json!({}), false);
        menu_button(ui, app, "shapes", Some("Shapes"), "Shapes", true, |ui, app| {
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
                mi(ui, app, l, "insert.shape", json!({"kind": k}));
            }
        });
        stack(ui, |ui| {
            small(ui, app, "icons", Some("Icons"), "Icons", "insert.icon", json!({}), false);
            small(ui, app, "smartArt", Some("SmartArt"), "SmartArt", "insert.smartArt", json!({}), false);
            small(ui, app, "chart", Some("Chart"), "Chart", "insert.chart", json!({}), false);
        });
    });
    group(ui, "Links", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "link", Some("Link"), "Link", "ui.dialog", json!({"name": "link"}), false);
            small(ui, app, "bookmark", Some("Bookmark"), "Bookmark", "ui.dialog", json!({"name": "bookmark"}), false);
            small(ui, app, "crossRef", Some("Cross-reference"), "Cross-reference", "insert.crossReference", json!({}), false);
        });
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "Comment", "review.newComment", json!({}), false);
    });
    group(ui, "כותרת עליונה ותחתונה", None, app, |ui, app| {
        menu_button(ui, app, "header", Some("Header"), "Header", true, |ui, app| {
            mi(ui, app, "Blank", "insert.header", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.header", json!({"preset": "blankThree"}));
            mi(ui, app, "כותרת מסמך", "insert.header", json!({"preset": "title"}));
            ui.separator();
            mi(ui, app, "ערוך כותרת עליונה", "insert.editHeader", json!({}));
            mi(ui, app, "הסר כותרת עליונה", "insert.removeHeader", json!({}));
        });
        menu_button(ui, app, "footer", Some("Footer"), "Footer", true, |ui, app| {
            mi(ui, app, "Blank", "insert.footer", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.footer", json!({"preset": "blankThree"}));
            mi(ui, app, "Page Number", "insert.footer", json!({"preset": "pageNumber"}));
            ui.separator();
            mi(ui, app, "ערוך כותרת תחתונה", "insert.editFooter", json!({}));
            mi(ui, app, "הסר כותרת תחתונה", "insert.removeFooter", json!({}));
        });
        menu_button(ui, app, "pageNumber", Some("Page\nNumber"), "Page Number", true, |ui, app| {
            mi(ui, app, "ראש העמוד", "insert.pageNumber", json!({"position": "top", "align": "right"}));
            mi(ui, app, "תחתית העמוד", "insert.pageNumber", json!({"position": "bottom", "align": "center"}));
            mi(ui, app, "Page X of Y", "insert.pageNumber", json!({"position": "bottom", "format": "x of y"}));
            mi(ui, app, "Current Position", "insert.pageNumber", json!({"position": "current"}));
            mi(ui, app, "עצב מספרי עמודים…", "layout.pageNumberFormat", json!({}));
        });
    });
    group(ui, "Text", None, app, |ui, app| {
        big(ui, app, "textBox", "תיבת\nטקסט", "insert.textBox", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "quickParts", Some("חלקים מהירים"), "חלקים מהירים", "insert.quickParts", json!({}), false);
            small(ui, app, "wordArt", Some("WordArt"), "WordArt", "insert.wordArt", json!({}), false);
            small(ui, app, "dropCap", Some("אות פתיחה"), "אות פתיחה", "insert.dropCap", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "signature", None, "שורת חתימה", "insert.signatureLine", json!({}), false);
            small(ui, app, "dateTime", None, "תאריך ושעה", "insert.dateTime", json!({}), false);
            small(ui, app, "object", None, "Object", "insert.object", json!({}), false);
        });
    });
    group(ui, "Symbols", None, app, |ui, app| {
        big(ui, app, "equation", "Equation", "insert.equation", json!({}), false);
        menu_button(ui, app, "symbol", Some("Symbol"), "Symbol", true, |ui, app| {
            egui::Grid::new("syms").show(ui, |ui| {
                for (i, c) in [
                    "©", "®", "™", "§", "¶", "€", "£", "¥", "°", "±", "≠", "≤", "≥", "÷", "×", "∞", "µ", "α", "β", "π", "Ω", "∑", "√", "→", "←", "✓",
                    "★", "♥", "—", "…",
                ]
                .iter()
                .enumerate()
                {
                    if ui.button(egui::RichText::new(*c).size(16.0)).clicked() {
                        let _ = app.run("insert.symbol", json!({"char": c}));
                        ui.close();
                    }
                    if i % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
        });
    });
}

fn draw(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Drawing Tools", None, app, |ui, app| {
        big(ui, app, "select", "Select", "draw.select", json!({}), false);
        big(ui, app, "lasso", "Lasso", "draw.lasso", json!({}), false);
        big(ui, app, "eraser", "Eraser", "draw.eraser", json!({}), false);
        big(ui, app, "pen", "Pen", "draw.pen", json!({}), false);
        big(ui, app, "pencil", "Pencil", "draw.pencil", json!({}), false);
        big(ui, app, "highlight", "Highlighter", "draw.highlighter", json!({}), false);
    });
    group(ui, "Convert", None, app, |ui, app| {
        big(ui, app, "inkToShape", "Ink to\nShape", "draw.inkToShape", json!({}), false);
        big(ui, app, "inkToMath", "Ink to\nMath", "draw.inkToMath", json!({}), false);
    });
    group(ui, "Insert", None, app, |ui, app| {
        big(ui, app, "canvas", "Drawing\nCanvas", "insert.canvas", json!({}), false);
    });
    group(ui, "Replay", None, app, |ui, app| {
        big(ui, app, "replay", "Ink\nReplay", "draw.replay", json!({}), false);
    });
}

fn design(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Themes", None, app, |ui, app| {
        menu_button(ui, app, "themes", Some("Themes"), "Themes", true, |ui, app| {
            for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                mi(ui, app, name, "design.theme", json!({"name": name}));
            }
        });
    });
    group(ui, "עיצוב מסמך", None, app, |ui, app| {
        crate::previews::style_set_gallery(app, ui);
        stack(ui, |ui| {
            menu_button(ui, app, "colors", Some("Colors"), "Theme Colors", false, |ui, app| {
                for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                    mi(ui, app, name, "design.themeColors", json!({"name": name}));
                }
            });
            menu_button(ui, app, "fonts", Some("Fonts"), "גופני ערכת נושא", false, |ui, app| {
                for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                    mi(ui, app, &format!("{name}: {h} / {b}"), "design.themeFonts", json!({"heading": h, "body": b}));
                }
            });
        });
        stack(ui, |ui| {
            menu_button(ui, app, "paraSpacing", Some("מרווח פסקה"), "מרווח פסקה", false, |ui, app| {
                for (l, v) in [
                    ("Default", "default"),
                    ("ללא מרווח פסקה", "none"),
                    ("Compact", "compact"),
                    ("Tight", "tight"),
                    ("Open", "open"),
                    ("Relaxed", "relaxed"),
                    ("Double", "double"),
                ] {
                    mi(ui, app, l, "design.paragraphSpacing", json!({"value": v}));
                }
            });
            small(ui, app, "effectsDesign", Some("Effects"), "Effects", "design.effects", json!({}), false);
            small(ui, app, "setDefault", Some("Set as Default"), "Set as Default", "design.setDefault", json!({}), false);
        });
    });
    group(ui, "רקע עמוד", None, app, |ui, app| {
        menu_button(ui, app, "watermark", Some("Watermark"), "Watermark", true, |ui, app| {
            for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                mi(ui, app, w, "design.watermark", json!({"text": w}));
            }
            ui.separator();
            mi(ui, app, "סימן מים מותאם אישית…", "ui.dialog", json!({"name": "watermark"}));
            mi(ui, app, "הסר סימן מים", "design.watermark", json!({"remove": true}));
        });
        menu_button(ui, app, "pageColor", Some("Page\nColor"), "Page Color", true, |ui, app| {
            mi(ui, app, "ללא צבע", "design.pageColor", json!({"color": null}));
            let theme = app.session.doc.settings.theme_colors.clone();
            if let Some(hex) = color_grid(ui, &theme) {
                let _ = app.run("design.pageColor", json!({"color": hex}));
                ui.close();
            }
        });
        menu_button(ui, app, "pageBorders", Some("Page\nBorders"), "Page Borders", true, |ui, app| {
            mi(ui, app, "Box", "design.pageBorders", json!({"kind": "box"}));
            mi(ui, app, "None", "design.pageBorders", json!({"kind": "none"}));
        });
    });
}

fn layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "הגדרת עמוד", Some("ui.dialog"), app, |ui, app| {
        menu_button(ui, app, "margins", Some("Margins"), "Margins", true, |ui, app| {
            for (l, k) in [
                ("Normal  1\" all", "normal"),
                ("Narrow  0.5\" all", "narrow"),
                ("Moderate", "moderate"),
                ("Wide", "wide"),
                ("Mirrored", "mirrored"),
                ("Office 2003 Default", "office2003"),
            ] {
                mi(ui, app, l, "layout.margins", json!({"preset": k}));
            }
            mi(ui, app, "שוליים מותאמים אישית…", "ui.dialog", json!({"name": "pageSetup"}));
        });
        menu_button(ui, app, "orientation", Some("Orientation"), "Orientation", true, |ui, app| {
            mi(ui, app, "Portrait", "layout.orientation", json!({"value": "portrait"}));
            mi(ui, app, "Landscape", "layout.orientation", json!({"value": "landscape"}));
        });
        menu_button(ui, app, "size", Some("Size"), "Size", true, |ui, app| {
            for (n, w, h) in wordcraft_geom::PAPER_SIZES {
                mi(ui, app, &format!("{n}   {:.2}\" × {:.2}\"", w / 72.0, h / 72.0), "layout.size", json!({"name": n}));
            }
        });
        menu_button(ui, app, "columns", Some("Columns"), "Columns", true, |ui, app| {
            mi(ui, app, "One", "layout.columns", json!({"count": 1}));
            mi(ui, app, "Two", "layout.columns", json!({"count": 2}));
            mi(ui, app, "Three", "layout.columns", json!({"count": 3}));
            mi(ui, app, "Left", "layout.columns", json!({"preset": "left"}));
            mi(ui, app, "Right", "layout.columns", json!({"preset": "right"}));
        });
        stack(ui, |ui| {
            menu_button(ui, app, "breaks", Some("Breaks"), "Breaks", false, |ui, app| {
                ui.label(egui::RichText::new("מעברי עמוד").strong());
                mi(ui, app, "Page", "layout.break", json!({"kind": "page"}));
                mi(ui, app, "Column", "layout.break", json!({"kind": "column"}));
                mi(ui, app, "גלישת טקסט", "layout.break", json!({"kind": "textWrapping"}));
                ui.label(egui::RichText::new("מעברי מקטעים").strong());
                mi(ui, app, "העמוד הבא", "layout.break", json!({"kind": "nextPage"}));
                mi(ui, app, "Continuous", "layout.break", json!({"kind": "continuous"}));
                mi(ui, app, "Even Page", "layout.break", json!({"kind": "evenPage"}));
                mi(ui, app, "Odd Page", "layout.break", json!({"kind": "oddPage"}));
            });
            menu_button(ui, app, "lineNumbers", Some("מספרי שורות"), "מספרי שורות", false, |ui, app| {
                for (l, v) in
                    [("None", "none"), ("Continuous", "continuous"), ("התחל מחדש בכל עמוד", "restartPage"), ("התחל מחדש בכל מקטע", "restartSection")]
                {
                    mi(ui, app, l, "layout.lineNumbers", json!({"value": v}));
                }
            });
            let hy = app.session.doc.settings.auto_hyphenation;
            menu_button(ui, app, "hyphenation", Some("Hyphenation"), "Hyphenation", false, |ui, app| {
                mi(ui, app, if hy { "✓ Automatic" } else { "Automatic" }, "layout.hyphenation", json!({"value": true}));
                mi(ui, app, if hy { "None" } else { "✓ None" }, "layout.hyphenation", json!({"value": false}));
            });
        });
    });
    group(ui, "Paragraph", Some("para.dialog"), app, |ui, app| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
        let Some(rp) = rp else { return };
        egui::Grid::new("layout_para").spacing(vec2(6.0, 4.0)).show(ui, |ui| {
            ui.label(egui::RichText::new("Indent").small().strong());
            ui.label("");
            ui.label(egui::RichText::new("Spacing").small().strong());
            ui.end_row();
            let mut l = rp.indent_left / 72.0;
            let mut b = rp.space_before;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Left:").small());
                if ui.add(egui::DragValue::new(&mut l).speed(0.05).range(-11.0..=22.0).suffix("\"").max_decimals(2)).changed() {
                    let _ = app.run("para.indents", json!({"left": l * 72.0}));
                }
            });
            ui.label("");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Before:").small());
                if ui.add(egui::DragValue::new(&mut b).speed(1.0).range(0.0..=1584.0).suffix(" pt")).changed() {
                    let _ = app.run("para.spacing", json!({"before": b}));
                }
            });
            ui.end_row();
            let mut r = rp.indent_right / 72.0;
            let mut a = rp.space_after;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Right:").small());
                if ui.add(egui::DragValue::new(&mut r).speed(0.05).range(-11.0..=22.0).suffix("\"").max_decimals(2)).changed() {
                    let _ = app.run("para.indents", json!({"right": r * 72.0}));
                }
            });
            ui.label("");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("After:").small());
                if ui.add(egui::DragValue::new(&mut a).speed(1.0).range(0.0..=1584.0).suffix(" pt")).changed() {
                    let _ = app.run("para.spacing", json!({"after": a}));
                }
            });
            ui.end_row();
        });
    });
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        big(ui, app, "wrapText", "גלישת\nטקסט", "arrange.wrap", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "bringForward", Some("הבא לחזית"), "הבא לחזית", "arrange.bringForward", json!({}), false);
            small(ui, app, "sendBackward", Some("שלח לאחור"), "שלח לאחור", "arrange.sendBackward", json!({}), false);
            small(ui, app, "selectionPane", Some("חלונית בחירה"), "חלונית בחירה", "arrange.selectionPane", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "align", None, "Align", "arrange.align", json!({}), false);
            small(ui, app, "group", None, "Group", "arrange.group", json!({}), false);
            small(ui, app, "rotate", None, "Rotate", "arrange.rotate", json!({}), false);
        });
    });
}

fn references(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "תוכן העניינים", None, app, |ui, app| {
        menu_button(ui, app, "toc", Some("Table of\nContents"), "תוכן העניינים", true, |ui, app| {
            mi(ui, app, "Automatic Table 1 (3 levels)", "references.toc", json!({"levels": 3}));
            mi(ui, app, "Automatic Table 2 (2 levels)", "references.toc", json!({"levels": 2, "title": "תוכן העניינים"}));
            mi(ui, app, "הסר תוכן עניינים", "references.removeToc", json!({}));
        });
        stack(ui, |ui| {
            menu_button(ui, app, "addText", Some("הוסף טקסט"), "הוסף טקסט", false, |ui, app| {
                mi(ui, app, "Do Not Show in TOC", "references.addText", json!({"level": 0}));
                for l in 1..=3u64 {
                    mi(ui, app, &format!("Level {l}"), "references.addText", json!({"level": l}));
                }
            });
            small(ui, app, "updateTable", Some("Update Table"), "Update Table", "references.updateToc", json!({}), false);
        });
    });
    group(ui, "Footnotes", None, app, |ui, app| {
        big(ui, app, "footnote", "Insert\nFootnote", "references.footnote", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "endnote", Some("הוסף הערת סיום"), "הוסף הערת סיום", "references.endnote", json!({}), false);
            small(ui, app, "nextFootnote", Some("הערת השוליים הבאה"), "הערת השוליים הבאה", "references.nextFootnote", json!({}), false);
            small(ui, app, "showNotes", Some("הצג הערות"), "הצג הערות", "references.notes", json!({}), false);
        });
    });
    group(ui, "Research", None, app, |ui, app| {
        big(ui, app, "researcher", "Researcher", "references.researcher", json!({}), false);
    });
    group(ui, "ציטוטים וביבליוגרפיה", None, app, |ui, app| {
        big(ui, app, "citation", "Insert\nCitation", "references.citation", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "sources", Some("ניהול מקורות"), "ניהול מקורות", "references.sources", json!({}), false);
            small(ui, app, "styles", Some("Style: APA"), "Citation Style", "references.citationStyle", json!({}), false);
            small(ui, app, "bibliography", Some("Bibliography"), "Bibliography", "references.bibliography", json!({}), false);
        });
    });
    group(ui, "Captions", None, app, |ui, app| {
        big(ui, app, "caption", "Insert\nCaption", "references.caption", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "tableOfFigures",
                Some("Insert Table of Figures"),
                "Insert Table of Figures",
                "references.tableOfFigures",
                json!({}),
                false,
            );
            small(ui, app, "update", Some("Update Table"), "Update Table of Figures", "references.updateFigures", json!({}), false);
            small(ui, app, "crossRef", Some("Cross-reference"), "Cross-reference", "insert.crossReference", json!({}), false);
        });
    });
    group(ui, "Index", None, app, |ui, app| {
        big(ui, app, "markEntry", "Mark\nEntry", "references.markEntry", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "index", Some("הוסף אינדקס"), "הוסף אינדקס", "references.index", json!({}), false);
            small(ui, app, "update", Some("Update Index"), "Update Index", "references.updateIndex", json!({}), false);
        });
    });
    group(ui, "טבלת אסמכתאות", None, app, |ui, app| {
        big(ui, app, "markCitation", "Mark\nCitation", "references.markCitation", json!({}), false);
        small(ui, app, "tableOfAuthorities", None, "Insert Table of Authorities", "references.tableOfAuthorities", json!({}), false);
    });
}

fn mailings(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Create", None, app, |ui, app| {
        big(ui, app, "envelope", "Envelopes", "mailings.envelopes", json!({}), false);
        big(ui, app, "labels", "Labels", "mailings.labels", json!({}), false);
    });
    group(ui, "התחל מיזוג דואר", None, app, |ui, app| {
        big(ui, app, "mailMerge", "Start Mail\nMerge", "mailings.start", json!({}), false);
        big(ui, app, "recipients", "Select\nRecipients", "mailings.recipients", json!({}), false);
        big(ui, app, "editRecipients", "Edit\nRecipient List", "mailings.editRecipients", json!({}), false);
    });
    group(ui, "כתוב והוסף שדות", None, app, |ui, app| {
        big(ui, app, "highlightFields", "Highlight\nMerge Fields", "mailings.highlightFields", json!({}), false);
        big(ui, app, "addressBlock", "Address\nBlock", "mailings.addressBlock", json!({}), false);
        big(ui, app, "greetingLine", "Greeting\nLine", "mailings.greetingLine", json!({}), false);
        big(ui, app, "mergeField", "Insert Merge\nField", "mailings.insertField", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "rules", Some("Rules"), "Rules", "mailings.rules", json!({}), false);
            small(ui, app, "matchFields", Some("Match Fields"), "Match Fields", "mailings.matchFields", json!({}), false);
        });
    });
    group(ui, "תצוגה מקדימה של תוצאות", None, app, |ui, app| {
        big(ui, app, "preview", "Preview\nResults", "mailings.preview", json!({}), false);
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                small(ui, app, "previous", None, "הרשומה הקודמת", "mailings.previous", json!({}), false);
                small(ui, app, "next", None, "הרשומה הבאה", "mailings.next", json!({}), false);
            });
            small(ui, app, "find", Some("חפש נמען"), "חפש נמען", "mailings.findRecipient", json!({}), false);
            small(ui, app, "checkErrors", Some("בדוק שגיאות"), "בדוק שגיאות", "mailings.checkErrors", json!({}), false);
        });
    });
    group(ui, "Finish", None, app, |ui, app| {
        big(ui, app, "finish", "Finish &\nMerge", "mailings.finish", json!({}), false);
    });
}

fn review(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Proofing", None, app, |ui, app| {
        big(ui, app, "spelling", "איות ו\nדקדוק", "review.spelling", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "thesaurus", Some("Thesaurus"), "Thesaurus", "review.thesaurus", json!({}), false);
            small(ui, app, "wordCount", Some("ספירת מילים"), "ספירת מילים", "ui.dialog", json!({"name": "wordCount"}), false);
        });
    });
    group(ui, "Speech", None, app, |ui, app| {
        big(ui, app, "readAloud", "הקרא\nבקול", "review.readAloud", json!({}), false);
    });
    group(ui, "Accessibility", None, app, |ui, app| {
        big(ui, app, "accessibility", "Check\nAccessibility", "file.accessibility", json!({}), false);
    });
    group(ui, "Language", None, app, |ui, app| {
        big(ui, app, "translate", "Translate", "review.translate", json!({}), false);
        big(ui, app, "language", "Language", "review.language", json!({}), false);
    });
    group(ui, "Comments", None, app, |ui, app| {
        big(ui, app, "newComment", "New\nComment", "review.newComment", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "deleteComment", Some("Delete"), "מחק הערה", "review.deleteComment", json!({}), false);
            small(ui, app, "prevComment", Some("Previous"), "ההערה הקודמת", "review.previousComment", json!({}), false);
            small(ui, app, "nextComment", Some("Next"), "ההערה הבאה", "review.nextComment", json!({}), false);
        });
        let shown = app.session.view.comments_pane;
        big(ui, app, "showComments", "הצג\nהערות", "view.commentsPane", json!({"value": !shown}), false);
    });
    group(ui, "Tracking", None, app, |ui, app| {
        let on = app.session.doc.settings.track_changes;
        let r = big(ui, app, "trackChanges", if on { "Track\nChanges ✓" } else { "מעקב אחר\nשינויים" }, "review.trackChanges", json!({}), false);
        let _ = r;
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                "markup",
                Some(if app.session.view.show_markup { "כל הסימונים" } else { "ללא סימונים" }),
                "Display for Review",
                false,
                |ui, app| {
                    mi(ui, app, "כל הסימונים", "review.markup", json!({"value": "all"}));
                    mi(ui, app, "ללא סימונים", "review.markup", json!({"value": "noMarkup"}));
                },
            );
            small(ui, app, "reviewingPane", Some("חלונית סקירה"), "חלונית סקירה", "review.changes", json!({}), false);
        });
    });
    group(ui, "Changes", None, app, |ui, app| {
        menu_button(ui, app, "accept", Some("Accept"), "Accept", true, |ui, app| {
            mi(ui, app, "אשר שינוי זה", "review.accept", json!({}));
            mi(ui, app, "אשר את כל השינויים", "review.acceptAll", json!({}));
        });
        menu_button(ui, app, "reject", Some("Reject"), "Reject", true, |ui, app| {
            mi(ui, app, "דחה שינוי", "review.reject", json!({}));
            mi(ui, app, "דחה את כל השינויים", "review.rejectAll", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "prevChange", Some("Previous"), "השינוי הקודם", "review.previousChange", json!({}), false);
            small(ui, app, "nextChange", Some("Next"), "השינוי הבא", "review.nextChange", json!({}), false);
        });
    });
    group(ui, "Compare", None, app, |ui, app| {
        big(ui, app, "compare", "Compare", "review.compare", json!({}), false);
    });
    group(ui, "Protect", None, app, |ui, app| {
        big(ui, app, "blockAuthors", "Block\nAuthors", "review.blockAuthors", json!({}), false);
        big(ui, app, "restrict", "Restrict\nEditing", "review.restrict", json!({}), false);
    });
}

fn view(app: &mut WordApp, ui: &mut Ui) {
    let v = app.session.view.clone();
    group(ui, "Views", None, app, |ui, app| {
        big(ui, app, "readMode", "מצב\nקריאה", "view.readMode", json!({}), false);
        big(ui, app, "printLayout", "Print\nLayout", "view.printLayout", json!({}), false);
        big(ui, app, "webLayout", "פריסת\nאינטרנט", "view.webLayout", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "outline", Some("Outline"), "Outline", "view.outline", json!({}), false);
            small(ui, app, "draft", Some("Draft"), "Draft", "view.draft", json!({}), false);
        });
    });
    group(ui, "Immersive", None, app, |ui, app| {
        big(ui, app, "focus", "Focus", "view.focus", json!({}), false);
        big(ui, app, "immersive", "Immersive\nReader", "view.immersive", json!({}), false);
    });
    group(ui, "תנועת עמוד", None, app, |ui, app| {
        big(ui, app, "vertical", "Vertical", "view.vertical", json!({}), false);
        big(ui, app, "sideToSide", "Side\nto Side", "view.sideToSide", json!({}), false);
    });
    group(ui, "Show", None, app, |ui, app| {
        stack(ui, |ui| {
            let mut r = v.ruler;
            if ui.checkbox(&mut r, "Ruler").changed() {
                let _ = app.run("view.ruler", json!({"value": r}));
            }
            let mut g = v.gridlines;
            if ui.checkbox(&mut g, "Gridlines").changed() {
                let _ = app.run("view.gridlines", json!({"value": g}));
            }
            let mut n = v.nav_pane;
            if ui.checkbox(&mut n, "חלונית ניווט").changed() {
                let _ = app.run("view.navigationPane", json!({"value": n}));
            }
        });
    });
    group(ui, "Zoom", None, app, |ui, app| {
        big(ui, app, "zoom", "Zoom", "ui.dialog", json!({"name": "zoom"}), false);
        big(ui, app, "zoom100", "100%", "view.zoom100", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "onePage", Some("עמוד אחד"), "עמוד אחד", "view.onePage", json!({}), v.fit == "onePage");
            small(ui, app, "multiplePages", Some("מספר עמודים"), "מספר עמודים", "view.multiplePages", json!({}), v.multi_page);
            small(ui, app, "pageWidth", Some("Page Width"), "Page Width", "view.pageWidth", json!({}), v.fit == "pageWidth");
        });
    });
    group(ui, "Dark Mode", None, app, |ui, app| {
        big(ui, app, "darkMode", "Switch\nModes", "view.darkMode", json!({}), false);
    });
    group(ui, "Window", None, app, |ui, app| {
        big(ui, app, "newWindow", "New\nWindow", "view.newWindow", json!({}), false);
        big(ui, app, "arrangeAll", "סדר הכול", "view.arrangeAll", json!({}), false);
        big(ui, app, "split", "Split", "view.split", json!({}), false);
    });
    group(ui, "Macros", None, app, |ui, app| {
        big(ui, app, "macros", "Macros", "tools.macros", json!({}), false);
    });
}

fn help(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Help", None, app, |ui, app| {
        big(ui, app, "help", "Help", "ui.dialog", json!({"name": "about"}), false);
        big(ui, app, "discord", "Community\nDiscord", "ui.discord", json!({}), false);
    });
    group(ui, "Agents", None, app, |ui, app| {
        big(ui, app, "macros", "Commands", "ui.dialog", json!({"name": "commands"}), false);
    });
}

fn table_design(app: &mut WordApp, ui: &mut Ui) {
    let look = app.session.sel.focus.path.cell().and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look));
    group(ui, "אפשרויות סגנון טבלה", None, app, |ui, app| {
        let Some(l) = look else { return };
        egui::Grid::new("look").show(ui, |ui| {
            for (row, items) in [
                [("שורת כותרת", "headerRow", l.header_row), ("עמודה ראשונה", "firstColumn", l.first_column)],
                [("שורת סיכום", "totalRow", l.total_row), ("עמודה אחרונה", "lastColumn", l.last_column)],
                [("Banded Rows", "bandedRows", l.banded_rows), ("Banded Columns", "bandedColumns", l.banded_columns)],
            ]
            .iter()
            .enumerate()
            {
                let _ = row;
                for (label, key, val) in items {
                    let mut v = *val;
                    if ui.checkbox(&mut v, *label).changed() {
                        let _ = app.run("table.look", json!({ *key: v }));
                    }
                }
                ui.end_row();
            }
        });
    });
    group(ui, "סגנונות טבלה", None, app, |ui, app| {
        let styles: Vec<(String, String)> = app
            .session
            .doc
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && !s.hidden)
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        egui::ScrollArea::horizontal().max_width(420.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (id, name) in styles {
                    if crate::previews::table_style_tile(ui, app, &id).on_hover_text(name).clicked() {
                        let _ = app.run("table.style", json!({"style": id}));
                    }
                }
            });
        });
        stack(ui, |ui| {
            split(ui, app, "shading", "Shading", "table.shading", json!({"color": app.canvas.last_shading.clone()}), false, None, |ui, app| {
                mi(ui, app, "ללא צבע", "table.shading", json!({"color": null}));
                let theme = app.session.doc.settings.theme_colors.clone();
                if let Some(hex) = color_grid(ui, &theme) {
                    app.canvas.last_shading = hex.clone();
                    let _ = app.run("table.shading", json!({"color": hex}));
                    ui.close();
                }
            });
        });
    });
    group(ui, "Borders", None, app, |ui, app| {
        menu_button(ui, app, "borders", Some("Borders"), "Borders", true, |ui, app| {
            for (l, k) in [
                ("כל הגבולות", "all"),
                ("גבולות חיצוניים", "outside"),
                ("Inside Borders", "inside"),
                ("ללא גבול", "none"),
                ("גבול עליון", "top"),
                ("גבול תחתון", "bottom"),
            ] {
                mi(ui, app, l, "table.borders", json!({"kind": k}));
            }
        });
        big(ui, app, "borderPainter", "Border\nPainter", "table.borderPainter", json!({}), false);
    });
}

fn table_layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "selectTable", Some("Select"), "Select", false, |ui, app| {
                mi(ui, app, "בחר תא", "table.selectCell", json!({}));
                mi(ui, app, "בחר שורה", "table.selectRow", json!({}));
                mi(ui, app, "בחר טבלה", "table.selectTable", json!({}));
            });
            let g = app.session.view.gridlines;
            small(ui, app, "gridlines", Some("הצג קווי רשת"), "הצג קווי רשת", "view.gridlines", json!({}), g);
            small(ui, app, "properties", Some("Properties"), "מאפייני טבלה", "table.properties", json!({}), false);
        });
    });
    group(ui, "שורות ועמודות", None, app, |ui, app| {
        menu_button(ui, app, "deleteTable", Some("Delete"), "Delete", true, |ui, app| {
            mi(ui, app, "מחק תאים", "table.deleteCells", json!({}));
            mi(ui, app, "מחק עמודות", "table.deleteColumn", json!({}));
            mi(ui, app, "מחק שורות", "table.deleteRow", json!({}));
            mi(ui, app, "מחק טבלה", "table.deleteTable", json!({}));
        });
        big(ui, app, "insertAbove", "Insert\nAbove", "table.insertRowAbove", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "insertBelow", Some("הוסף מתחת"), "הוסף מתחת", "table.insertRowBelow", json!({}), false);
            small(ui, app, "insertLeft", Some("הוסף משמאל"), "הוסף משמאל", "table.insertColumnLeft", json!({}), false);
            small(ui, app, "insertRight", Some("הוסף מימין"), "הוסף מימין", "table.insertColumnRight", json!({}), false);
        });
    });
    group(ui, "Merge", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "merge", Some("מזג תאים"), "מזג תאים", "table.merge", json!({}), false);
            small(ui, app, "splitCells", Some("פצל תאים"), "פצל תאים", "table.split", json!({}), false);
            small(ui, app, "splitTable", Some("פצל טבלה"), "פצל טבלה", "table.splitTable", json!({}), false);
        });
    });
    group(ui, "Cell Size", None, app, |ui, app| {
        menu_button(ui, app, "autofit", Some("AutoFit"), "AutoFit", true, |ui, app| {
            mi(ui, app, "התאם תוכן אוטומטית", "table.autofit", json!({"mode": "contents"}));
            mi(ui, app, "התאם לחלון אוטומטית", "table.autofit", json!({"mode": "window"}));
            mi(ui, app, "רוחב עמודה קבוע", "table.autofit", json!({"mode": "fixed"}));
        });
        stack(ui, |ui| {
            small(ui, app, "distributeRows", Some("פזר שורות"), "פזר שורות", "table.distributeRows", json!({}), false);
            small(ui, app, "distributeCols", Some("פזר עמודות"), "פזר עמודות", "table.distributeColumns", json!({}), false);
        });
    });
    group(ui, "Alignment", None, app, |ui, app| {
        egui::Grid::new("cellalign").spacing(vec2(1.0, 1.0)).show(ui, |ui| {
            for row in [["topLeft", "topCenter", "topRight"], ["centerLeft", "center", "centerRight"], ["bottomLeft", "bottomCenter", "bottomRight"]]
            {
                for v in row {
                    small(ui, app, "cellAlign", None, v, "table.cellAlign", json!({"value": v}), false);
                }
                ui.end_row();
            }
        });
        big(ui, app, "textDirection", "כיוון\nטקסט", "table.textDirection", json!({}), false);
        big(ui, app, "cellMargins", "Cell\nMargins", "table.cellMargins", json!({}), false);
    });
    group(ui, "Data", None, app, |ui, app| {
        big(ui, app, "sort", "Sort", "table.sort", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "repeatHeader", Some("חזור על שורות כותרת"), "חזור על שורות כותרת", "table.repeatHeader", json!({}), false);
            small(ui, app, "toText", Some("המר לטקסט"), "המר לטקסט", "table.toText", json!({}), false);
            small(ui, app, "formula", Some("Formula"), "Formula", "table.formula", json!({}), false);
        });
    });
}
