use craft::behavior::{IconId, keyed};
use craft::menu::{CommandRef, RibbonGroup, RibbonRow};
use craft::theme::variant::Tone;
use craft::theme::{Emphasis, Variant};
use craft::ui::buttons::{Arrangement, Button, IconButton, SplitButton, SplitMenu, ToggleButton};
use craft::ui::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, Entry, Gap, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft::ui::display::Divider;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::{HAlign, TextRun};
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use wordcraft_doc::props::Align;

use super::common::{icon_only, menu_button};
use super::underline::underline_menu;
use egui::vec2;
use serde_json::Value;

use super::{Ui, WordApp, json};
use crate::colors::{Remember, Target};

/// A ribbon group titled `title`, with the Dialog Launcher `launcher` when it has one.
fn ribbon_group(key: &'static str, title: &str, icon: &'static str, launcher: Option<&str>) -> RibbonGroup<'static> {
    let group = RibbonGroup::new(key, tl!(title).to_string(), crate::icons::id(icon).into());
    match launcher {
        Some(id) => group.launcher(CommandRef::new(id)),
        None => group,
    }
}

/// The Format Painter button. A click arms it for one use (outlined); a double-click keeps
/// it on across several uses (selected look), like Word. Esc or another click turns it off.
fn format_painter_button(ui: &mut Ui, app: &mut WordApp) {
    let sticky = matches!(app.session.painter, Some((_, _, true)));
    let armed = app.session.painter.is_some();
    let mut button = Button::command(CommandRef::with_params("edit.formatPainter", json!({})))
        .icon(IconId::new("painter"))
        .label(tl!("Format").to_string())
        .tooltip(tl!("Format Painter").to_string())
        .selected(sticky);
    if armed && !sticky {
        button = button.variant(Variant::look(Emphasis::Outline, Tone::Accent));
    }
    if button.show(ui).clicks == 2 {
        painter_double_click(app);
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

/// The bullets of the Bullet Library, in tile order.
const BULLETS: [&str; 7] = ["•", "○", "▪", "◆", "➢", "✓", "–"];

/// Change List Level › and Define New Bullet… at the foot of the Bullets drop-down (#2786).
/// Change List Level is for a paragraph in a list of its own numbering, the one `para.listLevel`
/// changes (a style's numbering it leaves alone).
fn bullet_list_items(ui: &mut Ui, app: &mut WordApp) {
    Divider::horizontal().show(ui);
    let in_list = app.session.doc.para_at(&app.session.sel.focus).and_then(|p| p.props.numbering).is_some_and(|n| n.num != 0);
    Menu::submenu_row_enabled(ui, tl!("Change List Level").to_string(), in_list, |ui, _| {
        for level in 0..9u8 {
            let label = format!("{} {}", tl!("Level"), level + 1);
            MenuItem::command(CommandRef::with_params("para.listLevel", json!({"level": level}))).label(label).show(ui);
        }
    });
    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "defineBullet"}))).label(tl!("Define New Bullet…").to_string()).show(ui);
}

/// The Bullets drop-down: the Bullet Library as tiles, None, and the list-level rows.
fn bullets_menu(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    MenuItem::heading(tl!("Bullet Library").to_string()).show(ui);
    let source = keyed(&BULLETS, |c| *c);
    let out = TileGrid::new(tl!("Bullet Library").to_string())
        .caption(CaptionPlace::None)
        .select(SelectMode::None)
        .activate(ActivateOn::Click)
        .card_columns(BULLETS.len())
        .content_cells()
        // One row, all shown; the menu's body is the one scroll.
        .natural_height(true)
        .show_items(ui, &source, &mut |c, _| {
            let c: &'static str = c;
            Tile::new(c).thumb(ThumbAspect::SQUARE, move |cx, r| paint_bullet(cx, r, c))
        });
    for event in &out.response.events {
        if let CollectionEvent::Activate(c) = event {
            let _ = app.run("para.bullets", json!({"kind": c}));
            cx.request_close();
        }
    }
    MenuItem::command(CommandRef::with_params("para.bullets", json!({"off": true}))).label(tl!("None").to_string()).show(ui);
    bullet_list_items(ui, app);
}

