//! File › Options (#485): a list of panes on the left (General, Display, Proofing, Save,
//! Language, Accessibility, Advanced, Agents), the chosen pane's settings on the right. Only
//! settings WordCraft carries out are shown. Each pane is one function below, so a new setting
//! goes into its pane's function. Settings agents can change too go through `file.options`; the
//! rest live in [`crate::UiState`] (saved in `ui.json`).

use std::cell::Cell;

use egui::{RichText, Ui, vec2};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use wordcraft_engine::cmd::options::{MAX_WORD_CHARS, PANES};
use wordcraft_geom::Unit;

use crate::WordApp;
use crate::theme::{Tokens, semibold};

thread_local! {
    // The unit dialogs and the ribbon show lengths in: File › Options › Advanced, copied from the
    // session each frame so dialog code without the app can read it.
    static UNIT: Cell<Unit> = const { Cell::new(Unit::Inches) };
    // The region's decimal separator (#838).
    static DECIMAL: Cell<char> = const { Cell::new('.') };
}

/// The unit lengths are shown in (File › Options › Advanced › Measurement units, else the
/// region's).
pub fn unit() -> Unit {
    UNIT.get()
}

/// The decimal separator lengths are written with (the region's: `,` in Poland).
pub fn decimal() -> char {
    DECIMAL.get()
}

/// `pt` in [`unit`], written the region's way (`2,54 cm`), with the unit in the interface
/// language (`2,54 см` in Russian).
pub fn length(pt: f32) -> String {
    length_in(unit(), pt)
}

/// [`length`] in a given unit.
pub fn length_in(unit: Unit, pt: f32) -> String {
    format!("{}{}", unit.number(pt, decimal()), unit_suffix(unit))
}

/// The unit's suffix as the interface shows it: ` cm`, ` mm`, ` pt` and ` pi` go through the
/// catalog (` см` in Russian, #1734); inches keep `"`.
pub fn unit_suffix(unit: Unit) -> String {
    match unit {
        Unit::Inches => unit.suffix().to_string(),
        Unit::Centimeters => format!(" {}", tl!("cm")),
        Unit::Millimeters => format!(" {}", tl!("mm")),
        Unit::Points => format!(" {}", tl!("pt")),
        Unit::Picas => format!(" {}", tl!("pi")),
    }
}

/// A length typed into a field in `unit`, as a number of that unit: either decimal separator,
/// another unit (`1 in` in a centimetre field), and the units' names in the interface language
/// as well as in English (`3 см`).
pub fn parse_length(text: &str, unit: Unit) -> Option<f64> {
    let mut text: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    for (english, translated) in [("cm", tl!("cm")), ("mm", tl!("mm")), ("pt", tl!("pt")), ("pi", tl!("pi"))] {
        let translated = translated.to_lowercase();
        if translated != english
            && let Some(number) = text.strip_suffix(translated.as_str())
        {
            text = format!("{number}{english}");
            break;
        }
    }
    wordcraft_geom::parse_measure(&text, unit).map(|pt| f64::from(pt / unit.pt_per_unit()))
}

/// Copy the session's settings the UI reads without the app.
pub(crate) fn sync(app: &WordApp) {
    UNIT.set(app.session.units());
    DECIMAL.set(app.session.region().decimal());
}

/// A length field: the unit's suffix, and numbers written and read with the region's decimal
/// separator (`2,54 cm`). Typing either separator works, and so does another unit (`1 in` in a
/// centimetre field).
pub trait MeasureExt {
    fn measure(self, unit: Unit) -> Self;
    /// The same field showing nothing, not even the unit, until a length is typed in: a value
    /// the selected paragraphs differ in.
    fn measure_blank(self, unit: Unit) -> Self;
}

impl MeasureExt for egui::DragValue<'_> {
    fn measure(self, unit: Unit) -> Self {
        let sep = decimal();
        self.suffix(unit_suffix(unit))
            .custom_formatter(move |n, decimals| {
                let s = egui::emath::format_with_decimals_in_range(n, decimals);
                if sep == '.' { s } else { s.replace('.', &sep.to_string()) }
            })
            .custom_parser(move |text| parse_length(text, unit))
    }
    fn measure_blank(self, unit: Unit) -> Self {
        self.custom_formatter(|_, _| String::new()).custom_parser(move |text| parse_length(text, unit))
    }
}

/// Regions offered in File › Options › Advanced › Region (any locale tag works through
/// `file.options`): Letter and A4 countries, decimal point and decimal comma.
const REGIONS: [&str; 16] =
    ["en-US", "en-CA", "en-GB", "en-AU", "en-IN", "es-MX", "es-ES", "pt-BR", "fr-FR", "de-DE", "it-IT", "pl-PL", "ru-RU", "ja-JP", "zh-CN", "ko-KR"];

/// The units offered, with their names.
pub const UNITS: [(Unit, &str); 5] = [
    (Unit::Inches, "Inches"),
    (Unit::Centimeters, "Centimeters"),
    (Unit::Millimeters, "Millimeters"),
    (Unit::Points, "Points"),
    (Unit::Picas, "Picas"),
];

/// The pane names, in [`PANES`] order.
fn pane_label(id: &str) -> &'static str {
    match id {
        "display" => "Display",
        "proofing" => "Proofing",
        "save" => "Save",
        "language" => "Language",
        "accessibility" => "Accessibility",
        "advanced" => "Advanced",
        "agents" => "Agents",
        _ => "General",
    }
}

/// The Options page.
pub fn page(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Options")).font(semibold(26.0)));
    ui.add_space(16.0);
    let current = if PANES.contains(&app.ui.options_pane.as_str()) { app.ui.options_pane.clone() } else { "general".to_string() };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(170.0);
            for id in PANES {
                let label = tl!(pane_label(id));
                let active = current == id;
                let (r, resp) = ui.allocate_exact_size(vec2(170.0, 30.0), egui::Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label));
                if active {
                    ui.painter().rect_filled(r, 4.0, t.checked);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, t.hover);
                }
                let font = if active { semibold(14.0) } else { crate::theme::regular(14.0) };
                // Long translations stay inside the row; WidgetInfo above keeps the full name.
                ui.painter().with_clip_rect(r).text(egui::pos2(r.min.x + 12.0, r.center().y), egui::Align2::LEFT_CENTER, label, font, t.text);
                if resp.clicked() {
                    app.ui.options_pane = id.to_string();
                }
            }
        });
        ui.add_space(12.0);
        let (line, _) = ui.allocate_exact_size(vec2(1.0, 420.0), egui::Sense::hover());
        ui.painter().rect_filled(line, 0.0, t.border);
        ui.add_space(20.0);
        ui.vertical(|ui| {
            ui.set_max_width(640.0);
            match current.as_str() {
                "display" => display(app, ui),
                "proofing" => proofing(app, ui),
                "save" => save(app, ui),
                "language" => language(app, ui),
                "accessibility" => accessibility(app, ui),
                "advanced" => advanced(app, ui),
                "agents" => agents(ui),
                _ => general(app, ui),
            }
        });
    });
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(tl!(title)).font(semibold(15.0)));
    ui.add_space(2.0);
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(tl!(text)).small().weak());
}

