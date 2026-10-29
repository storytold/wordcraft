use super::bookmarks::{bookmark, enclose_preview, icons_body};
use super::chrome::{buttons, color_field};
use super::find::{FindTab, SEARCH_SUGGESTIONS, command_matches, find_in_navigation_pane, goto_body, special_menu, swallow_shortcut_text};
use super::font::{apply_font, preview_size};
use super::forms::{cell_margins_dialog, custom_properties, table_properties};
use super::mailings::{ADDRESS_PARTS, recipient_list, rule_params};
use super::page_setup::{page_layout_commands, page_paper_group, page_valign_combo, page_valign_value, run_as_one_step};
use super::paragraph::{ParaNumber, apply_paragraph, para_check, para_number};
use super::styles::manage_styles;
use super::{Dialog, MeasureExt, Tokens, Ui, Value, WordApp, json, semibold, vec2};

/// Returns true to close.
pub(super) fn body(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    match d {
        Dialog::RenameDocument { .. } => rename_document_dialog(app, ui, d),
        Dialog::Font { .. } => font_dialog(app, ui, d),
        Dialog::Paragraph { .. } => paragraph_dialog(app, ui, d),
        Dialog::Tabs { form } => crate::dialogs_para::tabs(app, ui, form),
        Dialog::Borders { form } => crate::dialogs_para::borders(app, ui, form),
        Dialog::Find { .. } => find_dialog(app, ui, d),
        Dialog::Enclose { .. } => enclose_dialog(app, ui, d),
        Dialog::TableProperties { .. } => table_properties_dialog(app, ui, d),
        Dialog::CellMargins { .. } => cell_margins_arm(app, ui, d),
        Dialog::InsertTable { .. } => insert_table_dialog(app, ui, d),
        Dialog::PageSetup { .. } => page_setup_dialog(app, ui, d),
        Dialog::Link { .. } => link_dialog(app, ui, d),
        Dialog::Bookmark { name, by_location, hidden, deleted } => bookmark(app, ui, name, by_location, hidden, deleted),
        Dialog::WordCount { .. } => word_count_dialog(app, ui, d),
        Dialog::Zoom { .. } => zoom_dialog(app, ui, d),
        Dialog::Watermark { .. } => watermark_dialog(app, ui, d),
        Dialog::Phonetic { .. } => phonetic_dialog(app, ui, d),
        Dialog::NewStyle { .. } => new_style_dialog(app, ui, d),
        Dialog::ModifyStyle { .. } => modify_style_dialog(app, ui, d),
        Dialog::ManageStyles { alphabetical, selected } => manage_styles(app, ui, alphabetical, selected),
        Dialog::TableStyle { .. } => table_style_dialog(app, ui, d),
        Dialog::Insert(f) => crate::dialogs_insert::body(app, ui, f),
        Dialog::Refs(f) => crate::dialogs_refs::body(app, ui, f),
        Dialog::Commands { .. } => commands_dialog(app, ui, d),
        Dialog::Icons { query, category, selected } => icons_body(app, ui, query, category, selected),
        Dialog::PasteSpecial { .. } => paste_special_dialog(app, ui, d),
        Dialog::About { .. } => about_dialog(ui, d),
        Dialog::DiscardRecovery { .. } => discard_recovery_dialog(app, ui, d),
        Dialog::SaveChanges { .. } => save_changes_dialog(app, ui, d),
        Dialog::RecipientList { fields, rows, message, filter } => recipient_list(app, ui, fields, rows, message, filter),
        Dialog::InsertMergeField { .. } => insert_merge_field_dialog(app, ui, d),
        Dialog::Password { .. } => password_dialog(app, ui, d),
        Dialog::EncryptPassword { .. } => encrypt_password_dialog(app, ui, d),
        Dialog::DefineList { form } => crate::dialogs_lists::define_list(app, ui, form),
        Dialog::TrackOptions { form } => crate::dialogs_lists::track_options(app, ui, form),
        Dialog::Review(d) => crate::dialogs_review::body(app, ui, d),
        Dialog::CustomizeKeyboard { form } => crate::dialogs_keyboard::customize_keyboard(app, ui, form),
        Dialog::CustomizeRibbon { form } => crate::dialogs_ribbon::customize_ribbon(app, ui, form),
        Dialog::FieldCode { .. } => field_code_dialog(app, ui, d),
        Dialog::Control { form } => crate::dialogs_controls::body(app, ui, form),
        Dialog::SelectTable { .. } => select_table_dialog(app, ui, d),
        Dialog::NoteOptions { form } => crate::dialogs_notes::note_options(app, ui, form),
        Dialog::Templates { form } => crate::dialogs_templates::templates(app, ui, form),
        Dialog::Organizer { form } => crate::dialogs_templates::organizer(app, ui, form),
        Dialog::CustomProperties(form) => custom_properties(app, ui, form),
        Dialog::Layout(d) => crate::dialogs_layout::body(app, ui, d),
        Dialog::FindRecipient { .. } => find_recipient_dialog(app, ui, d),
        Dialog::MergeRecords { .. } => merge_records_dialog(app, ui, d),
        Dialog::MergeRule { .. } => merge_rule_dialog(app, ui, d),
        Dialog::MatchFields { .. } => match_fields_dialog(ui, d),
        Dialog::CheckErrors { .. } => check_errors_dialog(ui, d),
    }
}

fn rename_document_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::RenameDocument { name, message, focus } = d else {
        return false;
    };
    let label = ui.label(tl!("Document name"));
    let r = ui.add(egui::TextEdit::singleline(name).desired_width(320.0).char_limit(200)).labelled_by(label.id);
    if *focus {
        r.request_focus();
        *focus = false;
    }
    if !message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, message.as_str());
    }
    let (ok, cancel) = buttons(ui, tl!("Rename"));
    if cancel {
        return true;
    }
    if ok && name.trim() == app.title_stem() {
        return true;
    }
    if ok {
        match app.run("file.rename", json!({"name": name})) {
            Ok(_) => return true,
            Err(e) => *message = e,
        }
    }
    false
}