/// One Bullet Library tile. The interface fonts have no ➢, so its tile draws the arrowhead, scaled from the tile.
fn paint_bullet(cx: &craft::ui::primitives::ContentPainterCx<'_>, r: egui::Rect, c: &str) {
    let ink = cx.ink();
    if c == "➢" {
        let m = r.center();
        let s = r.height() * 0.2;
        let pts = [(-0.8, -1.0), (1.0, 0.0), (-0.8, 1.0), (-0.3, 0.0)].map(|(x, y)| m + vec2(x * s, y * s));
        cx.painter.add(egui::Shape::convex_polygon(vec![pts[0], pts[1], pts[3]], ink, egui::Stroke::NONE));
        cx.painter.add(egui::Shape::convex_polygon(vec![pts[3], pts[1], pts[2]], ink, egui::Stroke::NONE));
    } else {
        cx.with_draw(|d| {
            d.text(r, &TextRun::new(c).role(craft::theme::TypeRole::Body).align(HAlign::Center));
        });
    }
}

/// The Text Effects and Typography menu.
fn text_effects_menu(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    super::galleries::wordart(ui, cx, app, "format.textEffects");
    Divider::horizontal().show(ui);
    let theme = app.session.doc.settings.theme_colors.clone();
    Menu::submenu_row(ui, tl!("Text Fill").to_string(), |ui, cx| crate::wordart_menus::fill_menu(ui, cx, app, &theme));
    let outline = crate::wordart_menus::current_outline(app);
    Menu::submenu_row(ui, tl!("Text Outline").to_string(), |ui, _| crate::wordart_menus::outline_menu(ui, app, outline));
    crate::wordart_menus::effects_menu(ui, app, false);
    Divider::horizontal().show(ui);
    for (label, id) in [
        ("Outline", "format.outline"),
        ("Shadow", "format.shadow"),
        ("Small Caps", "format.smallCaps"),
        ("All Caps", "format.allCaps"),
        ("Double Strikethrough", "format.doubleStrikethrough"),
    ] {
        MenuItem::command(CommandRef::new(id)).label(tl!(label).to_string()).show(ui);
    }
}

