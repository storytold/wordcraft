//! The ribbon: tab strip and every tab's groups. Each control runs a command by id.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::theme::{Tokens, medium, regular, semibold};
use crate::widgets::{CONTENT_H, LABEL_H, big, color_grid, combo, group, menu_button, small, split};
use crate::{WordApp, icons};

pub const TABS: [&str; 12] = ["File", "Home", "Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Zotero", "Help"];

/// True when the caret/selection touches a picture (#147).
pub fn has_picture_selected(s: &wordcraft_engine::Session) -> bool {
    matches!(wordcraft_engine::cmd::objects::selected(s), Some((_, wordcraft_doc::para::InlineObject::Image { .. })))
}

/// Whether the selection is a shape or text box (so Shape Format is shown).
pub fn has_shape_selected(s: &wordcraft_engine::Session) -> bool {
    matches!(wordcraft_engine::cmd::objects::selected(s), Some((_, wordcraft_doc::para::InlineObject::Shape { .. })))
}

/// Contextual tabs for the current selection (pure, tested).
pub fn contextual_tabs(s: &wordcraft_engine::Session) -> Vec<&'static str> {
    let mut tabs = Vec::new();
    if s.sel.focus.path.cell().is_some() {
        tabs.push("Table Design");
        tabs.push("Table Layout");
    }
    if has_picture_selected(s) {
        tabs.push("Picture Format");
    }
    if has_shape_selected(s) {
        tabs.push("Shape Format");
    }
    // Editing an equation.
    if s.math.is_some() {
        tabs.push("Equation");
    }
    tabs
}

/// Tab to show when the stored tab is no longer applicable (pure, tested).
pub fn resolve_tab<'a>(current: &'a str, available: &[&str]) -> &'a str {
    if available.contains(&current) { current } else { "Home" }
}

/// Whether the caret is inside a table (so the contextual tabs and their badges are shown).
pub fn in_table_public(app: &WordApp) -> bool {
    app.session.sel.focus.path.cell().is_some()
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // Editing an equation brings up the Equation tab; leaving it goes back.
    if app.session.math.is_some() {
        if app.equation_prev_tab.is_none() {
            app.equation_prev_tab = Some(app.ui.tab.clone());
            app.ui.tab = "Equation".into();
        }
    } else if let Some(prev) = app.equation_prev_tab.take()
        && app.ui.tab == "Equation"
    {
        app.ui.tab = prev;
    }
    // A contextual tab (Table, Picture Format) may be stored while the selection moved away.
    {
        let mut tabs: Vec<&str> = TABS.to_vec();
        tabs.extend(contextual_tabs(&app.session));
        let next = resolve_tab(&app.ui.tab, &tabs);
        if next != app.ui.tab {
            app.ui.tab = next.to_string();
        }
    }
    // Tab strip.
    egui::Panel::top("tabs")
        .exact_size(30.0)
        .frame(egui::Frame::NONE.fill(t.tab_strip).inner_margin(egui::Margin { left: 8, right: 10, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                let mut tabs: Vec<&str> = TABS.to_vec();
                for ct in contextual_tabs(&app.session) {
                    if !tabs.contains(&ct) {
                        tabs.push(ct);
                    }
                }
                for tab in tabs {
                    let contextual = tab.starts_with("Table ") || tab.ends_with(" Format") || tab == "Equation";
                    let shown = tl!(tab);
                    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(shown.to_string(), medium(12.5), t.text).size().x) + 18.0;
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                    if app.ui.keytips == crate::keytips::Phase::Tabs {
                        crate::keytips::record_tab(r, tab, &mut app.keytip_rects);
                    }
                    let active = app.ui.tab == tab && !app.ui.backstage;
                    if resp.hovered() && !active {
                        ui.painter().rect_filled(r.shrink2(vec2(0.0, 4.0)), 4.0, t.hover);
                    }
                    let color = if contextual || active { t.accent_text } else { t.text };
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, shown, if active { semibold(12.5) } else { medium(12.5) }, color);
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
                    let share = tl!("Share");
                    let share_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(share.to_string(), medium(12.0), t.text).size().x);
                    let (r, resp) = ui.allocate_exact_size(vec2((share_w + 36.0).max(74.0), 24.0), Sense::click());
                    ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.accent_text } else { t.accent });
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(pos2(r.min.x + 14.0, r.center().y), vec2(14.0, 14.0)),
                        "share",
                        egui::Color32::WHITE,
                        egui::Color32::WHITE,
                    );
                    ui.painter().text(pos2(r.min.x + 26.0, r.center().y), Align2::LEFT_CENTER, share, medium(12.0), egui::Color32::WHITE);
                    if resp.on_hover_text(tl!("Export a copy to share (PDF, Word document)")).clicked() {
                        let _ = app.run("ui.backstage", json!({"value": true, "page": "export"}));
                    }
                    ui.add_space(6.0);
                    let track = app.session.doc.settings.track_changes;
                    ui.menu_button(
                        egui::RichText::new(format!("✎ {} ▾", if track { tl!("Reviewing") } else { tl!("Editing") })).font(regular(12.0)),
                        |ui| {
                            if ui.selectable_label(!track, tl!(tl!("Editing — edit the document directly"))).clicked() {
                                let _ = app.run("review.trackChanges", json!({"value": false}));
                                ui.close();
                            }
                            if ui.selectable_label(track, tl!(tl!("Reviewing — edits become suggestions"))).clicked() {
                                let _ = app.run("review.trackChanges", json!({"value": true}));
                                ui.close();
                            }
                        },
                    );
                    ui.add_space(4.0);
                    if ui.button(egui::RichText::new(format!("💬 {}", tl!("Comments"))).font(regular(12.0))).clicked() {
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
            let scroll = egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal_top(|ui| {
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
                        "Zotero" => zotero(app, ui),
                        "Help" => help(app, ui),
                        "Table Design" => table_design(app, ui),
                        "Table Layout" => table_layout(app, ui),
                        "Equation" if app.session.math.is_some() => crate::equation_tab::show(app, ui),
                        "Picture Format" => picture_format(app, ui),
                        "Shape Format" => shape_format(app, ui),
                        _ => home(app, ui),
                    }
                });
            });
            // The bar is hidden, so a row wider than the window would be clipped with no sign. Edge chevrons show
            // where more is and scroll it on click. Every command also stays reachable through Search Commands.
            let r = scroll.inner_rect;
            let overflow = scroll.content_size.x - r.width();
            let offset = scroll.state.offset.x;
            let step = 160.0;
            let mut next = None;
            if offset > 1.0 {
                let left = Rect::from_min_max(r.left_top(), pos2(r.left() + 18.0, r.bottom()));
                if edge_cue(ui, left, "‹", tl!("Scroll ribbon left"), &t) {
                    next = Some(offset - step);
                }
            }
            if overflow - offset > 1.0 {
                let right = Rect::from_min_max(pos2(r.right() - 18.0, r.top()), r.right_bottom());
                if edge_cue(ui, right, "›", tl!("Scroll ribbon right"), &t) {
                    next = Some(offset + step);
                }
            }
            if let Some(x) = next {
                let mut state = scroll.state;
                state.offset.x = x.clamp(0.0, overflow.max(0.0));
                state.store(ui.ctx(), scroll.id);
            }
        });
}