fn font_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Font {
        font,
        font_ea,
        asian,
        size,
        bold,
        italic,
        underline,
        strike,
        sup,
        sub,
        small_caps,
        caps,
        hidden,
        color,
        spacing,
        kern,
        kern_at,
        ..
    } = d
    else {
        return false;
    };
    // Build the sample text format for the preview pane below.
    let preview_size = preview_size(size);
    let preview_color = wordcraft_doc::Rgb::parse(color).map(crate::theme::c32).unwrap_or_else(|| ui.visuals().text_color());
    let mut fmt =
        egui::TextFormat { font_id: egui::FontId::new(preview_size, egui::FontFamily::Proportional), color: preview_color, ..Default::default() };
    if *bold {
        fmt.font_id = crate::theme::semibold(preview_size);
    }
    if *italic {
        fmt.italics = true;
    }
    if *underline {
        fmt.underline = egui::Stroke::new(1.0, preview_color);
    }
    if *strike {
        fmt.strikethrough = egui::Stroke::new(1.0, preview_color);
    }
    egui::Grid::new("fontdlg").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        // Word's two font boxes when East Asian text is in use: the Latin text font and
        // the Asian text font, each with its own list (type to find a font).
        let prev = app.previews.font_preview_fn();
        if *asian {
            ui.label(tl!("Asian text font:"));
            let fams = app.previews.east_asian_families();
            if let Some(f) = crate::widgets::font_combo(ui, "fontdlg_asian", 220.0, font_ea, &fams, Some(&*prev)) {
                if f == crate::widgets::ADD_FONT {
                    let _ = app.run("font.add", json!({}));
                } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                    let _ = app.run("font.loadSystem", json!({}));
                } else {
                    *font_ea = f;
                }
            }
            ui.end_row();
        }
        ui.label(if *asian { tl!("Latin text font:") } else { tl!("Font:") });
        let fams = app.previews.families();
        if let Some(f) = crate::widgets::font_combo(ui, "fontdlg_font", 220.0, font, &fams, Some(&*prev)) {
            if f == crate::widgets::ADD_FONT {
                let _ = app.run("font.add", json!({}));
            } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                let _ = app.run("font.loadSystem", json!({}));
            } else {
                *font = f;
            }
        }
        ui.end_row();
        let size_label = ui.label(tl!("Size:"));
        ui.text_edit_singleline(size).labelled_by(size_label.id);
        ui.end_row();
        ui.label(tl!("Font color:"));
        let theme = app.session.doc.settings.theme_colors.clone();
        color_field(ui, &theme, color, "Automatic", "auto");
        ui.end_row();
        let spacing_label = ui.label(tl!("Spacing (pt):"));
        ui.add(egui::DragValue::new(spacing).speed(0.1).range(-20.0..=20.0)).labelled_by(spacing_label.id);
        ui.end_row();
        let label = tl!("Kerning for fonts:");
        match kern {
            Some(on) => {
                ui.checkbox(on, label);
            }
            None => {
                if ui.add(egui::Checkbox::new(&mut false, label).indeterminate(true)).clicked() {
                    *kern = Some(true);
                }
            }
        }
        ui.horizontal(|ui| {
            let points = ui.add_enabled(*kern == Some(true), egui::DragValue::new(kern_at).speed(0.5).range(1.0..=1638.0).max_decimals(1));
            let points_label = ui.label(tl!("Points and above"));
            points.labelled_by(points_label.id);
        });
        ui.end_row();
    });
    ui.separator();
    ui.label(egui::RichText::new(tl!("Effects")).font(semibold(12.5)));
    egui::Grid::new("fx").num_columns(2).show(ui, |ui| {
        ui.checkbox(bold, tl!("Bold"));
        ui.checkbox(italic, tl!("Italic"));
        ui.end_row();
        ui.checkbox(underline, tl!("Underline"));
        ui.checkbox(strike, tl!("Strikethrough"));
        ui.end_row();
        ui.checkbox(sup, tl!("Superscript"));
        ui.checkbox(sub, tl!("Subscript"));
        ui.end_row();
        ui.checkbox(small_caps, tl!("Small caps"));
        ui.checkbox(caps, tl!("All caps"));
        ui.end_row();
        ui.checkbox(hidden, tl!("Hidden"));
        ui.end_row();
    });
    // Live preview of the chosen font/size/colour/effects.
    ui.separator();
    ui.label(egui::RichText::new(tl!("Preview")).small().weak());
    let sample = if *caps { tl!("AaBbYyZz").to_uppercase() } else { tl!("AaBbYyZz").to_string() };
    let mut job = egui::text::LayoutJob::single_section(sample, fmt);
    job.wrap.max_width = ui.available_width().min(400.0);
    ui.label(job);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        apply_font(app, d);
    }
    ok || cancel
}

fn paragraph_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Paragraph {
        rtl,
        align,
        left,
        right,
        first,
        before,
        after,
        line,
        keep_next,
        keep_lines,
        page_break,
        widow,
        tab,
        asian,
        opts,
        mixed,
        ..
    } = d
    else {
        return false;
    };
    let names = ["Indents and Spacing", "Line and Page Breaks", "Asian Typography"].map(|n| tl!(n).to_string());
    if let Some(k) = crate::widgets::dialog_tabs(ui, &names, *tab as usize) {
        *tab = k as u8;
    }
    ui.add_space(6.0);
    match *tab {
        0 => {
            ui.label(egui::RichText::new(tl!("General")).font(semibold(12.5)));
            ui.horizontal(|ui| {
                ui.label(tl!("Direction:"));
                // Neither while the selected paragraphs differ.
                let blank = mixed.contains("bidi");
                for (value, text) in [(true, tl!("Right-to-left")), (false, tl!("Left-to-right"))] {
                    if ui.radio(!blank && *rtl == value, text).clicked() {
                        *rtl = value;
                        mixed.remove("bidi");
                    }
                }
            });
            const ALIGNS: [(&str, &str); 4] = [("left", "Left"), ("center", "Center"), ("right", "Right"), ("justify", "Justify")];
            ui.horizontal(|ui| {
                let align_label = ui.label(tl!("Alignment:"));
                let shown =
                    if mixed.contains("align") { "" } else { ALIGNS.iter().find(|(v, _)| v == align).map_or(align.as_str(), |(_, l)| tl!(l)) };
                let combo = craft_ui::ComboBox::from_id_salt("para_align").selected_text(shown).show_ui(ui, |ui| {
                    for (v, l) in ALIGNS {
                        if ui.selectable_value(align, v.to_string(), tl!(l)).clicked() {
                            mixed.remove("align");
                        }
                    }
                });
                combo.response.labelled_by(align_label.id);
            });
            ui.label(egui::RichText::new(tl!("Indentation")).font(semibold(12.5)));
            // Mirrored indents are the inside and outside ones (they swap on even pages).
            let (l_label, r_label) = if opts[0] { ("Inside:", "Outside:") } else { ("Left:", "Right:") };
            egui::Grid::new("ind").num_columns(4).show(ui, |ui| {
                let left_label = ui.label(tl!(l_label));
                para_number(ui, left, ParaNumber::Length, "left", mixed, &left_label);
                let right_label = ui.label(tl!(r_label));
                para_number(ui, right, ParaNumber::Length, "right", mixed, &right_label);
                ui.end_row();
                let first_label = ui.label(tl!("First line:"));
                para_number(ui, first, ParaNumber::Length, "indentFirst", mixed, &first_label);
                ui.end_row();
            });
            para_check(ui, &mut opts[0], tl!("Mirror indents"), "mirrorIndents", mixed);
            ui.label(egui::RichText::new(tl!("Spacing")).font(semibold(12.5)));
            egui::Grid::new("sp").num_columns(4).show(ui, |ui| {
                let before_label = ui.label(tl!("Before:"));
                para_number(ui, before, ParaNumber::Points, "spaceBefore", mixed, &before_label);
                let line_label = ui.label(tl!("Line spacing:"));
                para_number(ui, line, ParaNumber::Lines, "lineSpacing", mixed, &line_label);
                ui.end_row();
                let after_label = ui.label(tl!("After:"));
                para_number(ui, after, ParaNumber::Points, "spaceAfter", mixed, &after_label);
                ui.end_row();
            });
            para_check(ui, &mut opts[1], tl!("Don't add space between paragraphs of the same style"), "contextualSpacing", mixed);
        }
        1 => {
            ui.label(egui::RichText::new(tl!("Line and Page Breaks")).font(semibold(12.5)));
            para_check(ui, widow, tl!("Widow/Orphan control"), "widowControl", mixed);
            para_check(ui, keep_next, tl!("Keep with next"), "keepNext", mixed);
            para_check(ui, keep_lines, tl!("Keep lines together"), "keepLines", mixed);
            para_check(ui, page_break, tl!("Page break before"), "pageBreakBefore", mixed);
            ui.label(egui::RichText::new(tl!("Formatting exceptions")).font(semibold(12.5)));
            para_check(ui, &mut opts[2], tl!("Suppress line numbers"), "suppressLineNumbers", mixed);
            para_check(ui, &mut opts[3], tl!("Don't hyphenate"), "suppressHyphens", mixed);
        }
        _ => {
            let [kinsoku, word_wrap, overflow, top_line, de, dn] = asian;
            ui.label(egui::RichText::new(tl!("Line breaking")).font(semibold(12.5)));
            para_check(ui, kinsoku, tl!("Apply Asian line-breaking rules (kinsoku)"), "kinsoku", mixed);
            // The model's flag is "wrap whole words"; the box offers the opposite.
            let mut mid_word = !*word_wrap;
            para_check(ui, &mut mid_word, tl!("Allow Latin text to wrap in the middle of a word"), "wordWrap", mixed);
            *word_wrap = !mid_word;
            para_check(ui, overflow, tl!("Allow hanging punctuation"), "overflowPunct", mixed);
            ui.label(egui::RichText::new(tl!("Character Spacing")).font(semibold(12.5)));
            para_check(ui, top_line, tl!("Compress punctuation at the start of a line"), "topLinePunct", mixed);
            para_check(ui, de, tl!("Add space between Asian and Latin text"), "autoSpaceDE", mixed);
            para_check(ui, dn, tl!("Add space between Asian text and numbers"), "autoSpaceDN", mixed);
        }
    }
    ui.add_space(6.0);
    let tabs = ui.button(tl!("Tabs…")).clicked();
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok || tabs {
        apply_paragraph(app, d);
    }
    // Tabs… keeps what was set here and moves on to the Tabs dialog.
    if tabs {
        app.dialog = Dialog::open("tabs", app);
    }
    ok || cancel || tabs
}