pub(super) fn home(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    // Read directly, not as a command: every command consumes the engine's pending undo joins, and
    // the ribbon draws between an input method's deletion and the commit that replaces it.
    let st = wordcraft_engine::cmd::format::state(&app.session);
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    row.group(ribbon_group("clipboard", "Clipboard", "paste", Some("edit.clipboardPane")), |ui| {
        // The face opens the menu and runs no command, so its tooltip adds Paste's key itself, as this
        // platform writes it ("Paste (Ctrl+V)", "Paste (⌘V)" on macOS). The name screen readers read is "Paste".
        let paste_tip = match craft::ui::runtime::commands::resolve(ui.ctx(), CommandRef::new("edit.paste")).chord {
            Some(chord) => craft::ui::contract::tooltip::with_chord(tl!("Paste"), &chord, craft::ui::runtime::windows::os_of(ui.ctx())),
            None => tl!("Paste").to_string(),
        };
        let face = Button::new(tl!("Paste").to_string()).icon(IconId::new("paste")).tooltip(paste_tip).arrangement(Arrangement::Stacked);
        menu_button(ui, face, tl!("Paste").to_string(), Some("edit.paste"), |ui, _| {
            for (label, id) in [
                ("Paste", "edit.paste"),
                ("Keep Text Only", "edit.pasteText"),
                ("Merge Formatting", "edit.pasteMerge"),
                ("Paste Special…", "edit.pasteSpecial"),
            ] {
                MenuItem::command(CommandRef::new(id)).label(tl!(label).to_string()).show(ui);
            }
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (icon, label, id) in [("cut", "Cut", "edit.cut"), ("copy", "Copy", "edit.copy")] {
                Button::command(CommandRef::new(id))
                    .icon(crate::icons::id(icon))
                    .label(tl!(label).to_string())
                    .tooltip(tl!(label).to_string())
                    .show(ui);
            }
            format_painter_button(ui, app);
        });
    });
    row.group(ribbon_group("font", "Font", "font", Some("format.fontDialog")), |ui| {
        // The Font and Paragraph groups' small buttons sit side by side with no gap, as the old ribbon's (1 pt apart),
        // and show their icons alone so they never expand with text labels (#7308).
        icon_only(ui, |ui| {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                Row::new().gap(Gap::None).show(ui, |ui, _| {
                    let mut family = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
                    let out = crate::widgets::font_box(ui, app, tl!("Font").to_string(), &mut family);
                    if let Some(f) = out.picked {
                        let _ = app.run("format.font", json!({"name": f}));
                    }
                    let fmt = |s: f64| if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s}") };
                    let mut size = st.get("size").and_then(Value::as_f64).map(fmt).unwrap_or_default();
                    let sizes: Vec<Entry<String>> =
                        wordcraft_engine::cmd::format::SIZES.iter().map(|s| Entry::new(fmt(f64::from(*s)), fmt(f64::from(*s)))).collect();
                    let out = craft::ui::pickers::ComboBox::new(tl!("Font Size").to_string(), Bind::bound(&mut size), &sizes).show(ui);
                    if let Some(v) = out.edit.events.committed()
                        && let Ok(x) = v.trim().parse::<f64>()
                    {
                        let _ = app.run("format.size", json!({"size": x}));
                    }
                    for (icon, tip, id) in [("grow", "Increase Font Size", "format.growFont"), ("shrink", "Decrease Font Size", "format.shrinkFont")]
                    {
                        IconButton::command(crate::icons::id(icon), CommandRef::new(id))
                            .label(tl!(tip).to_string())
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                    let face = Button::new(tl!("Change Case").to_string()).icon(IconId::new("case")).tooltip(tl!("Change Case").to_string());
                    menu_button(ui, face, tl!("Change Case").to_string(), Some("format.changeCase"), |ui, _| {
                        for (label, mode) in [
                            ("Sentence case.", "sentence"),
                            ("lowercase", "lower"),
                            ("UPPERCASE", "upper"),
                            ("Capitalize Each Word", "title"),
                            ("tOGGLE cASE", "toggle"),
                        ] {
                            MenuItem::command(CommandRef::with_params("format.changeCase", json!({"mode": mode})))
                                .label(tl!(label).to_string())
                                .show(ui);
                        }
                    });
                    for (icon, tip, id) in [("clear", "Clear All Formatting", "format.clear"), ("phonetic", "Phonetic Guide", "format.phonetic")] {
                        IconButton::command(crate::icons::id(icon), CommandRef::new(id))
                            .label(tl!(tip).to_string())
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                });
                Row::new().gap(Gap::None).show(ui, |ui, _| {
                    for (icon, tip, id, on) in [("bold", "Bold", "format.bold", flag("bold")), ("italic", "Italic", "format.italic", flag("italic"))]
                    {
                        ToggleButton::command(CommandRef::new(id))
                            .icon(crate::icons::id(icon))
                            .icon_only()
                            .selected(on)
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                    SplitButton::<String>::command(
                        CommandRef::with_params("format.underline", json!({})),
                        SplitMenu::Body(Box::new(|ui, _| underline_menu(ui, app))),
                    )
                    .icon(IconId::new("underline"))
                    .selected(flag("underline"))
                    .tooltip(tl!("Underline").to_string())
                    .show(ui);
                    for (icon, tip, id, on) in [
                        ("strike", "Strikethrough", "format.strikethrough", flag("strike")),
                        ("subscript", "Subscript", "format.subscript", flag("subscript")),
                        ("superscript", "Superscript", "format.superscript", flag("superscript")),
                        ("charborder", "Character Border", "format.border", flag("border")),
                    ] {
                        ToggleButton::command(CommandRef::new(id))
                            .icon(crate::icons::id(icon))
                            .icon_only()
                            .selected(on)
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                    IconButton::command(IconId::new("enclose"), CommandRef::new("format.enclose"))
                        .label(tl!("Enclose Characters").to_string())
                        .tooltip(tl!("Enclose Characters").to_string())
                        .show(ui);
                    let face = Button::new(tl!("Text Effects and Typography").to_string())
                        .icon(IconId::new("effects"))
                        .tooltip(tl!("Text Effects and Typography").to_string());
                    menu_button(ui, face, tl!("Text Effects and Typography").to_string(), None, |ui, cx| text_effects_menu(ui, cx, app));
                    crate::colors::highlight_split(ui, app, tl!("Text Highlight Color"));
                    crate::colors::font_color_split(ui, app, tl!("Font Color"), Target::cmd("format.color").remember(Remember::FontColor));
                });
            });
        });
    });
    row.group(ribbon_group("paragraph", "Paragraph", "paragraph", Some("para.dialog")), |ui| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|_| wordcraft_engine::cmd::para::selection_para(&app.session));
        let lists = wordcraft_engine::cmd::para::list_buttons(&app.session.doc, rp.as_ref().and_then(|r| r.numbering));
        // Alignment is shown as it reads on the page, so Left and Right swap in a right-to-left paragraph.
        let (align, rtl) = rp.as_ref().map_or((Align::Left, false), |r| (r.align.visual(r.bidi), r.bidi));
        let marks = app.session.view.marks;
        icon_only(ui, |ui| {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                Row::new().gap(Gap::None).show(ui, |ui, _| {
                    SplitButton::<String>::command(
                        CommandRef::with_params("para.bullets", json!({})),
                        SplitMenu::Body(Box::new(|ui, cx| bullets_menu(ui, cx, app))),
                    )
                    .icon(IconId::new("bullets"))
                    .selected(lists.bullets)
                    .tooltip(tl!("Bullets").to_string())
                    .show(ui);
                    SplitButton::<String>::command(
                        CommandRef::with_params("para.numbering", json!({})),
                        SplitMenu::Body(Box::new(|ui, _| {
                            for (l, k) in [
                                ("1. 2. 3.", "numbered"),
                                ("1) 2) 3)", "numberedParen"),
                                ("I. II. III.", "outline"),
                                ("A. B. C.", "upperLetter"),
                                ("a) b) c)", "lowerLetter"),
                                ("i. ii. iii.", "lowerRoman"),
                            ] {
                                MenuItem::command(CommandRef::with_params("para.numbering", json!({"kind": k}))).label(tl!(l).to_string()).show(ui);
                            }
                            Divider::horizontal().show(ui);
                            MenuItem::command(CommandRef::new("para.restartNumbering")).label(tl!("Restart at 1").to_string()).show(ui);
                            MenuItem::command(CommandRef::with_params("para.numbering", json!({"off": true})))
                                .label(tl!("None").to_string())
                                .show(ui);
                        })),
                    )
                    .icon(IconId::new("numbering"))
                    .selected(lists.numbering)
                    .tooltip(tl!("Numbering").to_string())
                    .show(ui);
                    SplitButton::<String>::command(
                        CommandRef::with_params("para.multilevel", json!({})),
                        SplitMenu::Body(Box::new(|ui, _| {
                            for (l, k) in [("1. 1.1. 1.1.1.", "legal"), ("I. A. 1. a.", "outline")] {
                                MenuItem::command(CommandRef::with_params("para.multilevel", json!({"kind": k}))).label(tl!(l).to_string()).show(ui);
                            }
                            Divider::horizontal().show(ui);
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
                                MenuItem::command(CommandRef::with_params("list.multilevelPreset", json!({"preset": k}))).label(text).show(ui);
                            }
                            Divider::horizontal().show(ui);
                            MenuItem::command(CommandRef::new("list.define")).label(tl!("Define New Multilevel List…").to_string()).show(ui);
                            Divider::horizontal().show(ui);
                            for lv in 0..5u64 {
                                MenuItem::command(CommandRef::with_params("para.listLevel", json!({"level": lv})))
                                    .label(format!("Change to Level {}", lv + 1))
                                    .show(ui);
                            }
                        })),
                    )
                    .icon(IconId::new("multilevel"))
                    .selected(lists.multilevel)
                    .tooltip(tl!("Multilevel List").to_string())
                    .show(ui);
                    for (icon, tip, id) in
                        [("outdent", "Decrease Indent", "para.outdent"), ("indent", "Increase Indent", "para.indent"), ("sort", "Sort", "para.sort")]
                    {
                        IconButton::command(crate::icons::id(icon), CommandRef::new(id))
                            .label(tl!(tip).to_string())
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                    ToggleButton::command(CommandRef::new("view.marks"))
                        .icon(IconId::new("wordcraft.pilcrow"))
                        .icon_only()
                        .selected(marks)
                        .tooltip(tl!("Show/Hide ¶").to_string())
                        .show(ui);
                });
                Row::new().gap(Gap::None).show(ui, |ui, _| {
                    for (icon, tip, id, on) in [
                        ("alignLeft", "Align Left", "para.alignLeft", align == Align::Left),
                        ("alignCenter", "Center", "para.alignCenter", align == Align::Center),
                        ("alignRight", "Align Right", "para.alignRight", align == Align::Right),
                        ("justify", "Justify", "para.justify", align == Align::Justify),
                        ("textLtr", "Left-to-Right Text Direction", "para.ltr", !rtl),
                        ("textRtl", "Right-to-Left Text Direction", "para.rtl", rtl),
                    ] {
                        ToggleButton::command(CommandRef::new(id))
                            .icon(crate::icons::id(icon))
                            .icon_only()
                            .selected(on)
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                    let face = Button::new(tl!("Line and Paragraph Spacing").to_string())
                        .icon(IconId::new("lineSpacing"))
                        .tooltip(tl!("Line and Paragraph Spacing").to_string());
                    menu_button(ui, face, tl!("Line and Paragraph Spacing").to_string(), Some("para.lineSpacing"), |ui, _| {
                        for v in [1.0, 1.15, 1.5, 2.0, 2.5, 3.0] {
                            MenuItem::command(CommandRef::with_params("para.lineSpacing", json!({"value": v}))).label(format!("{v}")).show(ui);
                        }
                        Divider::horizontal().show(ui);
                        for (label, id) in [
                            ("Add Space Before Paragraph", "para.addSpaceBefore"),
                            ("Remove Space After Paragraph", "para.removeSpaceAfter"),
                            ("Line Spacing Options…", "para.dialog"),
                        ] {
                            MenuItem::command(CommandRef::new(id)).label(tl!(label).to_string()).show(ui);
                        }
                    });
                    crate::colors::shading_split(ui, app, tl!("Shading"), Target::cmd("para.shading").remember(Remember::Shading));
                    SplitButton::<String>::command(
                        CommandRef::with_params("para.borders", json!({"kind": "bottom"})),
                        SplitMenu::Body(Box::new(|ui, _| {
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
                                MenuItem::command(CommandRef::with_params("para.borders", json!({"kind": k}))).label(tl!(l).to_string()).show(ui);
                            }
                            Divider::horizontal().show(ui);
                            MenuItem::command(CommandRef::new("insert.horizontalLine")).label(tl!("Horizontal Line").to_string()).show(ui);
                            MenuItem::command(CommandRef::new("para.borders")).label(tl!("Borders and Shading…").to_string()).show(ui);
                        })),
                    )
                    .icon(IconId::new("borders"))
                    .tooltip(tl!("Borders").to_string())
                    .show(ui);
                });
            });
        });
    });
    // Advanced paragraph tools (#5043): conversions and cleanup over the selection, or the
    // whole document with Ctrl+A first (the commands take `scope: "all"` for that as well).
    row.group(ribbon_group("paragraphTools", "Paragraph Tools", "pilcrow", None), |ui| {
        let face = Button::new(tl!("Paragraph\nTools").to_string())
            .icon(IconId::new("wordcraft.pilcrow"))
            .tooltip(tl!("Paragraph Tools").to_string())
            .arrangement(Arrangement::Stacked);
        menu_button(ui, face, tl!("Paragraph Tools").to_string(), None, |ui, _| {
            let run = |ui: &mut Ui, label: &str, id: &str, params: Value| {
                MenuItem::command(CommandRef::with_params(id, params)).label(tl!(label).to_string()).show(ui);
            };
            run(ui, "Split Lines into Paragraphs", "para.convertBreaks", json!({}));
            run(ui, "Add Blank Paragraphs Between", "para.blankBetween", json!({}));
            run(ui, "Separate Paragraphs", "para.separateParagraphs", json!({}));
            Divider::horizontal().show(ui);
            run(ui, "Delete Blank Paragraphs", "para.removeBlank", json!({}));
            run(ui, "Remove Leading Spaces", "para.trimLeadingSpaces", json!({}));
            run(ui, "Remove Extra Spaces", "para.removeExtraSpaces", json!({}));
            Divider::horizontal().show(ui);
            run(ui, "Indent Leading Spaces", "para.spacesToIndent", json!({}));
            run(ui, "Indents to Leading Spaces", "para.indentToSpaces", json!({}));
            run(ui, "Join Lines", "para.removeLineBreaks", json!({}));
            Divider::horizontal().show(ui);
            // The whole document, not just the selection.
            for (l, id) in [
                ("Delete Blank Paragraphs in the Whole Document", "para.removeBlank"),
                ("Remove Leading Spaces in the Whole Document", "para.trimLeadingSpaces"),
                ("Remove Extra Spaces in the Whole Document", "para.removeExtraSpaces"),
            ] {
                run(ui, l, id, json!({"scope": "all"}));
            }
        });
    });
    row.group(ribbon_group("styles", "Styles", "styles", Some("styles.pane")), |ui| {
        super::galleries::styles(ui, app, st.get("style").and_then(Value::as_str));
    });
    row.group(ribbon_group("editing", "Editing", "find", None), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            let face = Button::new(tl!("Find").to_string()).icon(IconId::new("find")).tooltip(tl!("Find").to_string());
            menu_button(ui, face, tl!("Find").to_string(), Some("edit.find"), |ui, _| {
                // As in Word: Find searches in the Navigation pane; Advanced Find is the dialog (#712).
                for (label, id, params) in [
                    ("Find", "edit.find", json!({})),
                    ("Advanced Find…", "edit.advancedFind", json!({})),
                    ("Go To…", "ui.dialog", json!({"name": "goto"})),
                ] {
                    MenuItem::command(CommandRef::with_params(id, params)).label(tl!(label).to_string()).show(ui);
                }
            });
            Button::command(CommandRef::with_params("ui.dialog", json!({"name": "replace"})))
                .icon(IconId::new("replace"))
                .label(tl!("Replace").to_string())
                .tooltip(tl!("Replace").to_string())
                .show(ui);
            let face = Button::new(tl!("Select").to_string()).icon(IconId::new("select")).tooltip(tl!("Select").to_string());
            menu_button(ui, face, tl!("Select").to_string(), Some("select.all"), |ui, _| {
                for (label, id) in [("Select All", "select.all"), ("Select Paragraph", "select.paragraph"), ("Select Sentence", "select.sentence")] {
                    MenuItem::command(CommandRef::new(id)).label(tl!(label).to_string()).show(ui);
                }
            });
        });
    });
    row.group(ribbon_group("voice", "Voice", "dictate", None), |ui| {
        Button::command(CommandRef::new("tools.dictate"))
            .icon(IconId::new("dictate"))
            .label(tl!("Dictate").to_string())
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(ribbon_group("editor", "Editor", "editor", None), |ui| {
        Button::command(CommandRef::new("review.spelling"))
            .icon(IconId::new("editor"))
            .label(tl!("Editor").to_string())
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}