/// Change settings through `file.options` (the command agents use).
fn set(app: &mut WordApp, params: Value) {
    if let Err(e) = app.run("file.options", params) {
        app.status(e);
    }
}

/// A checkbox for a `file.options` on/off setting.
fn flag(app: &mut WordApp, ui: &mut Ui, label: &str, key: &str, value: bool) {
    let mut v = value;
    if ui.checkbox(&mut v, tl!(label)).changed() {
        set(app, json!({ key: v }));
    }
}

fn general(app: &mut WordApp, ui: &mut Ui) {
    section(ui, "Interface");
    theme_picker(app, ui);
    interface_size_picker(app, ui);
    ui.checkbox(&mut app.ui.mini_toolbar, tl!("Show the mini toolbar when text is selected"));
    let mut never = app.ui.never_change_page.unwrap_or(false);
    if ui
        .checkbox(&mut never, tl!("Never change the document page color"))
        .on_hover_text(tl!("Off, pages follow the theme: light in the light theme, dark in the dark theme. On, pages keep the document's own colour in every theme; only View › Switch Modes darkens them. Saving, printing and PDFs are unchanged."))
        .changed()
    {
        let _ = app.run("ui.neverChangePage", json!({"value": never}));
    }
    ui.checkbox(&mut app.ui.show_discord, tl!("Show the community button in the title bar"));
    default_app_button(app, ui);
    ui.add_space(8.0);
    section(ui, "Start-up");
    let mut start = app.ui.start_screen;
    if ui
        .checkbox(&mut start, tl!("Show the Start screen when WordCraft starts"))
        .on_hover_text(tl!(
            "On: WordCraft opens on File › Home, with a blank document, the templates and your recent documents. Off: it opens a blank document."
        ))
        .changed()
    {
        let _ = app.run("ui.startScreen", json!({"value": start}));
    }
    ui.add_space(8.0);
    section(ui, "Personalize");
    egui::Grid::new("options_user").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("User name:"));
        let mut n = app.session.author.clone();
        let protected = wordcraft_engine::cmd::protect::enforced(&app.session.doc);
        if ui.add_enabled(!protected, egui::TextEdit::singleline(&mut n).desired_width(260.0)).changed() && !protected {
            app.session.author = n;
        }
        ui.end_row();
        ui.label(tl!("Initials:"));
        let mut i = app.session.initials.clone();
        let hint = app.session.user_initials();
        if ui
            .add(
                egui::TextEdit::singleline(&mut i).desired_width(80.0).char_limit(wordcraft_engine::cmd::options::MAX_INITIALS_CHARS).hint_text(hint),
            )
            .changed()
        {
            set(app, json!({"initials": i}));
        }
        ui.end_row();
    });
    note(ui, "New comments and tracked changes are recorded under this name and these initials.");
}

fn display(app: &mut WordApp, ui: &mut Ui) {
    section(ui, "Formatting marks");
    note(ui, "These marks show even when Show/Hide ¶ is off.");
    let m = app.session.prefs.marks;
    for (label, key, on) in [
        ("Tab characters", "tabs", m.tabs),
        ("Spaces", "spaces", m.spaces),
        ("Paragraph marks", "paragraphs", m.paragraphs),
        ("Hidden text", "hidden", m.hidden),
    ] {
        let mut v = on;
        if ui.checkbox(&mut v, tl!(label)).changed() {
            set(app, json!({"marks": { key: v }}));
        }
    }
    let mut marks = app.session.view.marks;
    if ui.checkbox(&mut marks, tl!("Show all formatting marks")).changed() {
        let _ = app.run("view.marks", json!({"value": marks}));
    }
    ui.add_space(8.0);
    section(ui, "Page display");
    let mut ruler = app.session.view.ruler;
    if ui.checkbox(&mut ruler, tl!("Show rulers")).changed() {
        let _ = app.run("view.ruler", json!({"value": ruler}));
    }
    numeral_picker(app, ui);
    ui.add_space(8.0);
    section(ui, "Printing");
    ui.checkbox(&mut app.ui.print_backgrounds, tl!("Print the page color"));
    ui.checkbox(&mut app.ui.update_fields_before_print, tl!("Update fields before printing"));
}

fn proofing(app: &mut WordApp, ui: &mut Ui) {
    // #476 (AutoCorrect Options…) goes here, at the top.
    section(ui, "Spelling");
    let pr = app.session.prefs.clone();
    flag(app, ui, "Ignore words in capitals", "ignoreUppercase", pr.ignore_uppercase);
    flag(app, ui, "Ignore words with numbers", "ignoreNumbers", pr.ignore_numbers);
    flag(app, ui, "Ignore web and file addresses", "ignoreInternet", pr.ignore_internet);
    ui.add_space(8.0);
    section(ui, "As you type");
    let spelling = app.session.view.proofing;
    flag(app, ui, "Check spelling as you type", "checkSpelling", spelling);
    ui.add_enabled_ui(app.session.view.proofing, |ui| flag(app, ui, "Mark grammar errors as you type", "markGrammar", pr.mark_grammar));
    ui.add_space(8.0);
    section(ui, "Custom dictionary");
    custom_dictionary(app, ui);
    ui.add_space(8.0);
    section(ui, "Dictionaries");
    dictionaries(app, ui);
}