fn find_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Find { query, replace, match_case, whole_word, regex, extra, tab, goto_what, goto_target, message, focus } = d else {
        return false;
    };
    const FIND_TABS: [(FindTab, &str); 3] = [(FindTab::Find, "Find"), (FindTab::Replace, "Replace"), (FindTab::GoTo, "Go To")];
    let names = FIND_TABS.map(|(_, l)| tl!(l).to_string());
    let current = FIND_TABS.iter().position(|(t, _)| t == tab).unwrap_or(0);
    if let Some((t, _)) = crate::widgets::dialog_tabs(ui, &names, current).and_then(|k| FIND_TABS.get(k)) {
        *tab = *t;
        message.clear();
    }
    ui.add_space(6.0);
    if *tab == FindTab::GoTo {
        return goto_body(app, ui, goto_what, goto_target, message);
    }
    let replace_mode = *tab == FindTab::Replace;
    let clipboard = app.services.clipboard_text.as_deref();
    // Per window: two windows on a document can each have the dialog open.
    let (find_what, replace_with) = (app.window_id("find_what"), app.window_id("replace_with"));
    ui.horizontal(|ui| {
        let find_label = ui.label(tl!("Find what:"));
        let edit = egui::TextEdit::singleline(query).desired_width(240.0);
        let r = crate::field_menu::text_field(ui, edit, find_what, clipboard).labelled_by(find_label.id);
        if std::mem::take(focus) {
            r.request_focus();
        }
        if r.gained_focus() {
            extra.to_replace = false;
        }
    });
    if replace_mode {
        ui.horizontal(|ui| {
            let replace_label = ui.label(tl!("Replace with:"));
            let edit = egui::TextEdit::singleline(replace).desired_width(228.0);
            if crate::field_menu::text_field(ui, edit, replace_with, clipboard).labelled_by(replace_label.id).gained_focus() {
                extra.to_replace = true;
            }
        });
    } else {
        extra.to_replace = false;
    }
    // Word greys out Match case and the word options with wildcards (they always match case).
    let plain = !extra.wildcards;
    ui.horizontal(|ui| {
        ui.add_enabled(plain, egui::Checkbox::new(match_case, tl!("Match case")));
        ui.add_enabled(plain, egui::Checkbox::new(whole_word, tl!("Whole words")));
        ui.checkbox(&mut extra.wildcards, tl!("Use wildcards"));
    });
    let arrow = if extra.more { "▾" } else { "▸" };
    if ui.selectable_label(extra.more, format!("{arrow} {}", tl!("Search Options"))).clicked() {
        extra.more = !extra.more;
    }
    if extra.more {
        ui.horizontal(|ui| {
            ui.add_enabled(plain, egui::Checkbox::new(&mut extra.prefix, tl!("Match prefix")));
            ui.add_enabled(plain, egui::Checkbox::new(&mut extra.suffix, tl!("Match suffix")));
        });
        ui.horizontal(|ui| {
            ui.add_enabled(plain, egui::Checkbox::new(&mut extra.ignore_punct, tl!("Ignore punctuation characters")));
            ui.add_enabled(plain, egui::Checkbox::new(&mut extra.ignore_space, tl!("Ignore white-space characters")));
        });
        ui.horizontal(|ui| {
            ui.checkbox(regex, tl!("Regular expressions"));
            if let Some(code) = special_menu(ui, extra) {
                if extra.to_replace { replace.push_str(code) } else { query.push_str(code) }
            }
        });
    }
    let opts = json!({
        "text": query,
        "with": replace,
        "matchCase": *match_case,
        "wholeWord": *whole_word,
        "regex": *regex,
        "wildcards": extra.wildcards,
        "matchPrefix": extra.prefix,
        "matchSuffix": extra.suffix,
        "ignorePunct": extra.ignore_punct,
        "ignoreSpace": extra.ignore_space,
    });
    if !replace_mode {
        ui.horizontal(|ui| {
            let lit = app.session.find.highlight;
            if ui.selectable_label(lit, tl!("Reading Highlight")).clicked() {
                let mut v = opts.clone();
                v["highlight"] = json!(!lit);
                match app.run("edit.advancedFind", v) {
                    Ok(r) if !lit => *message = crate::i18n::fmt(tl!("{count} items highlighted"), &[("count", &r["count"].to_string())]),
                    Ok(_) => message.clear(),
                    Err(e) => *message = e,
                }
            }
        });
    }
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).weak());
    }
    let mut close = false;
    ui.horizontal(|ui| {
        if replace_mode {
            if ui.button(tl!("Replace All")).clicked() {
                match app.run("edit.replaceAll", opts.clone()) {
                    Ok(r) => *message = format!("All done. We made {} replacements.", r["replaced"]),
                    Err(e) => *message = e,
                }
            }
            if ui.button(tl!("Replace")).clicked() {
                let _ = app.run("edit.replace", opts.clone());
            }
        }
        if ui.button(tl!("Find Next")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            // One step per click: `edit.find` would select a match itself, and the step after it skip one.
            match app.run("edit.findNext", opts.clone()) {
                Ok(r) => *message = format!("Match {} of {}", r["index"].as_u64().unwrap_or(0) + 1, r["count"]),
                Err(e) => *message = e,
            }
        }
        if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    close
}

fn enclose_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Enclose { style, text, shape } = d else {
        return false;
    };
    ui.label(egui::RichText::new(tl!("Style")).font(semibold(12.5)));
    ui.horizontal(|ui| {
        for (v, l) in [("none", "None"), ("shrink", "Shrink text"), ("enlarge", "Enlarge symbol")] {
            ui.radio_value(style, v.to_string(), tl!(l));
        }
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Text:"));
            ui.add(egui::TextEdit::singleline(text).desired_width(110.0).char_limit(2));
            ui.add_space(6.0);
            enclose_preview(ui, style, text, shape);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.label(tl!("Enclosure"));
            for (v, l) in [("circle", "Circle"), ("square", "Square"), ("triangle", "Triangle"), ("diamond", "Diamond")] {
                ui.add_enabled_ui(style != "none", |ui| ui.selectable_value(shape, v.to_string(), tl!(l)));
            }
        });
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let params = if style == "none" { json!({"shape": null}) } else { json!({"shape": shape, "style": style, "text": text}) };
        let _ = app.run("format.enclose", params);
    }
    ok || cancel
}