/// A chevron over a ribbon edge that is also a button; `true` on the frame it is clicked.
fn edge_cue(ui: &mut Ui, rect: Rect, glyph: &str, hint: &str, t: &Tokens) -> bool {
    let resp = ui.interact(rect, ui.id().with(glyph), Sense::click()).on_hover_text(hint);
    let ink = if resp.hovered() { t.text } else { t.text_dim };
    ui.painter().rect_filled(rect, 0.0, t.ribbon);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, egui::FontId::proportional(18.0), ink);
    resp.clicked()
}

fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui);
    });
}

fn mi(ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: Value) {
    let sc = crate::widgets::shortcut_text(app, id);
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new(tl!(label)).shortcut_text(sc).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        let _ = app.run(id, params);
        ui.close();
    }
}

/// A menu item with a check mark when `checked` (the current choice of several).
fn mi_check(ui: &mut Ui, app: &mut WordApp, label: &str, checked: bool, id: &str, params: Value) {
    // An invisible mark keeps the unchecked labels aligned with the checked one.
    let mark = egui::RichText::new("✓");
    let mark = if checked { mark } else { mark.color(egui::Color32::TRANSPARENT) };
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new((mark, tl!(label))).selected(checked).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        let _ = app.run(id, params);
        ui.close();
    }
}