/// Where the spelling dictionaries come from (#1063): a folder searched before the system's
/// usual places, saved with the settings, and the dictionary each language found. An app started
/// from Finder or the Dock doesn't see `WORDCRAFT_DICTIONARIES`, so this is how it finds a
/// dictionary kept in a folder of the user's own.
fn dictionaries(app: &mut WordApp, ui: &mut Ui) {
    let saved = app.session.prefs.dictionaries_folder.clone().unwrap_or_default();
    let id = egui::Id::new("options_dictionaries_folder");
    let mut folder = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| saved.clone());
    ui.label(tl!("Dictionaries folder:"));
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut folder).desired_width(320.0).hint_text(tl!("Not set: the system's usual places")));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let changed = folder.trim() != saved;
        if ui.add_enabled(changed, egui::Button::new(tl!("Apply"))).clicked() || (enter && changed) {
            let value = if folder.trim().is_empty() { "auto" } else { folder.trim() };
            match app.run("file.options", json!({"dictionariesFolder": value})) {
                // Shown as saved (`~/` expanded).
                Ok(_) => folder = app.session.prefs.dictionaries_folder.clone().unwrap_or_default(),
                Err(e) => app.status(e),
            }
        }
        if ui.add_enabled(!saved.is_empty(), egui::Button::new(tl!("Clear"))).clicked()
            && app.run("file.options", json!({"dictionariesFolder": "auto"})).is_ok()
        {
            folder.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, folder));
    note(ui, "Hunspell .aff and .dic files here (pl_PL, pt_BR, it_IT, hr_HR, tr_TR) are used before the system's.");
    // The dictionary each language found, looked up again after the folder changes.
    let found_id = egui::Id::new("options_dictionaries_found");
    let generation = wordcraft_engine::proof::dictionary_generation();
    let cached = ui.data(|d| d.get_temp::<(u64, Vec<(String, Option<String>, Option<String>)>)>(found_id)).filter(|(g, _)| *g == generation);
    let found = match cached {
        Some((_, f)) => f,
        None => {
            let f: Vec<(String, Option<String>, Option<String>)> = wordcraft_engine::proof::installed_dictionaries()
                .into_iter()
                .map(|(lang, dic)| {
                    let error = wordcraft_engine::proof::dictionary_load_error(lang);
                    (language_name(lang).to_string(), dic.map(|d| d.to_string_lossy().into_owned()), error)
                })
                .collect();
            ui.data_mut(|d| d.insert_temp(found_id, (generation, f.clone())));
            f
        }
    };
    for (language, file, error) in found {
        let line = match (file, error) {
            (Some(f), Some(e)) => {
                crate::i18n::fmt(tl!("{language}: {file} (unreadable: {reason})"), &[("language", &language), ("file", &f), ("reason", &e)])
            }
            (Some(f), None) => crate::i18n::fmt(tl!("{language}: {file}"), &[("language", &language), ("file", &f)]),
            (None, _) => crate::i18n::fmt(tl!("{language}: no dictionary found"), &[("language", &language)]),
        };
        ui.add(egui::Label::new(RichText::new(line).small()).truncate());
    }
}

/// The interface name of a language with a Hunspell dictionary.
fn language_name(lang: wordcraft_engine::proof::ProofLang) -> &'static str {
    use wordcraft_engine::proof::ProofLang;
    match lang {
        ProofLang::Pl => tl!("Polish"),
        ProofLang::Pt => tl!("Portuguese (Brazil)"),
        ProofLang::It => tl!("Italian"),
        ProofLang::Hr => tl!("Croatian"),
        ProofLang::Tr => tl!("Turkish"),
        ProofLang::Ru => tl!("Russian"),
        ProofLang::Cs => tl!("Czech"),
        ProofLang::Id => tl!("Indonesian"),
        // English uses its bundled word list; Bulgarian and German have no dictionary yet.
        ProofLang::En | ProofLang::EnGb | ProofLang::EnCa | ProofLang::Bg | ProofLang::De | ProofLang::Other => "",
    }
}

/// The custom dictionary: the words Add to Dictionary kept, each with a Remove button, and a
/// field to add one.
fn custom_dictionary(app: &mut WordApp, ui: &mut Ui) {
    let words = wordcraft_engine::proof::user_dictionary();
    if words.is_empty() {
        note(ui, "No words yet. Add to Dictionary on a misspelled word puts it here.");
    }
    egui::ScrollArea::vertical().id_salt("options_dictionary").max_height(180.0).show(ui, |ui| {
        for w in &words {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(vec2(220.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(220.0);
                    ui.add(egui::Label::new(w.as_str()).truncate());
                });
                if ui.small_button(tl!("Remove")).clicked() {
                    set(app, json!({"removeWords": [w]}));
                }
            });
        }
    });
    let id = egui::Id::new("options_dictionary_new");
    let mut word = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut word).desired_width(220.0).char_limit(MAX_WORD_CHARS).hint_text(tl!("New word")));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let w = word.trim().to_string();
        let ok = !w.is_empty() && !w.chars().any(char::is_whitespace);
        if (ui.add_enabled(ok, egui::Button::new(tl!("Add"))).clicked() || (enter && ok)) && app.run("file.options", json!({"addWords": [w]})).is_ok()
        {
            word.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, word));
}

fn save(app: &mut WordApp, ui: &mut Ui) {
    section(ui, "Save documents");
    // The browser can't write to the user's files, so AutoSave can't be turned on there (#176).
    let browser = app.autosave_block() == Some(crate::AutoSaveBlock::Browser);
    let mut autosave = app.autosave && !browser;
    let r = ui.add_enabled(!browser, egui::Checkbox::new(&mut autosave, tl!("AutoSave documents you have saved in WordCraft")));
    if browser {
        r.on_disabled_hover_text(crate::AutoSaveBlock::Browser.reason());
    } else if r.changed() {
        app.autosave = autosave;
        app.session.autosave = autosave;
    }
    note(ui, "AutoSave writes your changes to the file a moment after you stop typing.");
    ui.add_space(8.0);
    // The open document's `file.privacyOptions` (#1062): saved with it, honoured when it is saved.
    section(ui, "Privacy for this document");
    let settings = &app.session.doc.settings;
    let rows = [
        ("Remove my name from comments, changes and file properties when saving", "removePersonalInfo", settings.remove_personal_info),
        ("Leave out the date and time of comments and tracked changes when saving", "removeDateTime", settings.remove_date_time),
    ];
    for (label, key, value) in rows {
        let mut v = value;
        if ui.checkbox(&mut v, tl!(label)).changed()
            && let Err(e) = app.run("file.privacyOptions", json!({ key: v }))
        {
            app.status(e);
        }
    }
    note(ui, "The document you are editing keeps the names and dates; only the saved file leaves them out.");
}

fn language(app: &mut WordApp, ui: &mut Ui) {
    section(ui, "Interface language");
    language_picker(app, ui);
    // #414 (detect the document's language) goes here.
    ui.add_space(8.0);
    section(ui, "Proofing language");
    let lang = crate::ribbon::caret_language(app);
    let text = if crate::ribbon::caret_no_proof(app) {
        tl!("The text at the insertion point is marked Do not check spelling or grammar.").to_string()
    } else if !crate::ribbon::caret_language_proofed(app) {
        crate::i18n::fmt(tl!("{language}: WordCraft doesn't check spelling or grammar in this language yet."), &[("language", &lang)])
    } else {
        let unreadable = crate::ribbon::caret_unreadable_dictionary(app);
        crate::chrome::dictionary_problem(&lang, &unreadable, crate::ribbon::caret_missing_dictionary(app))
            .unwrap_or_else(|| crate::i18n::fmt(tl!("Spelling and grammar are checked in {language}."), &[("language", &lang)]))
    };
    ui.label(RichText::new(text).small().weak());
}

fn accessibility(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_engine::speech::{MAX_RATE, MIN_RATE};
    section(ui, "Visual effects");
    animations_picker(app, ui);
    ui.add_space(8.0);
    section(ui, "Read Aloud");
    let mut rate = app.session.read_aloud.rate();
    ui.horizontal(|ui| {
        ui.label(tl!("Speed"));
        let r = ui.add(egui::Slider::new(&mut rate, MIN_RATE..=MAX_RATE).step_by(0.05).custom_formatter(|v, _| format!("{v:.2}×")));
        if r.changed() {
            let _ = app.run("readAloud.speed", json!({"value": rate}));
        }
    });
    let mut skip = app.session.read_aloud.skip_citations;
    if ui.checkbox(&mut skip, tl!("Skip citations & bibliography")).changed() {
        let _ = app.run("readAloud.skipCitations", json!({"value": skip}));
    }
}