fn table_properties_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::TableProperties { form, basis } = d else {
        return false;
    };
    table_properties(ui, form);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let changes = form.changes(basis);
        if changes.as_object().is_some_and(|m| !m.is_empty()) {
            let _ = app.run("table.properties", changes);
        }
    }
    ok || cancel
}

fn cell_margins_arm(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::CellMargins { form, basis } = d else {
        return false;
    };
    cell_margins_dialog(ui, form);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let changes = form.changes(basis);
        if changes.as_object().is_some_and(|m| !m.is_empty()) {
            let _ = app.run("table.cellMargins", changes);
        }
    }
    ok || cancel
}

fn insert_table_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::InsertTable { rows, cols } = d else {
        return false;
    };
    egui::Grid::new("it").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Number of columns:"));
        ui.add(egui::DragValue::new(cols).range(1..=63));
        ui.end_row();
        ui.label(tl!("Number of rows:"));
        ui.add(egui::DragValue::new(rows).range(1..=1000));
        ui.end_row();
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let _ = app.run("insert.table", json!({"rows": *rows, "cols": *cols}));
    }
    ok || cancel
}

fn page_setup_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::PageSetup {
        top,
        bottom,
        left,
        right,
        landscape,
        mirror,
        rtl,
        grid,
        grid_was,
        paper,
        paper_was,
        paper_custom,
        message,
        valign,
        odd_even,
        first_page,
    } = d
    else {
        return false;
    };
    ui.label(egui::RichText::new(tl!("Margins")).font(semibold(12.5)));
    egui::Grid::new("ps").num_columns(4).show(ui, |ui| {
        let top_label = ui.label(tl!("Top:"));
        ui.add(egui::DragValue::new(top).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2))
            .labelled_by(top_label.id);
        let bottom_label = ui.label(tl!("Bottom:"));
        ui.add(egui::DragValue::new(bottom).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2))
            .labelled_by(bottom_label.id);
        ui.end_row();
        let left_label = ui.label(if *mirror { tl!("Inside:") } else { tl!("Left:") });
        ui.add(egui::DragValue::new(left).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2))
            .labelled_by(left_label.id);
        let right_label = ui.label(if *mirror { tl!("Outside:") } else { tl!("Right:") });
        ui.add(egui::DragValue::new(right).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2))
            .labelled_by(right_label.id);
        ui.end_row();
    });
    ui.label(egui::RichText::new(tl!("Orientation")).font(semibold(12.5)));
    ui.horizontal(|ui| {
        ui.radio_value(landscape, false, tl!("Portrait"));
        ui.radio_value(landscape, true, tl!("Landscape"));
    });
    ui.label(egui::RichText::new(tl!("Pages")).font(semibold(12.5)));
    ui.checkbox(mirror, tl!("Mirror margins"));
    page_paper_group(ui, paper, landscape, paper_custom);
    ui.label(egui::RichText::new(tl!("Document Grid")).font(semibold(12.5)));
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut grid.0, 0, tl!("No grid"));
        ui.radio_value(&mut grid.0, 1, tl!("Lines only"));
        ui.radio_value(&mut grid.0, 2, tl!("Lines and characters"));
        ui.radio_value(&mut grid.0, 3, tl!("Text snaps to characters"));
    });
    egui::Grid::new("ps_grid").num_columns(4).show(ui, |ui| {
        let chars_label = ui.add_enabled(grid.0 >= 2, egui::Label::new(tl!("Characters per line:")));
        ui.add_enabled(grid.0 >= 2, egui::DragValue::new(&mut grid.2).range(1..=1000)).labelled_by(chars_label.id);
        let lines_label = ui.add_enabled(grid.0 >= 1, egui::Label::new(tl!("Lines per page:")));
        ui.add_enabled(grid.0 >= 1, egui::DragValue::new(&mut grid.1).range(1..=1000)).labelled_by(lines_label.id);
        ui.end_row();
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Section direction:"));
        ui.radio_value(rtl, false, tl!("Left-to-right"));
        ui.radio_value(rtl, true, tl!("Right-to-left"));
    });
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
    }
    page_valign_combo(ui, valign);
    // Word's Page Setup › Layout: a title page without the page number (#1230).
    ui.label(egui::RichText::new(tl!("Headers and footers")).font(semibold(12.5)));
    ui.checkbox(odd_even, tl!("Different Odd & Even Pages"));
    ui.checkbox(first_page, tl!("Different First Page"));
    let (mut ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        // Everything OK changes is one undo step; a refused command applies nothing.
        let result = page_layout_commands(app, (*paper, *paper_was), *landscape, (*top, *bottom, *left, *right))
            .map_err(|e| tl!(e).to_string())
            .and_then(|mut commands| {
                // Mirror margins go with the margins (#1181).
                if let Some((_, v)) = commands.iter_mut().rev().find(|(id, _)| *id == "layout.margins") {
                    v["mirror"] = json!(*mirror);
                }
                if grid != grid_was {
                    let kind = ["none", "lines", "linesAndChars", "snapToChars"].get(grid.0 as usize).copied().unwrap_or("none");
                    let mut v = json!({"type": kind});
                    if grid.1 != grid_was.1 {
                        v["linesPerPage"] = json!(grid.1);
                    }
                    if grid.2 != grid_was.2 {
                        v["charsPerLine"] = json!(grid.2);
                    }
                    commands.push(("layout.documentGrid", v));
                }
                if *rtl != wordcraft_engine::cmd::page::sect(&app.session).rtl {
                    commands.push(("layout.sectionRtl", json!({"value": *rtl})));
                }
                if *valign != wordcraft_engine::cmd::page::sect(&app.session).valign {
                    commands.push(("layout.verticalAlign", json!({"value": page_valign_value(*valign)})));
                }
                if *odd_even != app.session.doc.settings.even_odd_headers {
                    commands.push(("layout.differentOddEven", json!({"value": *odd_even})));
                }
                if *first_page != wordcraft_engine::cmd::page::sect(&app.session).title_page {
                    commands.push(("layout.differentFirstPage", json!({"value": *first_page})));
                }
                run_as_one_step(app, commands)
            });
        if let Err(e) = result {
            *message = e;
            ok = false;
        }
    }
    ok || cancel
}