fn home(app: &mut WordApp, ui: &mut Ui) {
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    group(ui, "Clipboard", Some("edit.clipboardPane"), app, |ui, app| {
        menu_button(ui, app, "paste", Some("Paste"), "Paste (⌘V)", true, |ui, app| {
            mi(ui, app, "Paste", "edit.paste", json!({}));
            mi(ui, app, "Keep Text Only", "edit.pasteText", json!({}));
            mi(ui, app, "Merge Formatting", "edit.pasteMerge", json!({}));
            mi(ui, app, "Paste Special…", "edit.pasteSpecial", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "cut", Some("Cut"), "Cut", "edit.cut", json!({}), false);
            let r = small(ui, app, "copy", Some("Copy"), "Copy", "edit.copy", json!({}), false);
            if r.clicked() {
                ui.ctx().copy_text(app.session.clipboard_text.clone());
            }
            let active = app.session.painter.is_some();
            small(ui, app, "painter", Some("Format"), "Format Painter", "edit.formatPainter", json!({}), active);
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
                menu_button(ui, app, "case", None, "Change Case", false, |ui, app| {
                    mi(ui, app, "Sentence case.", "format.changeCase", json!({"mode": "sentence"}));
                    mi(ui, app, "lowercase", "format.changeCase", json!({"mode": "lower"}));
                    mi(ui, app, "UPPERCASE", "format.changeCase", json!({"mode": "upper"}));
                    mi(ui, app, "Capitalize Each Word", "format.changeCase", json!({"mode": "title"}));
                    mi(ui, app, "tOGGLE cASE", "format.changeCase", json!({"mode": "toggle"}));
                });
                small(ui, app, "clear", None, "Clear All Formatting", "format.clear", json!({}), false);
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
                        ("Words only", "words"),
                    ] {
                        mi(ui, app, l, "format.underline", json!({"style": s}));
                    }
                });
                small(ui, app, "strike", None, "Strikethrough", "format.strikethrough", json!({}), flag("strike"));
                small(ui, app, "subscript", None, "Subscript", "format.subscript", json!({}), flag("subscript"));
                small(ui, app, "superscript", None, "Superscript", "format.superscript", json!({}), flag("superscript"));
                small(ui, app, "charborder", None, "Character Border", "format.border", json!({}), flag("border"));
                ui.add_space(4.0);
                menu_button(ui, app, "effects", None, "Text Effects and Typography", false, |ui, app| {
                    mi(ui, app, "Outline", "format.outline", json!({}));
                    mi(ui, app, "Shadow", "format.shadow", json!({}));
                    mi(ui, app, "Small Caps", "format.smallCaps", json!({}));
                    mi(ui, app, "All Caps", "format.allCaps", json!({}));
                    mi(ui, app, "Double Strikethrough", "format.doubleStrikethrough", json!({}));
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
                    mi(ui, app, "No Color", "format.highlight", json!({"color": "none"}));
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
        // The buttons show alignment as seen on the page (Align Right is a right-to-left paragraph's start).
        let align = rp.as_ref().map(|r| r.align.visual(r.bidi)).unwrap_or_default();
        let rtl = rp.as_ref().is_some_and(|r| r.bidi);
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                split(ui, app, "bullets", "Bullets", "para.bullets", json!({}), false, None, |ui, app| {
                    ui.label(egui::RichText::new(tl!("Bullet Library")).small().weak());
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
                    mi(ui, app, "Restart at 1", "para.restartNumbering", json!({}));
                    mi(ui, app, "None", "para.numbering", json!({"off": true}));
                });
                split(ui, app, "multilevel", "Multilevel List", "para.multilevel", json!({}), false, None, |ui, app| {
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
                small(ui, app, "pilcrow", None, "Show/Hide ¶", "view.marks", json!({}), marks);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                use wordcraft_doc::Align as A;
                small(ui, app, "alignLeft", None, "Align Left", "para.alignLeft", json!({}), align == A::Left);
                small(ui, app, "alignCenter", None, "Center", "para.alignCenter", json!({}), align == A::Center);
                small(ui, app, "alignRight", None, "Align Right", "para.alignRight", json!({}), align == A::Right);
                small(ui, app, "justify", None, "Justify", "para.justify", json!({}), align == A::Justify);
                ui.add_space(4.0);
                small(ui, app, "textLtr", None, "Left-to-Right Text Direction", "para.ltr", json!({}), !rtl);
                small(ui, app, "textRtl", None, "Right-to-Left Text Direction", "para.rtl", json!({}), rtl);
                ui.add_space(4.0);
                menu_button(ui, app, "lineSpacing", None, "Line and Paragraph Spacing", false, |ui, app| {
                    for v in [1.0, 1.15, 1.5, 2.0, 2.5, 3.0] {
                        mi(ui, app, &format!("{v}"), "para.lineSpacing", json!({"value": v}));
                    }
                    ui.separator();
                    mi(ui, app, "Add Space Before Paragraph", "para.addSpaceBefore", json!({}));
                    mi(ui, app, "Remove Space After Paragraph", "para.removeSpaceAfter", json!({}));
                    mi(ui, app, "Line Spacing Options…", "para.dialog", json!({}));
                });
                split(ui, app, "shading", "Shading", "para.shading", json!({"color": app.canvas.last_shading.clone()}), false, None, |ui, app| {
                    mi(ui, app, "No Color", "para.shading", json!({"color": null}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    if let Some(hex) = color_grid(ui, &theme) {
                        app.canvas.last_shading = hex.clone();
                        let _ = app.run("para.shading", json!({"color": hex}));
                        ui.close();
                    }
                });
                split(ui, app, "borders", "Borders", "para.borders", json!({"kind": "bottom"}), false, None, |ui, app| {
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
                });
            });
        });
    });
    group(ui, "Styles", Some("styles.pane"), app, |ui, app| {
        crate::previews::style_gallery(app, ui, &st);
    });
    group(ui, "Editing", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "find", Some("Find"), "Find", false, |ui, app| {
                mi(ui, app, "Find", "ui.dialog", json!({"name": "find"}));
                mi(ui, app, "Advanced Find…", "edit.advancedFind", json!({}));
                mi(ui, app, "Go To…", "ui.dialog", json!({"name": "goto"}));
            });
            small(ui, app, "replace", Some("Replace"), "Replace", "ui.dialog", json!({"name": "replace"}), false);
            menu_button(ui, app, "select", Some("Select"), "Select", false, |ui, app| {
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

fn insert(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Pages", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "coverPage", Some("Cover Page"), "Cover Page", false, |ui, app| {
                mi(ui, app, "Studio cover", "insert.coverPage", json!({}));
            });
            small(ui, app, "blankPage", Some("Blank Page"), "Blank Page", "insert.blankPage", json!({}), false);
            small(ui, app, "pageBreak", Some("Page Break"), "Page Break", "insert.pageBreak", json!({}), false);
        });
    });
    group(ui, "Tables", None, app, |ui, app| {
        menu_button(ui, app, "table", Some("Table"), "Add a Table", true, |ui, app| {
            crate::dialogs::table_grid_picker(ui, app);
            ui.separator();
            mi(ui, app, "Insert Table…", "ui.dialog", json!({"name": "insertTable"}));
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
    group(ui, "Header & Footer", None, app, |ui, app| {
        menu_button(ui, app, "header", Some("Header"), "Header", true, |ui, app| {
            mi(ui, app, "Blank", "insert.header", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.header", json!({"preset": "blankThree"}));
            mi(ui, app, "Document Title", "insert.header", json!({"preset": "title"}));
            ui.separator();
            mi(ui, app, "Edit Header", "insert.editHeader", json!({}));
            mi(ui, app, "Remove Header", "insert.removeHeader", json!({}));
        });
        menu_button(ui, app, "footer", Some("Footer"), "Footer", true, |ui, app| {
            mi(ui, app, "Blank", "insert.footer", json!({"preset": "blank"}));
            mi(ui, app, "Blank (Three Columns)", "insert.footer", json!({"preset": "blankThree"}));
            mi(ui, app, "Page Number", "insert.footer", json!({"preset": "pageNumber"}));
            ui.separator();
            mi(ui, app, "Edit Footer", "insert.editFooter", json!({}));
            mi(ui, app, "Remove Footer", "insert.removeFooter", json!({}));
        });
        menu_button(ui, app, "pageNumber", Some("Page\nNumber"), "Page Number", true, |ui, app| {
            mi(ui, app, "Top of Page", "insert.pageNumber", json!({"position": "top", "align": "right"}));
            mi(ui, app, "Bottom of Page", "insert.pageNumber", json!({"position": "bottom", "align": "center"}));
            mi(ui, app, "Page X of Y", "insert.pageNumber", json!({"position": "bottom", "format": "x of y"}));
            mi(ui, app, "Current Position", "insert.pageNumber", json!({"position": "current"}));
            mi(ui, app, "Format Page Numbers…", "layout.pageNumberFormat", json!({}));
        });
    });
    group(ui, "Text", None, app, |ui, app| {
        big(ui, app, "textBox", "Text\nBox", "insert.textBox", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "quickParts", Some("Quick Parts"), "Quick Parts", "insert.quickParts", json!({}), false);
            small(ui, app, "wordArt", Some("WordArt"), "WordArt", "insert.wordArt", json!({}), false);
            small(ui, app, "dropCap", Some("Drop Cap"), "Drop Cap", "insert.dropCap", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "signature", None, "Signature Line", "insert.signatureLine", json!({}), false);
            small(ui, app, "dateTime", None, "Date & Time", "insert.dateTime", json!({}), false);
            small(ui, app, "object", None, "Object", "insert.object", json!({}), false);
        });
    });
    group(ui, "Symbols", None, app, |ui, app| {
        crate::equation_tab::insert_button(ui, app);
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
    group(ui, "Document Formatting", None, app, |ui, app| {
        crate::previews::style_set_gallery(app, ui);
        stack(ui, |ui| {
            menu_button(ui, app, "colors", Some("Colors"), "Theme Colors", false, |ui, app| {
                for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                    mi(ui, app, name, "design.themeColors", json!({"name": name}));
                }
            });
            menu_button(ui, app, "fonts", Some("Fonts"), "Theme Fonts", false, |ui, app| {
                for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                    mi(ui, app, &format!("{name}: {h} / {b}"), "design.themeFonts", json!({"heading": h, "body": b}));
                }
            });
        });
        stack(ui, |ui| {
            menu_button(ui, app, "paraSpacing", Some("Paragraph Spacing"), "Paragraph Spacing", false, |ui, app| {
                for (l, v) in [
                    ("Default", "default"),
                    ("No Paragraph Space", "none"),
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
    group(ui, "Page Background", None, app, |ui, app| {
        menu_button(ui, app, "watermark", Some("Watermark"), "Watermark", true, |ui, app| {
            for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                mi(ui, app, w, "design.watermark", json!({"text": w}));
            }
            ui.separator();
            mi(ui, app, "Custom Watermark…", "ui.dialog", json!({"name": "watermark"}));
            mi(ui, app, "Remove Watermark", "design.watermark", json!({"remove": true}));
        });
        menu_button(ui, app, "pageColor", Some("Page\nColor"), "Page Color", true, |ui, app| {
            mi(ui, app, "No Color", "design.pageColor", json!({"color": null}));
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
    group(ui, "Page Setup", Some("ui.dialog"), app, |ui, app| {
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
            mi(ui, app, "Custom Margins…", "ui.dialog", json!({"name": "pageSetup"}));
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
                ui.label(egui::RichText::new(tl!("Page Breaks")).strong());
                mi(ui, app, "Page", "layout.break", json!({"kind": "page"}));
                mi(ui, app, "Column", "layout.break", json!({"kind": "column"}));
                mi(ui, app, "Text Wrapping", "layout.break", json!({"kind": "textWrapping"}));
                ui.label(egui::RichText::new(tl!("Section Breaks")).strong());
                mi(ui, app, "Next Page", "layout.break", json!({"kind": "nextPage"}));
                mi(ui, app, "Continuous", "layout.break", json!({"kind": "continuous"}));
                mi(ui, app, "Even Page", "layout.break", json!({"kind": "evenPage"}));
                mi(ui, app, "Odd Page", "layout.break", json!({"kind": "oddPage"}));
            });
            menu_button(ui, app, "lineNumbers", Some("Line Numbers"), "Line Numbers", false, |ui, app| {
                for (l, v) in
                    [("None", "none"), ("Continuous", "continuous"), ("Restart Each Page", "restartPage"), ("Restart Each Section", "restartSection")]
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
            ui.label(egui::RichText::new(tl!("Indent")).small().strong());
            ui.label("");
            ui.label(egui::RichText::new(tl!("Spacing")).small().strong());
            ui.end_row();
            let mut l = rp.indent_left / 72.0;
            let mut b = rp.space_before;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Left:")).small());
                if ui.add(egui::DragValue::new(&mut l).speed(0.05).range(-11.0..=22.0).suffix("\"").max_decimals(2)).changed() {
                    let _ = app.run("para.indents", json!({"left": l * 72.0}));
                }
            });
            ui.label("");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Before:")).small());
                if ui.add(egui::DragValue::new(&mut b).speed(1.0).range(0.0..=1584.0).suffix(" pt")).changed() {
                    let _ = app.run("para.spacing", json!({"before": b}));
                }
            });
            ui.end_row();
            let mut r = rp.indent_right / 72.0;
            let mut a = rp.space_after;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Right:")).small());
                if ui.add(egui::DragValue::new(&mut r).speed(0.05).range(-11.0..=22.0).suffix("\"").max_decimals(2)).changed() {
                    let _ = app.run("para.indents", json!({"right": r * 72.0}));
                }
            });
            ui.label("");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("After:")).small());
                if ui.add(egui::DragValue::new(&mut a).speed(1.0).range(0.0..=1584.0).suffix(" pt")).changed() {
                    let _ = app.run("para.spacing", json!({"after": a}));
                }
            });
            ui.end_row();
        });
    });
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        big(ui, app, "wrapText", "Wrap\nText", "arrange.wrap", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "bringForward", Some("Bring Forward"), "Bring Forward", "arrange.bringForward", json!({}), false);
            small(ui, app, "sendBackward", Some("Send Backward"), "Send Backward", "arrange.sendBackward", json!({}), false);
            small(ui, app, "selectionPane", Some("Selection Pane"), "Selection Pane", "arrange.selectionPane", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "align", None, "Align", "arrange.align", json!({}), false);
            group_menu(ui, app, None);
            small(ui, app, "rotate", None, "Rotate", "arrange.rotate", json!({}), false);
        });
    });
}