fn advanced(app: &mut WordApp, ui: &mut Ui) {
    section(ui, "Editing");
    let pr = app.session.prefs.clone();
    flag(app, ui, "Typing replaces selected text", "typingReplacesSelection", pr.typing_replaces_selection);
    flag(app, ui, "Use the Insert key to switch overtype on and off", "insertKeyOvertype", pr.insert_key_overtype);
    flag(app, ui, "Overtype (typing replaces the characters after the caret)", "overtype", pr.overtype);
    let mut guides = app.ui.alignment_guides;
    if ui
        .checkbox(&mut guides, tl!("Show alignment guides when moving pictures and shapes"))
        .on_hover_text(tl!("Moved objects snap to the page's edges and centre, the margins and other objects. Hold Alt to move freely."))
        .changed()
    {
        let _ = app.run("ui.alignmentGuides", json!({"value": guides}));
    }
    ui.add_space(8.0);
    section(ui, "Display");
    egui::Grid::new("options_display").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Recent documents to show:"));
        let mut n = app.ui.recent_shown();
        if ui.add(egui::DragValue::new(&mut n).range(0..=crate::MAX_RECENT).speed(0.2)).changed() {
            app.ui.recent_count = n;
        }
        ui.end_row();
        ui.label(tl!("Measurement units:"));
        // The user's choice, else the region's (#838): Automatic follows the region.
        let units = app.session.units();
        let auto = app.session.prefs.units.is_none();
        let name = UNITS.iter().find(|(u, _)| *u == units).map_or("Inches", |(_, n)| n);
        let shown = if auto { format!("{} ({})", tl!("Automatic"), tl!(name)) } else { tl!(name).to_string() };
        egui::ComboBox::from_id_salt("options_units").selected_text(shown).width(200.0).show_ui(ui, |ui| {
            if ui.selectable_label(auto, tl!("Automatic")).clicked() {
                set(app, json!({"units": "auto"}));
            }
            for (u, label) in UNITS {
                if ui.selectable_label(!auto && units == u, tl!(label)).clicked() {
                    set(app, json!({"units": u}));
                }
            }
        });
        ui.end_row();
        // The region new documents, units and the decimal separator follow; Automatic is the
        // system's (#838).
        ui.label(tl!("Region:"));
        let chosen = app.session.prefs.region.clone();
        let system = &app.session.system_region;
        let system_tag = if system.territory.is_empty() { system.language.clone() } else { format!("{}-{}", system.language, system.territory) };
        let automatic = crate::i18n::fmt(tl!("Automatic ({region})"), &[("region", system_tag.as_str())]);
        let shown = chosen.clone().unwrap_or_else(|| automatic.clone());
        egui::ComboBox::from_id_salt("options_region").selected_text(shown).width(200.0).show_ui(ui, |ui| {
            if ui.selectable_label(chosen.is_none(), automatic.as_str()).clicked() {
                set(app, json!({"region": "auto"}));
            }
            for tag in REGIONS {
                if ui.selectable_label(chosen.as_deref() == Some(tag), tag).clicked() {
                    set(app, json!({"region": tag}));
                }
            }
        });
        ui.end_row();
    });
    let mut keytips = app.ui.keytips_enabled;
    if ui.checkbox(&mut keytips, tl!("Show KeyTips (Alt or F10)")).changed() {
        let _ = app.run("ui.keytips", json!({"value": keytips}));
    }
    ui.add_space(8.0);
    section(ui, "Animation");
    let mut caret = app.ui.caret_animation;
    if ui
        .checkbox(&mut caret, tl!("Animate the caret (it glides and fades)"))
        .on_hover_text(tl!(
            "On: the caret glides to its new place when you type or use the arrow keys, and its blink fades. Off: it jumps and blinks on and off."
        ))
        .changed()
    {
        let _ = app.run("ui.caretAnimation", json!({"value": caret}));
    }
    // The web app has no folders to name.
    if !cfg!(target_arch = "wasm32") {
        ui.add_space(8.0);
        section(ui, "File locations");
        templates_folder(app, ui);
    }
    ui.add_space(8.0);
    section(ui, "Customize");
    ui.horizontal(|ui| {
        ui.label(tl!("Keyboard shortcuts:"));
        if ui.button(tl!("Customize…")).clicked() {
            let _ = app.run("tools.customizeKeyboard", json!({}));
        }
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Ribbon and Quick Access Toolbar:"));
        if ui.button(tl!("Customize…")).clicked() {
            let _ = app.run("tools.customizeRibbon", json!({}));
        }
    });
}

/// The user templates folder (#2852): where File › New finds personal templates and Save as Word
/// Template starts. Empty is the app's own folder, shown as the hint.
fn templates_folder(app: &mut WordApp, ui: &mut Ui) {
    let saved = app.session.prefs.user_templates_folder.clone().unwrap_or_default();
    let id = egui::Id::new("options_user_templates_folder");
    let mut folder = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| saved.clone());
    let default = app.services.templates_dir.as_ref().map(|d| d.to_string_lossy().into_owned());
    ui.label(tl!("User templates:"));
    ui.horizontal(|ui| {
        let hint = default.clone().unwrap_or_else(|| tl!("Not set").to_string());
        let r = ui.add(egui::TextEdit::singleline(&mut folder).desired_width(320.0).hint_text(hint));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let changed = folder.trim() != saved;
        if ui.add_enabled(changed, egui::Button::new(tl!("Apply"))).clicked() || (enter && changed) {
            let value = if folder.trim().is_empty() { "auto" } else { folder.trim() };
            match app.run("file.options", json!({"userTemplatesFolder": value})) {
                Ok(_) => folder = app.session.prefs.user_templates_folder.clone().unwrap_or_default(),
                Err(e) => app.status(e),
            }
        }
        if ui.add_enabled(!saved.is_empty(), egui::Button::new(tl!("Clear"))).clicked()
            && app.run("file.options", json!({"userTemplatesFolder": "auto"})).is_ok()
        {
            folder.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, folder));
    note(ui, "File › New lists the .dotx and .dotm templates here under Personal; Save as Word Template starts here.");
}

fn agents(ui: &mut Ui) {
    section(ui, "Agents");
    ui.label(tl!("Every command is available to scripts and AI agents: run `wordcraft-cli mcp` for an MCP server, or start the app with `--control <port>` for the JSON control channel."));
}

/// Interface theme: Light, Dark, or follow the system's appearance (#115).
fn theme_picker(app: &mut WordApp, ui: &mut Ui) {
    use crate::theme::Appearance;
    ui.horizontal(|ui| {
        ui.label(tl!("Interface theme:"));
        egui::ComboBox::from_id_salt("interface_theme").selected_text(tl!(app.ui.theme.label())).width(220.0).show_ui(ui, |ui| {
            for a in Appearance::ALL {
                if ui.selectable_label(app.ui.theme == a, tl!(a.label())).clicked() {
                    let _ = app.run("ui.theme", json!({"value": a.code()}));
                }
            }
        });
    });
}

