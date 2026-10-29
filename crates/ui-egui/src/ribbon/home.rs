use super::common::{mi, mi_text, stack};
use super::underline::underline_menu;
use super::{Remember, Stroke, Target, Tokens, Ui, Value, WordApp, big, group, json, menu_button, small, split, vec2};

/// The Format Painter button. A click arms it for one use (blue border only); a double-click keeps
/// it on across several uses (full checked look), like Word. Esc or another click turns it off.
fn format_painter_button(ui: &mut Ui, app: &mut WordApp) {
    let sticky = matches!(app.session.painter, Some((_, _, true)));
    let armed = app.session.painter.is_some();
    let resp =
        small(ui, app, "painter", Some("Format"), "Format Painter", crate::widgets::ButtonAction::toggle("edit.formatPainter", json!({}), sticky));
    crate::widgets::describe_button(&resp, tl!("Format Painter"), ui.is_enabled(), Some(armed));
    if resp.double_clicked() {
        painter_double_click(app);
    } else if armed && !sticky {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(resp.rect, egui::CornerRadius::same(4), Stroke::new(1.0, t.accent.linear_multiply(0.5)), egui::StrokeKind::Inside);
    }
}

/// A double-click fires `clicked()` twice, so by now the painter has been armed and disarmed again.
/// Arm it once more, this time sticky.
pub(super) fn painter_double_click(app: &mut WordApp) {
    if app.session.painter.is_some() {
        let _ = app.run("edit.formatPainter", json!({}));
    }
    let _ = app.run("edit.formatPainter", json!({"sticky": true}));
}