/// Arrange › Group: Group (Shift+click objects to select several) and Ungroup.
fn group_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    menu_button(ui, app, "group", label, "Group", false, |ui, app| {
        mi(ui, app, "Group", "arrange.group", json!({}));
        mi(ui, app, "Ungroup", "arrange.ungroup", json!({}));
    });
}

fn references(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table of Contents", None, app, |ui, app| {
        menu_button(ui, app, "toc", Some("Table of\nContents"), "Table of Contents", true, |ui, app| {
            mi(ui, app, "Automatic Table 1 (3 levels)", "references.toc", json!({"levels": 3}));
            mi(ui, app, "Automatic Table 2 (2 levels)", "references.toc", json!({"levels": 2, "title": "Table of Contents"}));
            mi(ui, app, "Remove Table of Contents", "references.removeToc", json!({}));
        });
        stack(ui, |ui| {
            menu_button(ui, app, "addText", Some("Add Text"), "Add Text", false, |ui, app| {
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
            small(ui, app, "endnote", Some("Insert Endnote"), "Insert Endnote", "references.endnote", json!({}), false);
            small(ui, app, "nextFootnote", Some("Next Footnote"), "Next Footnote", "references.nextFootnote", json!({}), false);
            small(ui, app, "showNotes", Some("Show Notes"), "Show Notes", "references.notes", json!({}), false);
        });
    });
    group(ui, "Research", None, app, |ui, app| {
        big(ui, app, "researcher", "Researcher", "references.researcher", json!({}), false);
    });
    group(ui, "Citations & Bibliography", None, app, |ui, app| {
        big(ui, app, "citation", "Insert\nCitation", "references.citation", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "sources", Some("Manage Sources"), "Manage Sources", "references.sources", json!({}), false);
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
            small(ui, app, "index", Some("Insert Index"), "Insert Index", "references.index", json!({}), false);
            small(ui, app, "update", Some("Update Index"), "Update Index", "references.updateIndex", json!({}), false);
        });
    });
    group(ui, "Table of Authorities", None, app, |ui, app| {
        big(ui, app, "markCitation", "Mark\nCitation", "references.markCitation", json!({}), false);
        small(ui, app, "tableOfAuthorities", None, "Insert Table of Authorities", "references.tableOfAuthorities", json!({}), false);
    });
}

fn mailings(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Create", None, app, |ui, app| {
        big(ui, app, "envelope", "Envelopes", "mailings.envelopes", json!({}), false);
        big(ui, app, "labels", "Labels", "mailings.labels", json!({}), false);
    });
    group(ui, "Start Mail Merge", None, app, |ui, app| {
        // The kinds of merge document the engine makes, the current one checked.
        menu_button(ui, app, "mailMerge", Some("Start Mail\nMerge"), "Start Mail Merge", true, |ui, app| {
            let kind = app.session.merge.kind.clone();
            for (label, k) in [
                ("Letters", "letters"),
                ("E-mail Messages", "emails"),
                ("Envelopes", "envelopes"),
                ("Labels", "labels"),
                ("Directory", "directory"),
                ("Normal Word Document", "normal"),
            ] {
                let current = kind == k || (k == "normal" && kind.is_empty());
                mi_check(ui, app, label, current, "mailings.start", json!({"kind": k}));
            }
        });
        // The command needs data, so the button offers the two ways to give it (#240).
        menu_button(ui, app, "recipients", Some("Select\nRecipients"), "Select Recipients", true, |ui, app| {
            mi(ui, app, "Type a New List…", "ui.dialog", json!({"name": "newRecipientList"}));
            mi(ui, app, "Use an Existing List…", "ui.openRecipientList", json!({}));
        });
        big(ui, app, "editRecipients", "Edit\nRecipient List", "mailings.editRecipients", json!({}), false);
    });
    group(ui, "Write & Insert Fields", None, app, |ui, app| {
        big(ui, app, "highlightFields", "Highlight\nMerge Fields", "mailings.highlightFields", json!({}), false);
        big(ui, app, "addressBlock", "Address\nBlock", "mailings.addressBlock", json!({}), false);
        big(ui, app, "greetingLine", "Greeting\nLine", "mailings.greetingLine", json!({}), false);
        big(ui, app, "mergeField", "Insert Merge\nField", "mailings.insertField", json!({}), false);
        stack(ui, |ui| {
            // The merge rules the engine knows; If and Skip Record If ask for their condition.
            menu_button(ui, app, "rules", Some("Rules"), "Rules", false, |ui, app| {
                mi(ui, app, "If…Then…Else…", "mailings.rules", json!({"rule": "IF"}));
                mi(ui, app, "Merge Record #", "mailings.rules", json!({"rule": "MERGEREC"}));
                mi(ui, app, "Next Record", "mailings.rules", json!({"rule": "NEXT"}));
                mi(ui, app, "Skip Record If…", "mailings.rules", json!({"rule": "SKIPIF"}));
            });
            small(ui, app, "matchFields", Some("Match Fields"), "Match Fields", "mailings.matchFields", json!({}), false);
        });
    });
    group(ui, "Preview Results", None, app, |ui, app| {
        big(ui, app, "preview", "Preview\nResults", "mailings.preview", json!({}), false);
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                small(ui, app, "previous", None, "Previous Record", "mailings.previous", json!({}), false);
                small(ui, app, "next", None, "Next Record", "mailings.next", json!({}), false);
            });
            small(ui, app, "find", Some("Find Recipient"), "Find Recipient", "mailings.findRecipient", json!({}), false);
            small(ui, app, "checkErrors", Some("Check for Errors"), "Check for Errors", "mailings.checkErrors", json!({}), false);
        });
    });
    group(ui, "Finish", None, app, |ui, app| {
        big(ui, app, "finish", "Finish &\nMerge", "mailings.finish", json!({}), false);
    });
}