fn link_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Link { url, text, place, targets, pick, focus } = d else {
        return false;
    };
    // Word's "Link to:" choices; the places are read once, when that tab is first shown.
    ui.horizontal(|ui| {
        ui.label(tl!("Link to:"));
        ui.selectable_value(place, false, tl!("Existing File or Web Page"));
        if ui.selectable_value(place, true, tl!("Place in This Document")).clicked() && targets.is_none() {
            *targets = app.run("link.targets", json!({})).ok();
        }
    });
    ui.add_space(4.0);
    egui::Grid::new("lnk").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Text to display:"));
        ui.text_edit_singleline(text);
        ui.end_row();
        if !*place {
            ui.label(tl!("Address:"));
            let mut out = egui::TextEdit::singleline(url).id(egui::Id::unique("link_address")).show(ui);
            if std::mem::take(focus) {
                out.response.request_focus();
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(url.chars().count()));
                out.state.cursor.set_char_range(Some(all));
                out.state.store(ui.ctx(), out.response.id);
            }
            ui.end_row();
        }
    });
    if *place {
        ui.add_space(4.0);
        ui.label(tl!("Select a place in this document:"));
        let mut chosen: Option<(Value, String)> = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(220.0).min_scrolled_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                let is = |pick: &Option<(Value, String)>, key: &str, val: &Value| pick.as_ref().is_some_and(|(v, _)| v.get(key) == Some(val));
                let top = tl!("Top of the Document").to_string();
                if ui.selectable_label(is(pick, "top", &json!(true)), &top).clicked() {
                    chosen = Some((json!({"top": true}), top));
                }
                let headings = targets.as_ref().and_then(|t| t.get("headings")).and_then(Value::as_array).cloned().unwrap_or_default();
                if !headings.is_empty() {
                    ui.label(egui::RichText::new(tl!("Headings")).font(semibold(12.5)));
                }
                for h in &headings {
                    let (block, level) = (h.get("block").cloned().unwrap_or_default(), h.get("level").and_then(Value::as_u64).unwrap_or(1));
                    let name = h.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
                    ui.horizontal(|ui| {
                        ui.add_space(14.0 * level.saturating_sub(1).min(8) as f32 + 8.0);
                        if ui.selectable_label(is(pick, "heading", &block), &name).clicked() {
                            chosen = Some((json!({"heading": block}), name));
                        }
                    });
                }
                let marks = targets.as_ref().and_then(|t| t.get("bookmarks")).and_then(Value::as_array).cloned().unwrap_or_default();
                if !marks.is_empty() {
                    ui.label(egui::RichText::new(tl!("Bookmarks")).font(semibold(12.5)));
                }
                for m in marks.iter().filter_map(Value::as_str) {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        if ui.selectable_label(is(pick, "bookmark", &json!(m)), m).clicked() {
                            chosen = Some((json!({"bookmark": m}), m.to_string()));
                        }
                    });
                }
            });
        });
        if let Some(c) = chosen {
            // The text shown follows the choice until it is typed over (as in Word).
            if text.is_empty() || pick.as_ref().is_some_and(|(_, old)| old == text) {
                *text = c.1.clone();
            }
            *pick = Some(c);
        }
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        if *place {
            let Some((mut params, _)) = pick.clone() else { return false };
            if !text.is_empty() {
                params["text"] = json!(text);
            }
            let _ = app.run("insert.link", params);
        } else {
            let _ = app.run("insert.link", json!({"url": url, "text": text}));
        }
    }
    ok || cancel
}

fn word_count_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::WordCount { stats } = d else {
        return false;
    };
    egui::Grid::new("wc").num_columns(2).spacing(vec2(30.0, 6.0)).show(ui, |ui| {
        for (l, k) in [
            ("Pages", "pages"),
            ("Words", "words"),
            ("Characters (no spaces)", "characters"),
            ("Characters (with spaces)", "charactersWithSpaces"),
            ("Paragraphs", "paragraphs"),
            ("Lines", "lines"),
        ] {
            ui.label(tl!(l));
            ui.label(stats.get(k).map(|v| v.to_string()).unwrap_or_default());
            ui.end_row();
        }
    });
    let mut include = stats.get("includeTextBoxes").and_then(Value::as_bool).unwrap_or(true);
    if ui.checkbox(&mut include, tl!("Count words in text boxes and notes")).changed()
        && let Ok(v) = app.run("review.wordCount", json!({"includeTextBoxes": include}))
    {
        *stats = v;
    }
    let (ok, cancel) = buttons(ui, tl!("Close"));
    ok || cancel
}

fn zoom_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Zoom { percent } = d else {
        return false;
    };
    ui.horizontal(|ui| {
        for p in [200.0, 100.0, 75.0] {
            ui.radio_value(percent, p, format!("{p}%"));
        }
    });
    // Logarithmic like the status bar slider, and wide enough to aim: the default 100 pt
    // linear slider jumped from 100% to 480% within a short drag (issue #67).
    ui.spacing_mut().slider_width = 220.0;
    ui.add(egui::Slider::new(percent, 10.0..=500.0).logarithmic(true).step_by(1.0).suffix("%"));
    let mut fit = None;
    ui.horizontal(|ui| {
        if ui.button(tl!("Page width")).clicked() {
            fit = Some("pageWidth");
        }
        if ui.button(tl!("Whole page")).clicked() {
            fit = Some("onePage");
        }
        if ui.button(tl!("Many pages")).clicked() {
            fit = Some("multiplePages");
        }
    });
    if let Some(f) = fit {
        let _ = app.run("view.zoom", json!({"value": f}));
        return true;
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let _ = app.run("view.zoom", json!({"value": *percent}));
    }
    ok || cancel
}

fn watermark_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Watermark { text, diagonal } = d else {
        return false;
    };
    ui.horizontal(|ui| {
        ui.label(tl!("Text:"));
        ui.text_edit_singleline(text);
    });
    ui.horizontal(|ui| {
        ui.radio_value(diagonal, true, tl!("Diagonal"));
        ui.radio_value(diagonal, false, tl!("Horizontal"));
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let _ = app.run("design.watermark", json!({"text": text, "diagonal": *diagonal}));
    }
    ok || cancel
}

fn phonetic_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Phonetic { base, ruby, align, size, offset, existing, focus } = d else {
        return false;
    };
    // Base and ruby text are usually Chinese or Japanese: fonts for them load from the next frame.
    app.want_system_cjk = true;
    const ALIGNS: [(&str, &str); 6] = [
        ("center", "Center"),
        ("distributeLetter", "Distribute letters"),
        ("distributeSpace", "Distribute spacing"),
        ("left", "Left"),
        ("right", "Right"),
        ("rightVertical", "Right (vertical)"),
    ];
    egui::Grid::new("phonetic").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Base text:"));
        ui.label(egui::RichText::new(base.as_str()).strong());
        ui.end_row();
        let ruby_label = ui.label(tl!("Ruby text:"));
        let r = ui.text_edit_singleline(ruby).labelled_by(ruby_label.id);
        // Typing goes to the Ruby text box, not the selected text in the document (#3914).
        if std::mem::take(focus) {
            r.request_focus();
        }
        ui.end_row();
        let align_label = ui.label(tl!("Alignment:"));
        let shown = ALIGNS.iter().find(|(k, _)| k == align).map_or("Center", |(_, l)| l);
        let combo = craft_ui::ComboBox::from_id_salt("phonetic_align").selected_text(tl!(shown)).show_ui(ui, |ui| {
            for (k, l) in ALIGNS {
                ui.selectable_value(align, k.to_string(), tl!(l));
            }
        });
        combo.response.labelled_by(align_label.id);
        ui.end_row();
        let offset_label = ui.label(tl!("Offset (pt):"));
        ui.add(egui::DragValue::new(offset).speed(0.5).range(-100.0..=100.0)).labelled_by(offset_label.id);
        ui.end_row();
        let size_label = ui.label(tl!("Size (pt):"));
        ui.add(egui::DragValue::new(size).speed(0.5).range(1.0..=200.0)).labelled_by(size_label.id);
        ui.end_row();
    });
    let mut remove = false;
    if *existing {
        ui.add_space(4.0);
        remove = ui.button(tl!("Remove")).clicked();
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if remove {
        let _ = app.run("format.phonetic", json!({"ruby": null}));
    } else if ok {
        let _ = app.run("format.phonetic", json!({"ruby": ruby, "align": align, "size": *size, "offset": *offset}));
    }
    ok || cancel || remove
}