/// Interface size (#475, #822): Auto, which keeps the system's display scale and shows the size it
/// draws at, or an absolute size, so "100%" means one screen pixel per point.
fn interface_size_picker(app: &mut WordApp, ui: &mut Ui) {
    use crate::theme::InterfaceSize;
    let size = app.ui.interface_size;
    let native = ui.ctx().native_pixels_per_point().unwrap_or(1.0);
    let auto = crate::i18n::fmt(tl!("Auto ({percent}%)"), &[("percent", &InterfaceSize::AUTO.drawn_percent(native).to_string())]);
    let selected = size.percent().map_or_else(|| auto.clone(), |pct| format!("{pct}%"));
    ui.horizontal(|ui| {
        ui.label(tl!("Interface size:"));
        egui::ComboBox::from_id_salt("interface_size").selected_text(selected).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(size.is_auto(), auto).clicked() {
                let _ = app.run("ui.interfaceSize", json!({"value": "auto"}));
            }
            for pct in InterfaceSize::PRESETS {
                if ui.selectable_label(size.percent() == Some(pct), format!("{pct}%")).clicked() {
                    let _ = app.run("ui.interfaceSize", json!({"percent": pct}));
                }
            }
        });
    });
    ui.label(
        RichText::new(tl!(
            "Makes the ribbon, menus and dialogs larger or smaller. Auto follows your system's display scaling; 100% is one screen pixel per point. Document zoom is separate."
        ))
        .small()
        .weak(),
    );
}

/// Animation effects: on, off, or follow the system's setting (the default). Under "Use system
/// setting" a line says what the system reports, so it is plain what the choice comes to here.
fn animations_picker(app: &mut WordApp, ui: &mut Ui) {
    use crate::theme::Animations;
    ui.horizontal(|ui| {
        ui.label(tl!("Animation effects:"));
        egui::ComboBox::from_id_salt("animation_effects").selected_text(tl!(app.ui.animations.label())).width(220.0).show_ui(ui, |ui| {
            for a in Animations::ALL {
                if ui.selectable_label(app.ui.animations == a, tl!(a.label())).clicked() {
                    let _ = app.run("ui.animations", json!({"value": a.code()}));
                }
            }
        });
    });
    note(ui, "Smooth scrolling and zooming, and menus and buttons that fade in. Off shows every change at once.");
    if app.ui.animations == Animations::System {
        let reported = match app.system_animations() {
            Some(true) => "Your system has animation effects turned on.",
            Some(false) => "Your system has animation effects turned off.",
            None => "Your system doesn't report an animation setting, so animation effects stay on.",
        };
        note(ui, reported);
    }
}

/// Interface language: follow the system (the default) or pick one (#8).
fn language_picker(app: &mut WordApp, ui: &mut Ui) {
    use crate::i18n::{AUTO, Lang};
    // The language names are in their own scripts: fonts for them load from the next frame.
    app.want_system_cjk = true;
    let system = crate::i18n::system_lang();
    let auto_label = crate::i18n::fmt(tl!("Automatic ({language})"), &[("language", system.name())]);
    let current = if app.ui.language == AUTO { auto_label.clone() } else { Lang::from_pref(&app.ui.language).name().to_string() };
    ui.horizontal(|ui| {
        ui.label(tl!("Interface language:"));
        egui::ComboBox::from_id_salt("interface_language").selected_text(current).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(app.ui.language == AUTO, auto_label.as_str()).clicked() {
                let _ = app.run("ui.language", json!({"value": AUTO}));
            }
            for lang in Lang::for_picker() {
                if ui.selectable_label(app.ui.language == lang.code(), lang.name()).clicked() {
                    let _ = app.run("ui.language", json!({"value": lang.code()}));
                }
            }
        });
    });
    ui.label(RichText::new(tl!("Automatic follows your system's language. Menus and commands change; your documents don't.")).small().weak());
}

/// Deserialize `input` over `T`'s defaults one setting at a time: a value of the wrong type is
/// dropped (its default stays) instead of failing the whole read. Objects nest (`editing`).
pub fn lenient<T: Serialize + DeserializeOwned + Default>(input: &Value) -> T {
    let mut base = serde_json::to_value(T::default()).unwrap_or(Value::Null);
    if base.is_object() {
        let mut extra = Vec::new();
        overlay::<T>(&mut base, &mut Vec::new(), input, 0, &mut extra);
        // Keys with no slot in the defaults (write-only legacy settings, or unknown ones) are
        // kept only after the loop, so `base` stays the size of `T` while each key is checked.
        for (path, k, v) in extra {
            if let Some(parent) = object_at(&mut base, &path) {
                parent.insert(k, v);
            }
        }
    }
    serde_json::from_value(base).unwrap_or_default()
}

/// The object at `path` inside `root`.
fn object_at<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Map<String, Value>> {
    let mut v = root;
    for k in path {
        v = v.as_object_mut()?.get_mut(k)?;
    }
    v.as_object_mut()
}

fn overlay<T: DeserializeOwned>(
    root: &mut Value,
    path: &mut Vec<String>,
    input: &Value,
    depth: usize,
    extra: &mut Vec<(Vec<String>, String, Value)>,
) {
    let Some(map) = input.as_object() else { return };
    for (k, v) in map {
        let Some(parent) = object_at(root, path) else { return };
        let old = parent.get(k).cloned();
        if depth < 4 && v.is_object() && old.as_ref().is_some_and(Value::is_object) {
            path.push(k.clone());
            overlay::<T>(root, path, v, depth + 1, extra);
            path.pop();
            continue;
        }
        parent.insert(k.clone(), v.clone());
        if old.is_none() {
            let ok = serde_json::from_value::<T>(root.clone()).is_ok();
            if let Some(parent) = object_at(root, path) {
                parent.remove(k);
            }
            if ok {
                extra.push((path.clone(), k.clone(), v.clone()));
            }
            continue;
        }
        if serde_json::from_value::<T>(root.clone()).is_err()
            && let Some(parent) = object_at(root, path)
        {
            match old {
                Some(o) => parent.insert(k.clone(), o),
                None => parent.remove(k),
            };
        }
    }
}

/// File ▸ Options ▸ General: make WordCraft the default app for Word documents (#295). Only where
/// the host can (`Services::make_default_app`: Windows opens Default apps, Linux and BSD use
/// `xdg-mime`); macOS shows how to do it in Finder, the web shows nothing.
fn default_app_button(app: &mut WordApp, ui: &mut Ui) {
    if app.services.make_default_app.is_some() {
        let hint = if cfg!(target_os = "windows") {
            tl!("Opens Settings › Default apps, where you choose WordCraft for .docx and the other document types.")
        } else {
            tl!("Opens .docx, .doc, .rtf and .odt files and Word templates with WordCraft from now on.")
        };
        if ui.button(tl!("Make WordCraft the default for Word documents…")).on_hover_text(hint).clicked() {
            // Straight to the hook, not through `run`: only a click may change system defaults.
            let _ = app.make_default_app();
        }
    } else if cfg!(target_os = "macos") {
        ui.label(
            egui::RichText::new(tl!("To open Word documents with WordCraft by default: select one in Finder, choose File › Get Info, pick WordCraft under Open with, then click Change All."))
                .small()
                .weak(),
        );
    }
}