/// Zotero (`docs/zotero.md`): the Zotero desktop app does the work in its own window.
fn zotero(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Citations", None, app, |ui, app| {
        big(ui, app, "citation", "Add/Edit\nCitation", "ui.zotero.addEditCitation", json!({}), false);
        big(ui, app, "addNote", "Add\nNote", "ui.zotero.addNote", json!({}), false);
        big(ui, app, "pastCitation", "Move Past\nCitation", "caret.pastCitation", json!({}), false);
    });
    group(ui, "Bibliography", None, app, |ui, app| {
        big(ui, app, "bibliography", "Add/Edit\nBibliography", "ui.zotero.addEditBibliography", json!({}), false);
    });
    group(ui, "Document", None, app, |ui, app| {
        big(ui, app, "update", "Refresh", "ui.zotero.refresh", json!({}), false);
        big(ui, app, "docPrefs", "Document\nPreferences", "ui.zotero.setDocPrefs", json!({}), false);
        big(ui, app, "unlinkCitations", "Unlink\nCitations", "ui.zotero.removeCodes", json!({}), false);
    });
}

fn review(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Proofing", None, app, |ui, app| {
        big(ui, app, "spelling", "Spelling &\nGrammar", "review.spelling", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "thesaurus", Some("Thesaurus"), "Thesaurus", "review.thesaurus", json!({}), false);
            small(ui, app, "wordCount", Some("Word Count"), "Word Count", "ui.dialog", json!({"name": "wordCount"}), false);
        });
    });
    group(ui, "Speech", None, app, |ui, app| {
        big(ui, app, "readAloud", "Read\nAloud", "review.readAloud", json!({}), false);
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
            small(ui, app, "deleteComment", Some("Delete"), "Delete Comment", "review.deleteComment", json!({}), false);
            small(ui, app, "prevComment", Some("Previous"), "Previous Comment", "review.previousComment", json!({}), false);
            small(ui, app, "nextComment", Some("Next"), "Next Comment", "review.nextComment", json!({}), false);
        });
        let shown = app.session.view.comments_pane;
        big(ui, app, "showComments", "Show\nComments", "view.commentsPane", json!({"value": !shown}), false);
    });
    group(ui, "Tracking", None, app, |ui, app| {
        let on = app.session.doc.settings.track_changes;
        let r = big(ui, app, "trackChanges", if on { "Track\nChanges ✓" } else { "Track\nChanges" }, "review.trackChanges", json!({}), false);
        let _ = r;
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                "markup",
                Some(if app.session.view.show_markup { "All Markup" } else { "No Markup" }),
                "Display for Review",
                false,
                |ui, app| {
                    mi(ui, app, "All Markup", "review.markup", json!({"value": "all"}));
                    mi(ui, app, "No Markup", "review.markup", json!({"value": "noMarkup"}));
                },
            );
            small(ui, app, "reviewingPane", Some("Reviewing Pane"), "Reviewing Pane", "review.changes", json!({}), false);
        });
    });
    group(ui, "Changes", None, app, |ui, app| {
        menu_button(ui, app, "accept", Some("Accept"), "Accept", true, |ui, app| {
            mi(ui, app, "Accept This Change", "review.accept", json!({}));
            mi(ui, app, "Accept All Changes", "review.acceptAll", json!({}));
        });
        menu_button(ui, app, "reject", Some("Reject"), "Reject", true, |ui, app| {
            mi(ui, app, "Reject Change", "review.reject", json!({}));
            mi(ui, app, "Reject All Changes", "review.rejectAll", json!({}));
        });
        stack(ui, |ui| {
            small(ui, app, "prevChange", Some("Previous"), "Previous Change", "review.previousChange", json!({}), false);
            small(ui, app, "nextChange", Some("Next"), "Next Change", "review.nextChange", json!({}), false);
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
        big(ui, app, "readMode", "Read\nMode", "view.readMode", json!({}), false);
        big(ui, app, "printLayout", "Print\nLayout", "view.printLayout", json!({}), false);
        big(ui, app, "webLayout", "Web\nLayout", "view.webLayout", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "outline", Some("Outline"), "Outline", "view.outline", json!({}), false);
            small(ui, app, "draft", Some("Draft"), "Draft", "view.draft", json!({}), false);
        });
    });
    group(ui, "Immersive", None, app, |ui, app| {
        big(ui, app, "focus", "Focus", "view.focus", json!({}), false);
        big(ui, app, "immersive", "Immersive\nReader", "view.immersive", json!({}), false);
    });
    group(ui, "Page Movement", None, app, |ui, app| {
        big(ui, app, "vertical", "Vertical", "view.vertical", json!({}), false);
        big(ui, app, "sideToSide", "Side\nto Side", "view.sideToSide", json!({}), false);
    });
    group(ui, "Show", None, app, |ui, app| {
        stack(ui, |ui| {
            let mut r = v.ruler;
            if ui.checkbox(&mut r, tl!("Ruler")).changed() {
                let _ = app.run("view.ruler", json!({"value": r}));
            }
            let mut g = v.gridlines;
            if ui.checkbox(&mut g, tl!("Gridlines")).changed() {
                let _ = app.run("view.gridlines", json!({"value": g}));
            }
            let mut n = v.nav_pane;
            if ui.checkbox(&mut n, tl!("Navigation Pane")).changed() {
                let _ = app.run("view.navigationPane", json!({"value": n}));
            }
        });
    });
    group(ui, "Zoom", None, app, |ui, app| {
        big(ui, app, "zoom", "Zoom", "ui.dialog", json!({"name": "zoom"}), false);
        big(ui, app, "zoom100", "100%", "view.zoom100", json!({}), false);
        // Step the zoom up and down by 10% (issue #67), from whatever the page shows now.
        stack(ui, |ui| {
            small(ui, app, "zoomIn", Some("Zoom In"), "Zoom In", "view.zoomIn", json!({}), false);
            small(ui, app, "zoomOut", Some("Zoom Out"), "Zoom Out", "view.zoomOut", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "onePage", Some("One Page"), "One Page", "view.onePage", json!({}), v.fit == "onePage");
            small(ui, app, "multiplePages", Some("Multiple Pages"), "Multiple Pages", "view.multiplePages", json!({}), v.multi_page);
            small(ui, app, "pageWidth", Some("Page Width"), "Page Width", "view.pageWidth", json!({}), v.fit == "pageWidth");
        });
    });
    group(ui, "Dark Mode", None, app, |ui, app| {
        big(ui, app, "darkMode", "Switch\nModes", "view.darkMode", json!({}), false);
        menu_button(ui, app, "interfaceTheme", Some("Interface\nTheme"), "Interface Theme", true, |ui, app| {
            for a in crate::theme::Appearance::ALL {
                if ui.add(egui::Button::selectable(app.ui.theme == a, tl!(a.label())).min_size(vec2(200.0, 0.0))).clicked() {
                    let _ = app.run("ui.theme", json!({"value": a.code()}));
                    ui.close();
                }
            }
        });
    });
    group(ui, "Window", None, app, |ui, app| {
        big(ui, app, "newWindow", "New\nWindow", "view.newWindow", json!({}), false);
        big(ui, app, "arrangeAll", "Arrange\nAll", "view.arrangeAll", json!({}), false);
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

/// Shape Format: fill, outline and effects of the selected shape, and its arrangement.
fn shape_format(app: &mut WordApp, ui: &mut Ui) {
    let (stroke, glow_size) = match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, wordcraft_doc::para::InlineObject::Shape { stroke, effects, .. })) => (stroke, effects.glow.map(|g| g.size)),
        _ => (None, None),
    };
    let theme = app.session.doc.settings.theme_colors.clone();
    group(ui, "Shape Styles", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "shading", Some("Shape Fill"), "Shape Fill", false, |ui, app| {
                mi(ui, app, "No Color", "shape.fill", json!({"color": null}));
                if let Some(hex) = color_grid(ui, &theme) {
                    let _ = app.run("shape.fill", json!({"color": hex}));
                    ui.close();
                }
            });
            menu_button(ui, app, "pen", Some("Shape Outline"), "Shape Outline", false, |ui, app| {
                mi(ui, app, "No Color", "shape.outline", json!({"color": null}));
                if let Some(hex) = color_grid(ui, &theme) {
                    let _ = app.run("shape.outline", json!({"color": hex}));
                    ui.close();
                }
                ui.separator();
                ui.menu_button(tl!("Weight"), |ui| {
                    let color = stroke.unwrap_or(wordcraft_doc::Rgb::BLACK).hex();
                    for w in [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0] {
                        mi(ui, app, &format!("{w} pt"), "shape.outline", json!({"color": color, "width": w}));
                    }
                });
            });
            menu_button(ui, app, "shapeEffects", Some("Shape Effects"), "Shadow, glow and soft edges", false, |ui, app| {
                shape_effects_menu(ui, app, &theme, glow_size)
            });
        });
    });
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        big(ui, app, "wrapText", "Wrap\nText", "arrange.wrap", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "rotate", Some("Rotate"), "Rotate", "arrange.rotate", json!({}), false);
            small(ui, app, "align", Some("Align"), "Align", "arrange.align", json!({}), false);
            group_menu(ui, app, Some("Group"));
        });
    });
}