fn new_style_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::NewStyle { name, based_on, back } = d else {
        return false;
    };
    egui::Grid::new("ns").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Name:"));
        ui.text_edit_singleline(name);
        ui.end_row();
        ui.label(tl!("Style based on:"));
        ui.text_edit_singleline(based_on);
        ui.end_row();
    });
    ui.label(egui::RichText::new(tl!("The new style takes the formatting of the current paragraph.")).small().weak());
    let (ok, cancel) = buttons(ui, tl!("OK"));
    let mut created = None;
    if ok {
        created = app.run("styles.create", json!({"name": name, "basedOn": based_on})).ok().and_then(|v| v.get("id")?.as_str().map(str::to_string));
    }
    if (ok || cancel) && *back {
        app.dialog = Some(Dialog::ManageStyles { alphabetical: false, selected: created.unwrap_or_default() });
    }
    ok || cancel
}

fn modify_style_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::ModifyStyle { back, resume, id, name, font, size, bold, italic, color, before, after } = d else {
        return false;
    };
    egui::Grid::new("ms").num_columns(2).show(ui, |ui| {
        let name_label = ui.label(tl!("Name:"));
        ui.text_edit_singleline(name).labelled_by(name_label.id);
        ui.end_row();
        ui.label(tl!("Font:"));
        let shown = app.previews.font_shown(font);
        let fams = app.previews.families();
        let prev = app.previews.font_preview_fn();
        if let Some(f) = crate::widgets::font_combo(ui, "ms_font", 220.0, &shown, &fams, Some(&*prev)) {
            if f == crate::widgets::ADD_FONT {
                let _ = app.run("font.add", json!({}));
            } else {
                *font = app.previews.font_stored(&f);
            }
        }
        ui.end_row();
        let size_label = ui.label(tl!("Size:"));
        ui.add(egui::DragValue::new(size).range(1.0..=1638.0).speed(0.5)).labelled_by(size_label.id);
        ui.end_row();
        let color_label = ui.label(tl!("Color:"));
        ui.text_edit_singleline(color).labelled_by(color_label.id);
        ui.end_row();
        ui.label(tl!("Space before/after:"));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(before).range(0.0..=1584.0)).accessible_name(tl!("Space before"));
            ui.add(egui::DragValue::new(after).range(0.0..=1584.0)).accessible_name(tl!("Space after"));
        });
        ui.end_row();
    });
    ui.horizontal(|ui| {
        ui.checkbox(bold, tl!("Bold"));
        ui.checkbox(italic, tl!("Italic"));
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let mut chr = json!({"font": font, "size": *size, "bold": *bold, "italic": *italic});
        if let Some(c) = wordcraft_doc::Rgb::parse(color) {
            chr["color"] = json!({"Rgb": [c.0, c.1, c.2]});
        }
        let _ = app.run("styles.modify", json!({"style": id, "name": name, "chr": chr, "para": {"spaceBefore": *before, "spaceAfter": *after}}));
    }
    if (ok || cancel) && *back {
        app.dialog = Some(Dialog::ManageStyles { alphabetical: false, selected: id.clone() });
    }
    if (ok || cancel)
        && let Some(r) = resume.take()
    {
        app.dialog = Some(*r);
    }
    ok || cancel
}

fn table_style_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::TableStyle { id, name, based_on, region, regions, basis, pen } = d else {
        return false;
    };
    crate::dialogs_table_style::body(app, ui, crate::dialogs_table_style::Form { id, name, based_on, region, regions, basis, pen })
}

fn commands_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Commands { query, swallow } = d else {
        return false;
    };
    if *swallow {
        ui.ctx().input_mut(|i| i.events.retain(|e| !swallow_shortcut_text(swallow, e)));
    }
    let r = ui.add(egui::TextEdit::singleline(query).hint_text(tl!("Type a command, e.g. \"insert table\"")).desired_width(380.0));
    // Only when it lacks focus: `request_focus` always interrupts IME, and on X11 that
    // recreates the XIM context, so asking every frame left fcitx taking no keys (#2986).
    if !r.has_focus() {
        r.request_focus();
    }
    let reg = app.session.registry.clone();
    // An empty box suggests a few common commands, as Word's Search does.
    let mut hits: Vec<&wordcraft_engine::CommandSpec> = if query.trim().is_empty() {
        ui.label(egui::RichText::new(tl!("Suggested")).font(semibold(12.5)));
        SEARCH_SUGGESTIONS.iter().filter_map(|id| reg.get(id)).collect()
    } else {
        reg.all().iter().filter(|c| command_matches(c, query)).collect()
    };
    hits.truncate(14);
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut run: Option<&str> = enter.then(|| hits.first().map(|c| c.id)).flatten();
    for c in &hits {
        let sc = crate::widgets::shortcut_text(app, c.id);
        let label = format!("{}   —   {}", tl!(c.label), crate::i18n::location(c.location));
        if ui.add(egui::Button::new(label).shortcut_text(sc).min_size(vec2(380.0, 0.0))).clicked() {
            run = Some(c.id);
        }
    }
    // Search also finds the words in the document (the Navigation pane's results).
    let text = query.trim().to_string();
    let mut find = false;
    if !text.is_empty() {
        ui.separator();
        let label = crate::i18n::fmt(tl!("Find “{text}” in the document"), &[("text", &text)]);
        find = ui.add(egui::Button::new(label).min_size(vec2(380.0, 0.0))).clicked() || (enter && hits.is_empty());
    }
    if let Some(id) = run {
        let _ = app.run(id, json!({}));
    } else if find {
        find_in_navigation_pane(app, ui.ctx(), &text);
    }
    run.is_some() || find || ui.input(|i| i.key_pressed(egui::Key::Escape))
}

fn paste_special_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::PasteSpecial { formats, choice, payload, note } = d else {
        return false;
    };
    ui.label(egui::RichText::new(tl!("Paste as:")).font(semibold(12.5)));
    if formats.is_empty() {
        // The engine's wording, sentence-cased; the format names in it aren't translated.
        let text = if note.is_empty() || note == "the clipboard is empty" {
            tl!("The clipboard is empty.").to_string()
        } else {
            let mut c = note.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>() + ".").unwrap_or_default()
        };
        ui.label(egui::RichText::new(text).weak());
    }
    for (i, f) in formats.iter().enumerate() {
        let label = wordcraft_engine::cmd::paste::FORMATS.iter().find(|(id, _)| id == f).map(|(_, l)| *l).unwrap_or(f.as_str());
        ui.radio_value(choice, i, tl!(label));
    }
    if let Some(f) = formats.get(*choice) {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("Result")).font(semibold(12.5)));
        let hint = match f.as_str() {
            "formatted" => "Keeps the fonts and formatting of the copied text.",
            "rtf" => "Reads the clipboard's rich text and keeps its formatting.",
            "html" => "Reads the clipboard's web content and keeps its formatting.",
            "picture" => "Pastes the clipboard's picture.",
            _ => "Drops all formatting and pastes plain text.",
        };
        ui.label(egui::RichText::new(tl!(hint)).weak());
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Some(f) = formats.get(*choice) {
        let mut params = payload.clone();
        params["as"] = json!(f);
        let _ = app.run("edit.pasteSpecial", params);
    }
    ok || cancel
}