pub(super) fn home(app: &mut WordApp, ui: &mut Ui) {
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    group(ui, "Clipboard", Some("edit.clipboardPane"), app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("paste", Some("Paste"), "Paste (⌘V)", true, "edit.paste"), |ui, app| {
            mi(ui, app, "Paste", "edit.paste", json!({}));
            mi(ui, app, "Keep Text Only", "edit.pasteText", json!({}));
            mi(ui, app, "Merge Formatting", "edit.pasteMerge", json!({}));
            mi(ui, app, "Paste Special…", "edit.pasteSpecial", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "cut", Some("Cut"), "Cut", crate::widgets::ButtonAction::new("edit.cut", json!({}), false));
            small(ui, app, "copy", Some("Copy"), "Copy", crate::widgets::ButtonAction::new("edit.copy", json!({}), false));
            format_painter_button(ui, app);
        });
    });
    group(ui, "Font", Some("format.fontDialog"), app, |ui, app| {
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                let font = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
                let font = app.previews.font_shown(&font);
                let fams = app.previews.families();
                let recent: Vec<String> = app.ui.recent_fonts.iter().map(|f| app.previews.font_shown(f)).collect();
                let prev = app.previews.font_preview_fn();
                if let Some(f) = crate::widgets::font_combo_with_recent_hint(
                    ui,
                    crate::widgets::FontComboOptions {
                        id: "font",
                        width: 150.0,
                        current: &font,
                        items: &fams,
                        recent: &recent,
                        preview: Some(&*prev),
                        tip: tl!("Font"),
                    },
                ) {
                    let _ = if f == crate::widgets::ADD_FONT {
                        app.run("font.add", json!({}))
                    } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                        app.run("font.loadSystem", json!({}))
                    } else {
                        let f = app.previews.font_stored(&f);
                        app.run("format.font", json!({"name": f}))
                    };
                }
                let size = st
                    .get("size")
                    .and_then(Value::as_f64)
                    .map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s}") })
                    .unwrap_or_default();
                let sizes: Vec<String> =
                    wordcraft_engine::cmd::format::SIZES.iter().map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).collect();
                if let Some(v) = crate::widgets::combo_hint(ui, "size", 52.0, &size, &sizes, None, tl!("Font Size"))
                    && let Ok(x) = v.trim().parse::<f64>()
                {
                    let _ = app.run("format.size", json!({"size": x}));
                }
                ui.add_space(3.0);
                small(ui, app, "grow", None, "Increase Font Size", crate::widgets::ButtonAction::new("format.growFont", json!({}), false));
                small(ui, app, "shrink", None, "Decrease Font Size", crate::widgets::ButtonAction::new("format.shrinkFont", json!({}), false));
                menu_button(ui, app, crate::widgets::MenuButtonOptions::new("case", None, "Change Case", false, "format.changeCase"), |ui, app| {
                    mi(ui, app, "Sentence case.", "format.changeCase", json!({"mode": "sentence"}));
                    mi(ui, app, "lowercase", "format.changeCase", json!({"mode": "lower"}));
                    mi(ui, app, "UPPERCASE", "format.changeCase", json!({"mode": "upper"}));
                    mi(ui, app, "Capitalize Each Word", "format.changeCase", json!({"mode": "title"}));
                    mi(ui, app, "tOGGLE cASE", "format.changeCase", json!({"mode": "toggle"}));
                });
                small(ui, app, "clear", None, "Clear All Formatting", crate::widgets::ButtonAction::new("format.clear", json!({}), false));
                small(ui, app, "phonetic", None, "Phonetic Guide", crate::widgets::ButtonAction::new("format.phonetic", json!({}), false));
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                small(ui, app, "bold", None, "Bold", crate::widgets::ButtonAction::toggle("format.bold", json!({}), flag("bold")));
                small(ui, app, "italic", None, "Italic", crate::widgets::ButtonAction::toggle("format.italic", json!({}), flag("italic")));
                split(
                    ui,
                    app,
                    "underline",
                    "Underline",
                    crate::widgets::ButtonAction::toggle("format.underline", json!({}), flag("underline")),
                    None,
                    underline_menu,
                );
                small(
                    ui,
                    app,
                    "strike",
                    None,
                    "Strikethrough",
                    crate::widgets::ButtonAction::toggle("format.strikethrough", json!({}), flag("strike")),
                );
                small(
                    ui,
                    app,
                    "subscript",
                    None,
                    "Subscript",
                    crate::widgets::ButtonAction::toggle("format.subscript", json!({}), flag("subscript")),
                );
                small(
                    ui,
                    app,
                    "superscript",
                    None,
                    "Superscript",
                    crate::widgets::ButtonAction::toggle("format.superscript", json!({}), flag("superscript")),
                );
                small(
                    ui,
                    app,
                    "charborder",
                    None,
                    "Character Border",
                    crate::widgets::ButtonAction::toggle("format.border", json!({}), flag("border")),
                );
                small(ui, app, "enclose", None, "Enclose Characters", crate::widgets::ButtonAction::new("format.enclose", json!({}), false));
                ui.add_space(4.0);
                menu_button(ui, app, crate::widgets::MenuButtonOptions::new("effects", None, "Text Effects and Typography", false, ""), |ui, app| {
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::wordart_menus::style_gallery(ui, app, "format.textEffects");
                    ui.separator();
                    ui.menu_button(tl!("Text Fill"), |ui| crate::wordart_menus::fill_menu(ui, app, &theme));
                    let outline = crate::wordart_menus::current_outline(app);
                    ui.menu_button(tl!("Text Outline"), |ui| crate::wordart_menus::outline_menu(ui, app, &theme, outline));
                    crate::wordart_menus::effects_menu(ui, app, &theme, false);
                    ui.separator();
                    mi(ui, app, "Outline", "format.outline", json!({}));
                    mi(ui, app, "Shadow", "format.shadow", json!({}));
                    mi(ui, app, "Small Caps", "format.smallCaps", json!({}));
                    mi(ui, app, "All Caps", "format.allCaps", json!({}));
                    mi(ui, app, "Double Strikethrough", "format.doubleStrikethrough", json!({}));
                });
                let hl = app.canvas.last_highlight.clone();
                let (icon, sw, color) = crate::mini_toolbar::highlight_button_appearance(&hl);
                split(
                    ui,
                    app,
                    icon,
                    "Text Highlight Color",
                    crate::widgets::ButtonAction::new("format.highlight", json!({"color": color}), false),
                    sw,
                    crate::mini_toolbar::highlight_menu,
                );
                let fc = app.canvas.last_font_color.clone();
                let sw = wordcraft_doc::Rgb::parse(&fc).map(crate::theme::c32);
                split(
                    ui,
                    app,
                    "fontcolor",
                    "Font Color",
                    crate::widgets::ButtonAction::new("format.color", json!({"color": fc}), false),
                    sw,
                    |ui, app| {
                        mi(ui, app, "Automatic", "format.color", json!({"color": "auto"}));
                        let theme = app.session.doc.settings.theme_colors.clone();
                        crate::colors::menu(ui, app, &theme, Target::cmd("format.color").remember(Remember::FontColor));
                    },
                );
            });
        });
    });
    group(ui, "Paragraph", Some("para.dialog"), app, |ui, app| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
        // The buttons show alignment as seen on the page (Align Right is a right-to-left paragraph's start).
        let align = rp.as_ref().map(|r| r.align.visual(r.bidi)).unwrap_or_default();
        let rtl = rp.as_ref().is_some_and(|r| r.bidi);
        let lists = wordcraft_engine::cmd::para::list_buttons(&app.session.doc, rp.as_ref().and_then(|r| r.numbering));
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                split(
                    ui,
                    app,
                    "bullets",
                    "Bullets",
                    crate::widgets::ButtonAction::toggle("para.bullets", json!({}), lists.bullets),
                    None,
                    |ui, app| {
                        ui.label(egui::RichText::new(tl!("Bullet Library")).small().weak());
                        ui.horizontal(|ui| {
                            for c in ["•", "○", "▪", "◆", "➢", "✓", "–"] {
                                // The interface fonts have no ➢, so its tile draws the arrowhead.
                                let label = if c == "➢" { " " } else { c };
                                let resp = ui.button(egui::RichText::new(label).size(16.0));
                                if c == "➢" {
                                    let m = resp.rect.center();
                                    let pts = [(-4.0, -5.0), (5.0, 0.0), (-4.0, 5.0), (-1.5, 0.0)].map(|(x, y)| m + vec2(x, y));
                                    ui.painter().add(egui::Shape::convex_polygon(
                                        vec![pts[0], pts[1], pts[3]],
                                        Tokens::get(ui.ctx()).text,
                                        Stroke::NONE,
                                    ));
                                    ui.painter().add(egui::Shape::convex_polygon(
                                        vec![pts[3], pts[1], pts[2]],
                                        Tokens::get(ui.ctx()).text,
                                        Stroke::NONE,
                                    ));
                                }
                                if resp.clicked() {
                                    let _ = app.run("para.bullets", json!({"kind": c}));
                                    ui.close();
                                }
                            }
                        });
                        mi(ui, app, "None", "para.bullets", json!({"off": true}));
                    },
                );
                split(
                    ui,
                    app,
                    "numbering",
                    "Numbering",
                    crate::widgets::ButtonAction::toggle("para.numbering", json!({}), lists.numbering),
                    None,
                    |ui, app| {
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
                        mi(ui, app, "Restart at 1", "para.restartNumbering", json!({}));
                        mi(ui, app, "None", "para.numbering", json!({"off": true}));
                    },
                );
                split(
                    ui,
                    app,
                    "multilevel",
                    "Multilevel List",
                    crate::widgets::ButtonAction::toggle("para.multilevel", json!({}), lists.multilevel),
                    None,
                    |ui, app| {
                        mi(ui, app, "1. 1.1. 1.1.1.", "para.multilevel", json!({"kind": "legal"}));
                        mi(ui, app, "I. A. 1. a.", "para.multilevel", json!({"kind": "outline"}));
                        ui.separator();
                        // Lists linked to the heading styles: the headings carry the numbers. Each item
                        // reads as its first three levels, number then (translated) style name.
                        for (numbers, k) in [
                            (["1", "1.1", "1.1.1"], "headings"),
                            (["Article I.", "Section 1.01", "(a)"], "article"),
                            (["I.", "A.", "1."], "outlineHeadings"),
                            (["Chapter 1", "", ""], "chapter"),
                        ] {
                            let text = numbers
                                .iter()
                                .zip(["Heading 1", "Heading 2", "Heading 3"])
                                .map(|(n, h)| if n.is_empty() { tl!(h).to_string() } else { format!("{} {}", tl!(n), tl!(h)) })
                                .collect::<Vec<_>>()
                                .join(" / ");
                            mi_text(ui, app, &text, "list.multilevelPreset", json!({"preset": k}));
                        }
                        ui.separator();
                        mi(ui, app, "Define New Multilevel List…", "list.define", json!({}));
                        ui.separator();
                        for lv in 0..5u64 {
                            mi(ui, app, &format!("Change to Level {}", lv + 1), "para.listLevel", json!({"level": lv}));
                        }
                    },
                );
                ui.add_space(4.0);
                small(ui, app, "outdent", None, "Decrease Indent", crate::widgets::ButtonAction::new("para.outdent", json!({}), false));
                small(ui, app, "indent", None, "Increase Indent", crate::widgets::ButtonAction::new("para.indent", json!({}), false));
                ui.add_space(4.0);
                small(ui, app, "sort", None, "Sort", crate::widgets::ButtonAction::new("para.sort", json!({}), false));
                let marks = app.session.view.marks;
                small(ui, app, "pilcrow", None, "Show/Hide ¶", crate::widgets::ButtonAction::toggle("view.marks", json!({}), marks));
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                use wordcraft_doc::Align as A;
                small(ui, app, "alignLeft", None, "Align Left", crate::widgets::ButtonAction::toggle("para.alignLeft", json!({}), align == A::Left));
                small(
                    ui,
                    app,
                    "alignCenter",
                    None,
                    "Center",
                    crate::widgets::ButtonAction::toggle("para.alignCenter", json!({}), align == A::Center),
                );
                small(
                    ui,
                    app,
                    "alignRight",
                    None,
                    "Align Right",
                    crate::widgets::ButtonAction::toggle("para.alignRight", json!({}), align == A::Right),
                );
                small(ui, app, "justify", None, "Justify", crate::widgets::ButtonAction::toggle("para.justify", json!({}), align == A::Justify));
                ui.add_space(4.0);
                small(ui, app, "textLtr", None, "Left-to-Right Text Direction", crate::widgets::ButtonAction::toggle("para.ltr", json!({}), !rtl));
                small(ui, app, "textRtl", None, "Right-to-Left Text Direction", crate::widgets::ButtonAction::toggle("para.rtl", json!({}), rtl));
                ui.add_space(4.0);
                menu_button(
                    ui,
                    app,
                    crate::widgets::MenuButtonOptions::new("lineSpacing", None, "Line and Paragraph Spacing", false, "para.lineSpacing"),
                    |ui, app| {
                        for v in [1.0, 1.15, 1.5, 2.0, 2.5, 3.0] {
                            mi(ui, app, &format!("{v}"), "para.lineSpacing", json!({"value": v}));
                        }
                        ui.separator();
                        mi(ui, app, "Add Space Before Paragraph", "para.addSpaceBefore", json!({}));
                        mi(ui, app, "Remove Space After Paragraph", "para.removeSpaceAfter", json!({}));
                        mi(ui, app, "Line Spacing Options…", "para.dialog", json!({}));
                    },
                );
                split(
                    ui,
                    app,
                    "shading",
                    "Shading",
                    crate::widgets::ButtonAction::new("para.shading", json!({"color": app.canvas.last_shading.clone()}), false),
                    None,
                    |ui, app| {
                        mi(ui, app, "No Color", "para.shading", json!({"color": null}));
                        let theme = app.session.doc.settings.theme_colors.clone();
                        crate::colors::menu(ui, app, &theme, Target::cmd("para.shading").remember(Remember::Shading));
                    },
                );
                split(
                    ui,
                    app,
                    "borders",
                    "Borders",
                    crate::widgets::ButtonAction::new("para.borders", json!({"kind": "bottom"}), false),
                    None,
                    |ui, app| {
                        for (l, k) in [
                            ("Bottom Border", "bottom"),
                            ("Top Border", "top"),
                            ("Left Border", "left"),
                            ("Right Border", "right"),
                            ("No Border", "none"),
                            ("All Borders", "all"),
                            ("Outside Borders", "outside"),
                            ("Inside Borders", "inside"),
                        ] {
                            mi(ui, app, l, "para.borders", json!({"kind": k}));
                        }
                        ui.separator();
                        mi(ui, app, "Horizontal Line", "insert.horizontalLine", json!({}));
                        mi(ui, app, "Borders and Shading…", "para.borders", json!({}));
                    },
                );
            });
        });
    });
    group(ui, "Styles", Some("styles.pane"), app, |ui, app| {
        crate::previews::style_gallery(app, ui, &st);
    });
    group(ui, "Editing", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("find", Some("Find"), "Find", false, "edit.find"), |ui, app| {
                // As in Word: Find searches in the Navigation pane; Advanced Find is the dialog (#712).
                mi(ui, app, "Find", "edit.find", json!({}));
                mi(ui, app, "Advanced Find…", "edit.advancedFind", json!({}));
                mi(ui, app, "Go To…", "ui.dialog", json!({"name": "goto"}));
            });
            small(ui, app, "replace", Some("Replace"), "Replace", crate::widgets::ButtonAction::new("ui.dialog", json!({"name": "replace"}), false));
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("select", Some("Select"), "Select", false, "select.all"), |ui, app| {
                mi(ui, app, "Select All", "select.all", json!({}));
                mi(ui, app, "Select Paragraph", "select.paragraph", json!({}));
                mi(ui, app, "Select Sentence", "select.sentence", json!({}));
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