/// Shape Format › Shape Effects: Shadow, Glow and Soft Edges, each with presets and a "No …" entry.
fn shape_effects_menu(ui: &mut Ui, app: &mut WordApp, theme: &[wordcraft_doc::Rgb], glow_size: Option<f32>) {
    ui.menu_button(tl!("Shadow"), |ui| {
        mi(ui, app, "No Shadow", "shape.effects", json!({"shadow": null}));
        ui.separator();
        for (id, label) in wordcraft_doc::effects::SHADOW_PRESETS {
            mi(ui, app, label, "shape.effects", json!({"shadow": id}));
        }
    });
    ui.menu_button(tl!("Glow"), |ui| {
        mi(ui, app, "No Glow", "shape.effects", json!({"glow": null}));
        ui.separator();
        for size in [5.0, 8.0, 11.0, 18.0] {
            mi(ui, app, &format!("{size} pt"), "shape.effects", json!({"glow": size}));
        }
        ui.separator();
        ui.menu_button(tl!("Glow Colors"), |ui| {
            if let Some(hex) = color_grid(ui, theme) {
                let _ = app.run("shape.effects", json!({"glow": {"color": hex, "size": glow_size.unwrap_or(8.0)}}));
                ui.close();
            }
        });
    });
    ui.menu_button(tl!("Soft Edges"), |ui| {
        mi(ui, app, "No Soft Edges", "shape.effects", json!({"softEdge": null}));
        ui.separator();
        for r in [1.0, 2.5, 5.0, 10.0, 25.0, 50.0] {
            mi(ui, app, &format!("{r} pt"), "shape.effects", json!({"softEdge": r}));
        }
    });
}