fn about_dialog(ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::About { tab } = d else {
        return false;
    };
    ui.set_width(660.0);
    let names = ["About", "Contributors", "Models"].map(|n| tl!(n).to_string());
    if let Some(i) = crate::widgets::dialog_tabs(ui, &names, *tab as usize) {
        *tab = i as u8;
    }
    ui.add_space(6.0);
    match *tab {
        1 => crate::credits::contributors_ui(ui),
        2 => crate::credits::models_ui(ui),
        _ => {
            ui.label(egui::RichText::new(tl!("WordCraft")).font(semibold(22.0)));
            let build = option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or(tl!("development build"));
            ui.label(crate::i18n::fmt(tl!("Version {version} ({build})"), &[("version", env!("CARGO_PKG_VERSION")), ("build", build)]));
            ui.label(tl!("A free, open-source word processor written from scratch in Rust.\nPart of the Crafting Apps from the ArtCraft team."));
            ui.add_space(6.0);
            ui.hyperlink_to(tl!("getartcraft.com/apps/wordcraft"), "https://getartcraft.com/apps/wordcraft");
            ui.hyperlink_to(tl!("Join us on Discord: discord.gg/artcraft"), "https://discord.gg/artcraft");
            ui.hyperlink_to(tl!("Source code: github.com/storytold/wordcraft"), "https://github.com/storytold/wordcraft");
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!("MIT OR Apache-2.0. Copyright (c) 2026 ArtCraft Team and the WordCraft contributors.")).small().weak());
        }
    }
    let (ok, cancel) = buttons(ui, tl!("Close"));
    ok || cancel
}

fn discard_recovery_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::DiscardRecovery { name, .. } = d else {
        return false;
    };
    ui.label(crate::i18n::fmt(tl!("Discard the recovery copy of {name}?"), &[("name", name)]));
    ui.label(tl!("This cannot be undone. Your original file stays unchanged."));
    let answer = ui
        .horizontal(|ui| {
            let cancel = ui.button(tl!("Cancel"));
            let discard = ui.button(tl!("Discard"));
            if cancel.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                Some("cancel")
            } else if discard.clicked() {
                Some("discard")
            } else {
                None
            }
        })
        .inner;
    if let Some(answer) = answer {
        app.dialog = Some(d.clone());
        let result = app.run("ui.recoveryDiscard", json!({"answer": answer}));
        if let Err(e) = &result {
            app.status(e.clone());
        }
        return result.is_ok();
    }
    false
}

fn save_changes_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::SaveChanges { name, message, .. } = d else {
        return false;
    };
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Do you want to save changes to {name}?"), &[("name", name)])).font(semibold(15.0)));
    ui.label(tl!("Your changes will be lost if you don't save them."));
    // Why the last Save failed (#1445): the prompt came back so the user can't miss it.
    if !message.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(message.as_str()).color(ui.visuals().error_fg_color));
    }
    ui.add_space(8.0);
    let buttons = ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cancel = ui.button(tl!("Cancel"));
            let dont_save = ui.button(tl!("Don't Save"));
            let save = ui.add(crate::widgets::primary_button(ui, tl!("Save")));
            [(cancel, "cancel"), (dont_save, "dontSave"), (save, "save")]
        })
        .inner
    });
    let buttons = buttons.inner;
    // The keyboard alone answers, as in Word (#861): Save has focus when the prompt opens,
    // so Enter saves, and the arrow keys (egui's focus navigation) or Tab move between the
    // buttons, shown as Save, Don't Save, Cancel. The focus is taken from whatever held it,
    // not only from nothing: the page keeps it while typing and claims Tab and the arrows
    // (#1597).
    if !buttons.iter().any(|(b, _)| b.has_focus())
        && let Some((save, _)) = buttons.get(2)
    {
        save.request_focus();
    }
    // Enter clicks the focused button; it means Save only when none of them has focus.
    let focused = buttons.iter().any(|(b, _)| b.has_focus());
    let answer = if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        Some("cancel")
    } else if let Some((_, a)) = buttons.iter().find(|(b, _)| b.clicked()) {
        Some(*a)
    } else if !focused && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        Some("save")
    } else {
        None
    };
    let Some(answer) = answer else { return false };
    // `ui.saveChanges` reads the prompt from `app.dialog`, which `show` has taken out.
    app.dialog = Some(d.clone());
    let _ = app.run("ui.saveChanges", json!({"answer": answer}));
    true
}

fn insert_merge_field_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::InsertMergeField { fields, field } = d else {
        return false;
    };
    if fields.is_empty() {
        ui.label(egui::RichText::new(tl!("Select recipients first to choose from their fields.")).small().weak());
    } else {
        ui.label(tl!("Fields:"));
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for f in fields.iter() {
                let resp = ui.selectable_label(field == f, f);
                if resp.clicked() {
                    field.clone_from(f);
                }
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Field:"));
        ui.text_edit_singleline(field);
    });
    let (ok, cancel) = buttons(ui, tl!("Insert"));
    if ok && !field.trim().is_empty() {
        let _ = app.run("mailings.insertField", json!({"field": field.trim()}));
        return true;
    }
    cancel
}

fn password_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::Password { name, params, password, message } = d else {
        return false;
    };
    ui.label(crate::i18n::fmt(tl!("{name} is protected with a password."), &[("name", name)]));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Password:"));
        let r = ui.add(egui::TextEdit::singleline(password.as_mut_string()).password(true).desired_width(220.0));
        if password.is_empty() && !r.has_focus() {
            r.request_focus();
        }
    });
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && !password.is_empty() {
        let mut p = params.clone();
        if let Some(o) = p.as_object_mut() {
            o.insert("password".into(), json!(password.as_str()));
        }
        // `execute`, not `run`: Save Changes was asked when the open began.
        match app.execute("file.open", p) {
            Ok(_) => {
                app.ui.backstage = false;
                return true;
            }
            Err(e) if wordcraft_engine::io::wrong_password(&e) => {
                *message = tl!("That password isn't right. Check it (passwords are case-sensitive) and try again.").to_string();
                password.as_mut_string().clear();
            }
            Err(e) => *message = e,
        }
    }
    cancel
}

fn encrypt_password_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::EncryptPassword { password, confirm, message } = d else {
        return false;
    };
    ui.label(tl!("Anyone who opens the document will need this password."));
    ui.add_space(4.0);
    egui::Grid::new("encrypt_password").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Password:"));
        let r = ui.add(egui::TextEdit::singleline(password.as_mut_string()).password(true).desired_width(220.0));
        if password.is_empty() && confirm.is_empty() && !r.has_focus() {
            r.request_focus();
        }
        ui.end_row();
        ui.label(tl!("Confirm password:"));
        ui.add(egui::TextEdit::singleline(confirm.as_mut_string()).password(true).desired_width(220.0));
        ui.end_row();
    });
    ui.label(
        egui::RichText::new(tl!(
            "Keep the password somewhere safe: without it nobody, WordCraft included, can open the document. Leave both boxes empty to remove the password."
        ))
        .small()
        .weak(),
    );
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        if password != confirm {
            *message = tl!("The passwords don't match.").to_string();
            confirm.as_mut_string().clear();
            return false;
        }
        let pw = if password.is_empty() { Value::Null } else { json!(password.as_str()) };
        match app.run("file.encrypt", json!({"password": pw})) {
            Ok(_) => {
                app.status(if password.is_empty() {
                    tl!("The password is removed. Save the document to keep the change.")
                } else {
                    tl!("The document will be saved with a password. Save it to protect it.")
                });
                return true;
            }
            Err(e) => *message = e,
        }
    }
    cancel
}