/// File ▸ Options ▸ Numerals: how digits look on screen (display only, like Word's
/// File › Options › Advanced › Numeral).
fn numeral_picker(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_layout::NumeralMode;
    ui.horizontal(|ui| {
        ui.label(tl!("Numerals:"));
        let mut sel = app.session.view.numeral;
        egui::ComboBox::from_id_salt("numerals").selected_text(tl!(sel.label())).width(220.0).show_ui(ui, |ui| {
            for m in [NumeralMode::Arabic, NumeralMode::Hindi, NumeralMode::Context, NumeralMode::System] {
                ui.selectable_value(&mut sel, m, tl!(m.label()));
            }
        });
        if sel != app.session.view.numeral {
            let _ = app.run("view.numeral", json!({ "value": sel }));
        }
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{NodeT, Queryable};
    use serde_json::json;
    use wordcraft_geom::Unit;

    use super::{decimal, length, parse_length, unit};

    use crate::theme::Appearance;
    use crate::{Services, UiState, WordApp};

    #[test]
    fn unknown_settings_keys_load_in_linear_time() {
        let mut map = serde_json::Map::new();
        for i in 0..20_000 {
            map.insert(format!("unknownKey{i}"), json!(i));
        }
        map.insert("miniToolbar".into(), json!(false));
        let bytes = serde_json::to_vec(&serde_json::Value::Object(map)).unwrap();
        let state = UiState::from_json(&bytes).unwrap();
        assert!(!state.mini_toolbar);
    }

    fn app() -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    fn nav_harness(language: &str, percent: u16) -> egui_kittest::Harness<'static, WordApp> {
        let mut a = app();
        a.run("ui.language", json!({"value": language})).unwrap();
        a.run("ui.interfaceSize", json!({"percent": percent})).unwrap();
        a.run("file.options", json!({"pane": "general"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h
    }

    /// File › Options must not offer a way around the protected document's author check.
    #[test]
    fn user_name_field_disables_when_protection_starts() {
        let mut h = nav_harness("en", 100);
        let user_name = || egui_kittest::kittest::By::new().role(egui::accesskit::Role::TextInput).value("WordCraft User");
        assert!(!h.get(user_name()).accesskit_node().is_disabled());
        h.state_mut().run("review.restrict", json!({"mode": "readOnly"})).unwrap();
        h.run_steps(4);
        assert!(h.get(user_name()).accesskit_node().is_disabled());
    }

    /// #1266: check the actual painted text and renderer clip, including the bold selected
    /// label. AccessKit must still expose the full name and the row must keep its fixed size.
    #[test]
    fn options_nav_labels_stay_inside_their_rows() {
        if !crate::i18n::catalogs_embedded() {
            return;
        }
        for language in ["en", "ru"] {
            for percent in [100, 125, 150] {
                let mut h = nav_harness(language, percent);
                // An interface size is absolute (#822): `percent` points per 100 screen pixels,
                // whatever the display's own scale (2.0 here).
                assert_eq!(h.ctx.pixels_per_point(), f32::from(percent) / 100.0);
                for selected in ["general", "accessibility"] {
                    h.state_mut().run("file.options", json!({"pane": selected})).unwrap();
                    h.run_steps(4);
                    for id in wordcraft_engine::cmd::options::PANES {
                        let label = crate::i18n::tr(crate::i18n::Lang::from_pref(language), super::pane_label(id));
                        let node =
                            h.query_all_by_label(label).find(|n| n.accesskit_node().toggled().is_some()).expect("pane has its full accessible name");
                        assert_eq!(
                            node.accesskit_node().toggled(),
                            Some(if id == selected { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False })
                        );
                        let row = node.rect();
                        assert_eq!(row.size(), egui::vec2(170.0, 30.0));
                        let (clip, text) = h
                            .output()
                            .shapes
                            .iter()
                            .find_map(|s| match &s.shape {
                                egui::Shape::Text(text) if text.galley.text() == label && row.contains(text.pos) => Some((s.clip_rect, text)),
                                _ => None,
                            })
                            .expect("navigation label is painted");
                        let bounds = text.visual_bounding_rect();
                        assert!(row.contains_rect(bounds.intersect(clip)), "{language} {percent}% {selected}: {label} paints outside {row:?}");
                        if language == "en" && ["general", "display", "save"].contains(&id) {
                            assert!(clip.contains_rect(bounds), "short labels must remain fully visible");
                        }
                    }
                }
            }
        }
    }

    /// Clipping only the painter must leave the entire row clickable, its hover/selection
    /// backgrounds intact, and the selected pane's settings visible at every interface size.
    #[test]
    fn options_nav_rows_keep_hover_and_selection() {
        if !crate::i18n::catalogs_embedded() {
            return;
        }
        for language in ["en", "ru"] {
            for percent in [100, 125, 150] {
                let mut h = nav_harness(language, percent);
                for (id, setting) in
                    [("display", "Spaces"), ("save", "Save documents"), ("accessibility", "Visual effects"), ("general", "Interface")]
                {
                    let lang = crate::i18n::Lang::from_pref(language);
                    let row = h
                        .query_all_by_label(crate::i18n::tr(lang, super::pane_label(id)))
                        .find(|n| n.accesskit_node().toggled().is_some())
                        .expect("pane has its full accessible name")
                        .rect();
                    // Both ends of the row, including its empty left inset, stay clickable.
                    for x in [row.left() + 2.0, row.right() - 2.0] {
                        h.state_mut().run("file.options", json!({"pane": "agents"})).unwrap();
                        h.hover_at(egui::pos2(x, row.center().y));
                        h.run_steps(4);
                        let t = crate::theme::Tokens::get(&h.ctx);
                        let background = |h: &egui_kittest::Harness<'_, WordApp>, color| {
                            h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == row && r.fill == color))
                        };
                        assert!(background(&h, t.hover), "{language} {percent}%: hover covers the row");
                        for pressed in [true, false] {
                            h.input_mut().events.push(egui::Event::PointerButton {
                                pos: egui::pos2(x, row.center().y),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            });
                            h.run_steps(1);
                        }
                        h.run_steps(4);
                        assert_eq!(h.state().ui.options_pane, id);
                        assert!(background(&h, t.checked), "{language} {percent}%: selection covers the row");
                        assert!(h.query_by_label(crate::i18n::tr(lang, setting)).is_some(), "{id} settings are shown");
                    }
                }
            }
        }
    }

    /// File › Options settings come back after a restart, and a `ui.json` with a wrong value in
    /// one setting keeps every other setting (#485).
    #[test]
    fn options_survive_a_restart_and_junk_in_ui_json() {
        let mut a = app();
        a.run(
            "file.options",
            json!({"initials": "VK", "units": "centimeters", "typingReplacesSelection": false, "marks": {"tabs": true}, "ignoreNumbers": false, "addWords": ["zqxoptionsword"]}),
        )
        .unwrap();
        a.ui.recent_count = 30;
        a.ui.print_backgrounds = true;
        let saved = serde_json::to_vec(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(UiState::from_json(&saved).unwrap());
        assert_eq!(b.session.prefs, a.session.prefs);
        assert_eq!(b.session.prefs.units, Some(Unit::Centimeters));
        assert_eq!(b.session.initials, "VK");
        assert_eq!((b.ui.recent_count, b.ui.print_backgrounds), (30, true));
        assert!(wordcraft_engine::proof::user_dictionary().contains(&"zqxoptionsword".to_string()));

        let junk = br#"{"theme": "dark", "recentCount": "lots", "printBackgrounds": 7, "optionsPane": ["x"], "dark": true,
            "editing": {"measurementUnits": "furlongs", "typingReplacesSelection": false, "marks": {"tabs": "yes", "spaces": true}}}"#;
        let ui = UiState::from_json(junk).unwrap();
        assert_eq!(ui.theme, Appearance::Dark);
        assert_eq!((ui.recent_count, ui.print_backgrounds, ui.options_pane.as_str()), (12, false, "general"));
        assert_eq!(ui.legacy_dark, Some(true));
        assert_eq!(ui.editing.units, None);
        assert!(!ui.editing.typing_replaces_selection);
        assert!(ui.editing.marks.spaces && !ui.editing.marks.tabs);
        assert!(UiState::from_json(b"[1, 2]").is_none() && UiState::from_json(b"not json").is_none());
    }

    /// The dictionaries folder (#1063) is saved with the settings and searched again after a
    /// restart, before any document is checked.
    #[test]
    fn the_dictionaries_folder_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("wc-ui-dictionaries-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = app();
        a.run("file.options", json!({"dictionariesFolder": dir.to_string_lossy()})).unwrap();
        let saved = serde_json::to_vec(&a.prefs()).unwrap();
        wordcraft_engine::proof::hunspell::set_dictionary_dir(None);
        let mut b = app();
        b.apply_prefs(UiState::from_json(&saved).unwrap());
        assert_eq!(b.session.prefs.dictionaries_folder.as_deref(), Some(dir.to_string_lossy().as_ref()));
        assert_eq!(wordcraft_engine::proof::hunspell::search_dirs(None).first(), Some(&dir), "searched first");
    }

    /// #1792: Check spelling as you type is saved with the other proofing choices, so after a
    /// restart the box comes back the way the user left it.
    #[test]
    fn check_spelling_survives_a_restart() {
        let mut a = app();
        a.run("file.options", json!({"checkSpelling": false})).unwrap();
        assert!(!a.session.view.proofing, "off right away");
        let saved = serde_json::to_value(a.prefs()).unwrap();
        assert_eq!(saved["editing"]["checkSpelling"], false, "saved in ui.json");
        let mut b = app();
        b.apply_prefs(UiState::from_json(&serde_json::to_vec(&saved).unwrap()).unwrap());
        assert!(!b.session.view.proofing, "spelling stays off after a restart");
    }

    /// Settings saved before #838 kept the unit under `editing.units` (default inches): a unit
    /// the user chose carries over to `measurementUnits`; inches, junk or a newer value don't.
    #[test]
    fn a_unit_chosen_before_regions_survives_the_rename() {
        let editing = |json: &str| UiState::from_json(format!(r#"{{"editing": {json}}}"#).as_bytes()).unwrap().editing.units;
        assert_eq!(editing(r#"{"units": "centimeters"}"#), Some(Unit::Centimeters));
        assert_eq!(editing(r#"{"units": "picas", "overtype": true}"#), Some(Unit::Picas));
        assert_eq!(editing(r#"{"units": "inches"}"#), None);
        assert_eq!(editing(r#"{"units": "furlongs"}"#), None);
        assert_eq!(editing(r#"{"units": 7}"#), None);
        assert_eq!(editing(r#"{"units": "millimeters", "measurementUnits": "points"}"#), Some(Unit::Points));
        assert_eq!(editing(r#"{"units": "millimeters", "measurementUnits": null}"#), None);
        assert_eq!(editing("{}"), None);
        let mut a = app();
        a.apply_prefs(UiState::from_json(br#"{"editing": {"units": "millimeters"}}"#).unwrap());
        assert_eq!(a.session.units(), Unit::Millimeters);
        // Saved again, it's under the new key.
        let saved = serde_json::to_value(a.prefs()).unwrap();
        assert_eq!(saved["editing"]["measurementUnits"], "millimeters");
    }

    /// `file.options` opens the page at a pane; the pane list switches panes, and their
    /// checkboxes change the settings (#485).
    #[test]
    fn options_panes_open_and_change_settings() {
        let mut a = app();
        a.run("file.options", json!({"pane": "display"})).unwrap();
        assert!(a.ui.backstage);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Spaces").click();
        h.run_steps(2);
        assert!(h.state().session.prefs.marks.spaces, "Display › Spaces");
        h.get_by_label("Proofing").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.options_pane, "proofing");
        h.get_by_label("Ignore words in capitals").click();
        h.run_steps(2);
        assert!(!h.state().session.prefs.ignore_uppercase);
        // #1063: the dictionaries folder and the dictionary each language found.
        assert!(h.query_by_label("Dictionaries folder:").is_some());
        assert!(h.query_by_label_contains("Polish: ").is_some(), "the Polish dictionary's line");
        h.get_by_label("Advanced").click();
        h.run_steps(4);
        h.get_by_label("Typing replaces selected text").click();
        h.run_steps(2);
        assert!(!h.state().session.prefs.typing_replaces_selection);
        assert!(h.query_by_label("Measurement units:").is_some());
    }

    /// File › Options › Save › Privacy (#1062): the checkboxes are the document's
    /// `file.privacyOptions` settings, and an undoable change like any other.
    #[test]
    fn save_privacy_checkboxes_set_the_privacy_options() {
        let mut a = app();
        a.run("file.options", json!({"pane": "save"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Privacy for this document").is_some(), "the section is there");
        h.get_by_label("Remove my name from comments, changes and file properties when saving").click();
        h.run_steps(4);
        assert!(h.state().session.doc.settings.remove_personal_info);
        assert!(!h.state().session.doc.settings.remove_date_time);
        h.get_by_label("Leave out the date and time of comments and tracked changes when saving").click();
        h.run_steps(4);
        let privacy = h.state_mut().session.run("file.privacy", &json!({})).unwrap();
        assert_eq!(privacy, json!({"removePersonalInfo": true, "removeDateTime": true}));
        h.get_by_label("Remove my name from comments, changes and file properties when saving").click();
        h.run_steps(4);
        assert!(!h.state().session.doc.settings.remove_personal_info, "and off again");
    }

    /// File › Options › Accessibility › Animation effects: Use system setting (the default), On
    /// or Off, with a line saying what the system's own setting is while it is followed.
    #[test]
    fn animation_effects_is_an_accessibility_option() {
        use crate::theme::Animations;
        let services = Services { animations_enabled: Some(Box::new(|| Some(false))), ..Default::default() };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("file.options", json!({"pane": "accessibility"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert_eq!(h.state().ui.animations, Animations::System);
        assert!(h.query_by_label("Animation effects:").is_some());
        assert!(h.query_by_label("Your system has animation effects turned off.").is_some());
        assert!(!h.state().animations_on());
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run_steps(4);
        h.get_by_label("On").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.animations, Animations::On);
        assert!(h.state().animations_on());
        assert!(h.query_by_label("Your system has animation effects turned off.").is_none(), "only said while the system is followed");
    }

    /// File › Options › General › Start-up (#766): the checkbox is the `ui.startScreen` setting.
    #[test]
    fn general_start_up_turns_the_start_screen_off_and_on() {
        let mut a = app();
        a.run("file.options", json!({"pane": "general"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Start-up").is_some(), "the section is there");
        h.get_by_label("Show the Start screen when WordCraft starts").click();
        h.run_steps(4);
        assert!(!h.state().ui.start_screen, "the checkbox turns it off");
        h.get_by_label("Show the Start screen when WordCraft starts").click();
        h.run_steps(4);
        assert!(h.state().ui.start_screen, "and on again");
    }

    /// File › Options › General › Never change the document page color (#1722): the checkbox
    /// is the `ui.neverChangePage` setting and decides whether the page follows the theme.
    #[test]
    fn general_never_change_page_checkbox_pins_the_page() {
        let mut a = app();
        a.run("ui.theme", json!({"value": "dark"})).unwrap();
        a.run("file.options", json!({"pane": "general"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let label = "Never change the document page color";
        assert!(h.state().page_is_dark(), "off: the dark theme gives a dark page");
        h.get_by_label(label).click();
        h.run_steps(4);
        assert_eq!(h.state().ui.never_change_page, Some(true));
        assert!(!h.state().page_is_dark(), "on: the document's own light page");
        h.get_by_label(label).click();
        h.run_steps(4);
        assert_eq!(h.state().ui.never_change_page, Some(false));
        assert!(h.state().page_is_dark(), "off again: the page follows the theme");
    }

    /// File › Options › Advanced › Editing: the alignment guides checkbox is the
    /// `ui.alignmentGuides` setting.
    #[test]
    fn advanced_alignment_guides_checkbox_turns_snapping_off_and_on() {
        let mut a = app();
        a.run("file.options", json!({"pane": "advanced"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let label = "Show alignment guides when moving pictures and shapes";
        h.get_by_label(label).click();
        h.run_steps(4);
        assert!(!h.state().ui.alignment_guides, "the checkbox turns them off");
        h.get_by_label(label).click();
        h.run_steps(4);
        assert!(h.state().ui.alignment_guides, "and on again");
    }

    /// File › Options › Advanced › Animation: the checkbox is the `ui.caretAnimation` setting,
    /// and the section is translated.
    #[test]
    fn advanced_animation_turns_the_caret_animation_off_and_on() {
        let mut a = app();
        a.run("file.options", json!({"pane": "advanced"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Animation").is_some(), "the section is there");
        h.get_by_label("Animate the caret (it glides and fades)").click();
        h.run_steps(4);
        assert!(!h.state().ui.caret_animation, "the checkbox turns it off");
        h.get_by_label("Animate the caret (it glides and fades)").click();
        h.run_steps(4);
        assert!(h.state().ui.caret_animation, "and on again");
        if !crate::i18n::catalogs_embedded() {
            return;
        }
        h.state_mut().run("ui.language", json!({"value": "es"})).unwrap();
        h.run_steps(4);
        assert!(h.query_by_label("Animación").is_some() && h.query_by_label("Animation").is_none());
    }

    /// #838: in a Polish region the Page Setup margins read `2,54 cm` (the region's unit and
    /// decimal comma), a typed `3,5` or `1 in` is understood, and the Options page shows the
    /// region's unit until the user picks another.
    #[test]
    fn length_fields_follow_the_region() {
        let mut a = app();
        a.run("file.options", json!({"region": "pl-PL"})).unwrap();
        a.run("file.new", json!({})).unwrap();
        assert_eq!(wordcraft_geom::paper_name(a.session.doc.last_section.page_w, a.session.doc.last_section.page_h), Some("A4"));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(2);
        // The dialog reads the unit when it opens, after a frame has picked up the region.
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(4);
        assert_eq!((unit(), decimal()), (Unit::Centimeters, ','));
        assert_eq!(length(72.0), "2,54 cm");
        assert_eq!(h.query_all_by_value("2,54 cm").count(), 4, "four margins in centimetres with a decimal comma");

        h.state_mut().run("file.options", json!({"units": "inches"})).unwrap();
        h.run_steps(4);
        assert_eq!(length(108.0), "1,5\"", "the user's unit, still the region's decimal comma");
        h.state_mut().run("file.options", json!({"units": "auto", "region": "en-US"})).unwrap();
        h.run_steps(4);
        assert_eq!(length(72.0), "1\"");
    }

    /// #1734: a Russian interface shows centimetres as `см`, as Word does, and a length field
    /// takes the unit typed either way.
    #[test]
    fn length_units_follow_the_interface_language() {
        if !crate::i18n::catalogs_embedded() {
            return;
        }
        let mut a = app();
        a.run("ui.language", json!({"value": "ru"})).unwrap();
        a.run("file.options", json!({"region": "ru-RU"})).unwrap();
        a.run("file.new", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(2);
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(4);
        assert_eq!((unit(), decimal()), (Unit::Centimeters, ','));
        assert_eq!(length(72.0), "2,54 см");
        assert_eq!(h.query_all_by_value("2,54 см").count(), 4, "four margins in centimetres, named in Russian");
        assert_eq!(h.query_all_by_value("2,54 cm").count(), 0);
        assert_eq!(parse_length("3 см", Unit::Centimeters), Some(3.0));
        assert_eq!(parse_length("3cm", Unit::Centimeters), Some(3.0));
        assert_eq!(parse_length("10 мм", Unit::Centimeters).map(|v| (v * 100.0).round() / 100.0), Some(1.0));
        assert_eq!(parse_length("72 пт", Unit::Centimeters).map(|v| (v * 100.0).round() / 100.0), Some(2.54));

        h.state_mut().run("file.options", json!({"units": "millimeters"})).unwrap();
        h.run_steps(4);
        assert_eq!(length(72.0), "25,4 мм");
        h.state_mut().run("ui.language", json!({"value": "en"})).unwrap();
        h.run_steps(4);
        assert_eq!(length(72.0), "25,4 mm", "English keeps the English names");
    }
}