fn picture_format(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Adjust", None, app, |ui, app| {
        menu_button(ui, app, "picture", Some("Corrections"), "Brightness, contrast and sharpness", true, |ui, app| {
            for (l, b, c) in
                [("Brighter +20%", 20.0, 0.0), ("Darker −20%", -20.0, 0.0), ("More Contrast +20%", 0.0, 20.0), ("Less Contrast −20%", 0.0, -20.0)]
            {
                mi(ui, app, l, "picture.corrections", json!({"brightness": b, "contrast": c}));
            }
            ui.separator();
            mi(ui, app, "Sharpen", "picture.corrections", json!({"sharpen": 40.0}));
            mi(ui, app, "Soften", "picture.corrections", json!({"sharpen": -40.0}));
        });
        menu_button(ui, app, "picture", Some("Color"), "Saturation and recolor", true, |ui, app| {
            for (l, v) in [("Full Saturation 100%", 100.0), ("Muted 50%", 50.0), ("Gray 0%", 0.0), ("Vivid 200%", 200.0)] {
                mi(ui, app, l, "picture.color", json!({"mode": "saturation", "saturation": v}));
            }
            ui.separator();
            mi(ui, app, "Grayscale", "picture.color", json!({"mode": "grayscale"}));
            mi(ui, app, "Sepia", "picture.color", json!({"mode": "sepia"}));
            mi(ui, app, "Washout", "picture.color", json!({"mode": "washout"}));
        });
        menu_button(ui, app, "picture", Some("Transparency"), "Picture transparency", true, |ui, app| {
            for v in [0.0, 15.0, 30.0, 50.0, 65.0, 80.0, 95.0] {
                mi(ui, app, &format!("{v:.0}%"), "picture.transparency", json!({"percent": v}));
            }
        });
        stack(ui, |ui| {
            small(ui, app, "picture", Some("Change"), "Change Picture", "ui.changePicture", json!({}), false);
            small(ui, app, "picture", Some("Reset"), "Reset Picture", "picture.reset", json!({}), false);
        });
    });
    group(ui, "Picture Styles", None, app, |ui, app| {
        menu_button(ui, app, "picture", Some("Styles"), "Frames and effects", true, |ui, app| {
            for (l, s) in [
                ("Simple Frame", "simpleFrame"),
                ("Thick Frame", "thickFrame"),
                ("Rounded", "rounded"),
                ("Soft Edge", "softEdge"),
                ("Shadow", "shadow"),
            ] {
                mi(ui, app, l, "picture.style", json!({"style": s}));
            }
        });
        menu_button(ui, app, "picture", Some("Border"), "Picture border", true, |ui, app| {
            for (l, w) in [("Thin", 2), ("Medium", 4), ("Thick", 8)] {
                mi(ui, app, l, "picture.border", json!({"width": w}));
            }
        });
    });
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        big(ui, app, "wrapText", "Wrap\nText", "arrange.wrap", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "rotate", Some("Rotate"), "Rotate", "arrange.rotate", json!({}), false);
            small(ui, app, "align", Some("Align"), "Align", "arrange.align", json!({}), false);
            group_menu(ui, app, Some("Group"));
        });
    });
    group(ui, "Size", None, app, |ui, app| {
        let (media, off, w0, h0, crop0, alt0) = match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((pos, wordcraft_doc::para::InlineObject::Image { media, w, h, alt, crop, .. })) => (media, pos.off, w, h, crop, alt),
            _ => (String::new(), 0, 0.0, 0.0, [0.0; 4], String::new()),
        };
        // `picture.size` locks the aspect ratio unless told otherwise, so one dimension suffices.
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                ui.label(egui::RichText::new(tl!("W:")).small());
                let mut w = w0;
                let r = ui.add(egui::DragValue::new(&mut w).speed(1.0).range(4.0..=2000.0).suffix(" pt"));
                if r.changed() {
                    if r.dragged() && !r.drag_started() {
                        app.session.join_next_undo();
                    }
                    let _ = app.run("picture.size", json!({"width": w}));
                }
                ui.label(egui::RichText::new(tl!("H:")).small());
                let mut h = h0;
                let r = ui.add(egui::DragValue::new(&mut h).speed(1.0).range(4.0..=2000.0).suffix(" pt"));
                if r.changed() {
                    if r.dragged() && !r.drag_started() {
                        app.session.join_next_undo();
                    }
                    let _ = app.run("picture.size", json!({"height": h}));
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                for (i, side) in ["L", "T", "R", "B"].iter().enumerate() {
                    ui.label(egui::RichText::new(tl!(side)).small());
                    let mut v = crop0[i] * 100.0;
                    let r = ui.add(egui::DragValue::new(&mut v).speed(0.5).range(0.0..=45.0).suffix("%"));
                    if r.changed() {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let mut c = crop0;
                        c[i] = (v / 100.0).clamp(0.0, 0.45);
                        let _ = app.run("picture.crop", json!({"left": c[0], "top": c[1], "right": c[2], "bottom": c[3]}));
                    }
                }
                if ui.button(egui::RichText::new(tl!("Reset")).small()).on_hover_text(tl!("Reset Crop")).clicked() {
                    let _ = app.run("picture.crop", json!({"left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0}));
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                ui.label(egui::RichText::new(tl!("Alt:")).small());
                // Frame-local buffers lose keystrokes; keep the draft in egui temp memory keyed
                // by picture (same pattern as the comment editor in panes.rs).
                let key = egui::Id::new(("picture_alt", media.clone(), off));
                let mut alt = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| alt0.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut alt).desired_width(120.0).hint_text(tl!("Alt text")));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(key, alt.clone()));
                }
                if r.lost_focus() {
                    ui.data_mut(|d| d.remove::<String>(key));
                    // The click that took focus may have selected another picture; never commit
                    // one picture's draft onto another.
                    if alt != alt0 && app.selected_picture_media().as_deref() == Some(media.as_str()) {
                        let _ = app.run("picture.altText", json!({"text": alt}));
                    }
                }
            });
        });
    });
}