fn field_code_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::FieldCode { code, edit, message } = d else {
        return false;
    };
    ui.label(tl!("Field codes:"));
    let r = ui.add(egui::TextEdit::singleline(code).desired_width(320.0).font(egui::TextStyle::Monospace));
    if !r.has_focus() && code.is_empty() {
        r.request_focus();
    }
    ui.label(egui::RichText::new("PAGE · NUMPAGES · DATE \\@ \"MMMM d, yyyy\" · AUTHOR · TITLE · SEQ Figure").small().weak());
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).small().weak());
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && !code.trim().is_empty() {
        let r = if *edit { app.run("fields.setCode", json!({"code": code.trim()})) } else { app.run("insert.field", json!({"instr": code.trim()})) };
        match r {
            Ok(_) => return true,
            Err(e) => *message = e,
        }
    }
    cancel
}

fn select_table_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::SelectTable { sheets, selected, headers, message, params } = d else {
        return false;
    };
    ui.label(tl!("Choose the sheet that holds the recipients."));
    ui.add_space(4.0);
    // A double-click picks the sheet as OK does.
    let mut picked = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(300.0);
        egui::ScrollArea::vertical().id_salt("select_table").max_height(220.0).show(ui, |ui| {
            for (i, name) in sheets.iter().enumerate() {
                let r = ui.selectable_label(*selected == i, name.as_str());
                if r.clicked() || r.double_clicked() {
                    *selected = i;
                }
                picked |= r.double_clicked();
            }
        });
    });
    ui.checkbox(headers, tl!("First row of data contains column headers"));
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).color(Tokens::get(ui.ctx()).red));
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok || picked {
        let mut p = params.clone();
        if let Some(o) = p.as_object_mut() {
            o.insert("sheet".into(), json!(*selected + 1));
            o.insert("headers".into(), json!(*headers));
        }
        match app.load_recipients(p) {
            Ok(_) => return true,
            Err(e) => *message = e,
        }
    }
    cancel
}

fn find_recipient_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::FindRecipient { text, message } = d else {
        return false;
    };
    ui.horizontal(|ui| {
        ui.label(tl!("Find:"));
        ui.text_edit_singleline(text);
    });
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).small().weak());
    }
    let (ok, cancel) = buttons(ui, tl!("Find"));
    if ok && !text.trim().is_empty() {
        match app.run("mailings.findRecipient", json!({"text": text.trim()})) {
            Ok(_) => return true,
            Err(e) => *message = e,
        }
    }
    cancel
}

fn merge_records_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::MergeRecords { print, choice, from, to, message } = d else {
        return false;
    };
    ui.radio_value(choice, 0, tl!("All"));
    ui.radio_value(choice, 1, tl!("Current record"));
    ui.horizontal(|ui| {
        let from_label = ui.radio_value(choice, 2, tl!("From:"));
        // Typing a number picks the range, as in Word.
        if ui.add(egui::TextEdit::singleline(from).desired_width(60.0)).labelled_by(from_label.id).changed() {
            *choice = 2;
        }
        let to_label = ui.label(tl!("To:"));
        if ui.add(egui::TextEdit::singleline(to).desired_width(60.0)).labelled_by(to_label.id).changed() {
            *choice = 2;
        }
    });
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).small().weak());
    }
    let (ok, cancel) = buttons(ui, tl!(if *print { "Print" } else { "OK" }));
    if ok {
        let params = match *choice {
            0 => Some(json!({"records": "all"})),
            1 => Some(json!({"records": "current"})),
            _ => match (from.trim().parse::<u64>(), to.trim().parse::<u64>()) {
                (Ok(f), Ok(t)) => Some(json!({"from": f, "to": t})),
                _ => None,
            },
        };
        let id = if *print { "mailings.printDocuments" } else { "mailings.finish" };
        // An empty range is reported here, before Save Changes asks about the document.
        let run = params.ok_or_else(|| tl!("Enter a number.").to_string()).and_then(|p| {
            wordcraft_engine::cmd::mailings::merge_range(&app.session, &p).map_err(|e| e.to_string())?;
            app.run(id, p)
        });
        match run {
            Ok(_) => return true,
            Err(e) => *message = e,
        }
    }
    cancel
}

fn merge_rule_dialog(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::MergeRule { rule, fields, field, value, then, els } = d else {
        return false;
    };
    let skip = rule == "SKIPIF";
    ui.label(tl!(if skip {
        "Leave a recipient out of the merge when a field has this value."
    } else {
        "Insert one text or another, depending on a field's value."
    }));
    ui.add_space(4.0);
    egui::Grid::new("merge_rule").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Field:"));
        if fields.is_empty() {
            ui.text_edit_singleline(field);
        } else {
            craft_ui::ComboBox::from_id_salt("merge_rule_field").selected_text(field.as_str()).width(200.0).show_ui(ui, |ui| {
                for f in fields.iter() {
                    ui.selectable_value(field, f.clone(), f);
                }
            });
        }
        ui.end_row();
        ui.label(tl!("Equals:"));
        ui.text_edit_singleline(value);
        ui.end_row();
        if !skip {
            ui.label(tl!("Then insert:"));
            ui.text_edit_singleline(then);
            ui.end_row();
            ui.label(tl!("Otherwise insert:"));
            ui.text_edit_singleline(els);
            ui.end_row();
        }
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    let params = rule_params(d);
    if ok && let Some(params) = params {
        let _ = app.run("mailings.rules", params);
        return true;
    }
    cancel
}

fn match_fields_dialog(ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::MatchFields { fields, address } = d else {
        return false;
    };
    if fields.is_empty() {
        ui.label(egui::RichText::new(tl!("Select recipients first to match their fields.")).small().weak());
    } else {
        ui.label(tl!("The recipient-list field that fills each part of an address block and greeting line."));
        ui.add_space(4.0);
        egui::Grid::new("match_fields").num_columns(2).spacing(vec2(24.0, 6.0)).striped(true).show(ui, |ui| {
            for (part, key) in ADDRESS_PARTS {
                ui.label(tl!(part));
                match address.get(key).and_then(Value::as_str) {
                    Some(f) => ui.label(egui::RichText::new(f).strong()),
                    None => ui.label(egui::RichText::new(tl!("(not matched)")).weak()),
                };
                ui.end_row();
            }
        });
    }
    let (ok, cancel) = buttons(ui, tl!("Close"));
    ok || cancel
}

fn check_errors_dialog(ui: &mut Ui, d: &mut Dialog) -> bool {
    let Dialog::CheckErrors { unknown, records } = d else {
        return false;
    };
    if unknown.is_empty() {
        ui.label(tl!("No errors found: every merge field is in the recipient list."));
    } else {
        ui.label(tl!("These merge fields aren't in the recipient list:"));
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            for f in unknown.iter() {
                ui.label(egui::RichText::new(format!("• {f}")).color(Tokens::get(ui.ctx()).red));
            }
        });
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Recipients: {count}"), &[("count", &records.to_string())])).weak());
    let (ok, cancel) = buttons(ui, tl!("Close"));
    ok || cancel
}