fn table_design(app: &mut WordApp, ui: &mut Ui) {
    let look = app.session.sel.focus.path.cell().and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look));
    group(ui, "Table Style Options", None, app, |ui, app| {
        let Some(l) = look else { return };
        egui::Grid::new("look").show(ui, |ui| {
            for (row, items) in [
                [("Header Row", "headerRow", l.header_row), ("First Column", "firstColumn", l.first_column)],
                [("Total Row", "totalRow", l.total_row), ("Last Column", "lastColumn", l.last_column)],
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
    group(ui, "Table Styles", None, app, |ui, app| {
        // The document's own styles first, so a new one shows without scrolling.
        let mut styles: Vec<(bool, String, String)> = app
            .session
            .doc
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && !s.hidden)
            .map(|s| (s.builtin, s.id.clone(), s.name.clone()))
            .collect();
        styles.sort_by_key(|(builtin, _, _)| *builtin);
        egui::ScrollArea::horizontal().max_width(420.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (_, id, name) in styles {
                    if crate::previews::table_style_tile(ui, app, &id).on_hover_text(name).clicked() {
                        let _ = app.run("table.style", json!({"style": id}));
                    }
                }
            });
        });
        stack(ui, |ui| {
            let current = look.and_then(|_| {
                let (tp, _, _) = app.session.sel.focus.path.cell()?;
                let id = app.session.doc.table(app.session.sel.focus.story, &tp)?.props.style.clone()?;
                app.session.doc.styles.get(&id).filter(|st| st.kind == wordcraft_doc::StyleKind::Table).map(|st| st.builtin)
            });
            let can_modify = current.is_some();
            // Built-in table styles can't be deleted.
            let can_delete = current == Some(false);
            menu_button(ui, app, "styles", Some("Styles"), "Table Styles", false, |ui, app| {
                mi(ui, app, "New Table Style…", "ui.dialog", json!({"name": "newTableStyle"}));
                if ui.add_enabled(can_modify, egui::Button::new(tl!("Modify Table Style…")).min_size(vec2(200.0, 0.0))).clicked() {
                    let _ = app.run("ui.dialog", json!({"name": "modifyTableStyle"}));
                    ui.close();
                }
                if ui.add_enabled(can_delete, egui::Button::new(tl!("Delete Table Style")).min_size(vec2(200.0, 0.0))).clicked() {
                    if let Err(e) = app.run("table.deleteStyle", json!({})) {
                        app.status(e);
                    }
                    ui.close();
                }
            });
            split(ui, app, "shading", "Shading", "table.shading", json!({"color": app.canvas.last_shading.clone()}), false, None, |ui, app| {
                mi(ui, app, "No Color", "table.shading", json!({"color": null}));
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
                ("All Borders", "all"),
                ("Outside Borders", "outside"),
                ("Inside Borders", "inside"),
                ("No Border", "none"),
                ("Top Border", "top"),
                ("Bottom Border", "bottom"),
            ] {
                mi(ui, app, l, "table.borders", json!({"kind": k}));
            }
        });
        big(ui, app, "borderPainter", "Border\nPainter", "table.borderPainter", json!({}), false);
    });
}

/// Table Layout › Cell Size: the caret cell's row height and column width, in inches. They show
/// the size on the page (a row without a set height shows the height its text gives it) and
/// editing runs `table.rowHeight` / `table.columnWidth`; dragging one is a single Undo.
fn cell_size_boxes(ui: &mut Ui, app: &mut WordApp) {
    let story = app.session.sel.focus.story;
    let Some((tp, r, c)) = app.session.sel.focus.path.cell() else { return };
    let Some(t) = app.session.doc.table(story, &tp) else { return };
    let row = t.rows.get(r).map(|x| x.props.clone()).unwrap_or_default();
    let stored_w = t.grid.get(t.grid_col(r, c)).copied();
    let laid = app.session.layout().pages.iter().flat_map(|pg| pg.items.iter()).find_map(|it| match it {
        wordcraft_layout::Placed::Cell { rect, table, row, cell, story: st } if *table == tp && *row == r && *cell == c && *st == story => {
            Some(*rect)
        }
        _ => None,
    });
    let unit = wordcraft_geom::Unit::default();
    let k = unit.pt_per_unit();
    let h0 = row.height.or(laid.map(|x| x.h)).unwrap_or(0.0);
    let w0 = laid.map(|x| x.w).or(stored_w).unwrap_or(0.0);
    let exact = row.height_rule == wordcraft_doc::props::HeightRule::Exact;
    // The edited value in points, and whether it continues a drag (joins the previous Undo step).
    let boxed = |ui: &mut Ui, label: &str, pt: f32| -> Option<(f32, bool)> {
        let mut out = None;
        crate::widgets::row(ui, |ui| {
            ui.add_sized(vec2(48.0, 18.0), egui::Label::new(egui::RichText::new(tl!(label)).small()));
            let mut v = pt / k;
            let r = ui.add_sized(vec2(72.0, 18.0), egui::DragValue::new(&mut v).speed(0.01).range(0.02..=22.0).suffix(unit.suffix()).max_decimals(2));
            if r.changed() {
                out = Some((v * k, r.dragged() && !r.drag_started()));
            }
        });
        ui.add_space(2.0);
        out
    };
    stack(ui, |ui| {
        if let Some((h, join)) = boxed(ui, "Height:", h0) {
            if join {
                app.session.join_next_undo();
            }
            let _ = app.run("table.rowHeight", json!({"height": h, "rule": if exact { "exact" } else { "atLeast" }}));
        }
        if let Some((w, join)) = boxed(ui, "Width:", w0) {
            if join {
                app.session.join_next_undo();
            }
            let _ = app.run("table.columnWidth", json!({"width": w}));
        }
    });
}

fn table_layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(ui, app, "selectTable", Some("Select"), "Select", false, |ui, app| {
                mi(ui, app, "Select Cell", "table.selectCell", json!({}));
                mi(ui, app, "Select Row", "table.selectRow", json!({}));
                mi(ui, app, "Select Table", "table.selectTable", json!({}));
            });
            let g = app.session.view.table_gridlines;
            small(ui, app, "gridlines", Some("View Gridlines"), "View Gridlines", "table.viewGridlines", json!({}), g);
            small(ui, app, "properties", Some("Properties"), "Table Properties", "table.properties", json!({}), false);
        });
    });
    group(ui, "Rows & Columns", None, app, |ui, app| {
        menu_button(ui, app, "deleteTable", Some("Delete"), "Delete", true, |ui, app| {
            mi(ui, app, "Delete Cells", "table.deleteCells", json!({}));
            mi(ui, app, "Delete Columns", "table.deleteColumn", json!({}));
            mi(ui, app, "Delete Rows", "table.deleteRow", json!({}));
            mi(ui, app, "Delete Table", "table.deleteTable", json!({}));
        });
        big(ui, app, "insertAbove", "Insert\nAbove", "table.insertRowAbove", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "insertBelow", Some("Insert Below"), "Insert Below", "table.insertRowBelow", json!({}), false);
            small(ui, app, "insertLeft", Some("Insert Left"), "Insert Left", "table.insertColumnLeft", json!({}), false);
            small(ui, app, "insertRight", Some("Insert Right"), "Insert Right", "table.insertColumnRight", json!({}), false);
        });
    });
    group(ui, "Merge", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "merge", Some("Merge Cells"), "Merge Cells", "table.merge", json!({}), false);
            small(ui, app, "splitCells", Some("Split Cells"), "Split Cells", "table.split", json!({}), false);
            small(ui, app, "splitTable", Some("Split Table"), "Split Table", "table.splitTable", json!({}), false);
        });
    });
    group(ui, "Cell Size", None, app, |ui, app| {
        menu_button(ui, app, "autofit", Some("AutoFit"), "AutoFit", true, |ui, app| {
            mi(ui, app, "AutoFit Contents", "table.autofit", json!({"mode": "contents"}));
            mi(ui, app, "AutoFit Window", "table.autofit", json!({"mode": "window"}));
            mi(ui, app, "Fixed Column Width", "table.autofit", json!({"mode": "fixed"}));
        });
        cell_size_boxes(ui, app);
        stack(ui, |ui| {
            small(ui, app, "distributeRows", Some("Distribute Rows"), "Distribute Rows", "table.distributeRows", json!({}), false);
            small(ui, app, "distributeCols", Some("Distribute Columns"), "Distribute Columns", "table.distributeColumns", json!({}), false);
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
        big(ui, app, "textDirection", "Text\nDirection", "table.textDirection", json!({}), false);
        big(ui, app, "cellMargins", "Cell\nMargins", "table.cellMargins", json!({}), false);
    });
    group(ui, "Data", None, app, |ui, app| {
        big(ui, app, "sort", "Sort", "table.sort", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "repeatHeader", Some("Repeat Header Rows"), "Repeat Header Rows", "table.repeatHeader", json!({}), false);
            small(ui, app, "toText", Some("Convert to Text"), "Convert to Text", "table.toText", json!({}), false);
            small(ui, app, "formula", Some("Formula"), "Formula", "table.formula", json!({}), false);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_engine::Session;

    fn png_data() -> String {
        let img = image::RgbaImage::from_fn(20, 10, |x, y| {
            if x > 2 && x < 17 && y > 2 && y < 7 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let mut b = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        wordcraft_engine::cmd::insert::base64_encode(&b)
    }

    #[test]
    fn picture_format_tab_appears_when_picture_selected() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        assert!(!has_picture_selected(&s));
        assert!(!contextual_tabs(&s).contains(&"Picture Format"));
        s.run("insert.picture", &json!({"data": png_data()})).unwrap();
        assert!(has_picture_selected(&s));
        assert!(contextual_tabs(&s).contains(&"Picture Format"));
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!has_picture_selected(&s));
    }

    #[test]
    fn shape_format_tab_appears_when_shape_selected() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
        assert!(contextual_tabs(&s).contains(&"Shape Format"));
        assert!(!contextual_tabs(&s).contains(&"Picture Format"));
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!contextual_tabs(&s).contains(&"Shape Format"));
    }

    #[test]
    fn resolve_tab_falls_back_when_contextual_tab_expires() {
        let all = ["Home", "Table Design", "Picture Format"];
        assert_eq!(resolve_tab("Picture Format", &all), "Picture Format");
        assert_eq!(resolve_tab("Picture Format", &["Home"]), "Home");
        assert_eq!(resolve_tab("Table Design", &["Home"]), "Home");
        assert_eq!(resolve_tab("Home", &["Home"]), "Home");
    }
}
