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
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
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

/// Draws General options: interface theme and size, mini toolbar, dark page, community button and
/// default apps; the Start screen; and personalization such as user name and initials.
fn general(app: &mut WordApp, ui: &mut Ui) {
    section(ui, "Interface");
    theme_picker(app, ui);
    interface_size_picker(app, ui);
    ui.checkbox(&mut app.ui.mini_toolbar, tl!("Show the mini toolbar when text is selected"));
    let mut dark_page = app.session.view.dark_mode;
    if ui
        .checkbox(&mut dark_page, tl!("Dark page (white text on black)"))
        .on_hover_text(tl!("Show documents with their colours inverted, like View › Switch Modes. Saving, printing and PDFs are unchanged."))
        .changed()
    {
        let _ = app.run("view.darkMode", json!({"value": dark_page}));
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
        let user_label = ui.label(tl!("User name:"));
        let mut n = app.session.author.clone();
        let protected = wordcraft_engine::cmd::protect::enforced(&app.session.doc);
        if ui.add_enabled(!protected, egui::TextEdit::singleline(&mut n).desired_width(260.0)).labelled_by(user_label.id).changed() && !protected {
            app.session.author = n;
        }
        ui.end_row();
        let initials_label = ui.label(tl!("Initials:"));
        let mut i = app.session.initials.clone();
        let hint = app.session.user_initials();
        if ui
            .add(
                egui::TextEdit::singleline(&mut i).desired_width(80.0).char_limit(wordcraft_engine::cmd::options::MAX_INITIALS_CHARS).hint_text(hint),
            )
            .labelled_by(initials_label.id)
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
    section(ui, "AutoCorrect");
    if ui.button(tl!("AutoCorrect Options")).clicked() {
        let _ = app.run("tools.autocorrect", json!({}));
    }
    ui.add_space(8.0);
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
    let id = egui::Id::unique("options_dictionaries_folder");
    let mut folder = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| saved.clone());
    let folder_label = ui.label(tl!("Dictionaries folder:")).id;
    ui.horizontal(|ui| {
        let r = ui
            .add(egui::TextEdit::singleline(&mut folder).desired_width(320.0).hint_text(tl!("Not set: the system's usual places")))
            .labelled_by(folder_label);
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
    let found_id = egui::Id::unique("options_dictionaries_found");
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
        ProofLang::Es => tl!("Spanish (Spain)"),
        ProofLang::EsMx => tl!("Spanish (Mexico)"),
        ProofLang::Es419 => tl!("Spanish (Latin America)"),
        ProofLang::EsUy => tl!("Spanish (Uruguay)"),
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
    let id = egui::Id::unique("options_dictionary_new");
    let mut word = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        let r = ui
            .add(egui::TextEdit::singleline(&mut word).desired_width(220.0).char_limit(MAX_WORD_CHARS).hint_text(tl!("New word")))
            .accessible_name(tl!("New word"));
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

/// Draws Accessibility options: the animation setting and Read Aloud speed and skip-citations.
fn accessibility(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_engine::speech::{MAX_RATE, MIN_RATE};
    section(ui, "Visual effects");
    animations_picker(app, ui);
    ui.add_space(8.0);
    section(ui, "Read Aloud");
    let mut rate = app.session.read_aloud.rate();
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Speed"));
        let r =
            ui.add(egui::Slider::new(&mut rate, MIN_RATE..=MAX_RATE).step_by(0.05).custom_formatter(|v, _| format!("{v:.2}×"))).labelled_by(label.id);
        if r.changed() {
            let _ = app.run("readAloud.speed", json!({"value": rate}));
        }
    });
    let mut skip = app.session.read_aloud.skip_citations;
    if ui.checkbox(&mut skip, tl!("Skip citations & bibliography")).changed() {
        let _ = app.run("readAloud.skipCitations", json!({"value": skip}));
    }
}

/// Draws Advanced options: typing and overtype behavior, alignment guides, recent-document count,
/// measurement units, region, keytips, caret animation, file locations and customization.
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
        let recent_label = ui.label(tl!("Recent documents to show:"));
        let mut n = app.ui.recent_shown();
        if ui.add(egui::DragValue::new(&mut n).range(0..=crate::MAX_RECENT).speed(0.2)).labelled_by(recent_label.id).changed() {
            app.ui.recent_count = n;
        }
        ui.end_row();
        let units_label = ui.label(tl!("Measurement units:"));
        // The user's choice, else the region's (#838): Automatic follows the region.
        let units = app.session.units();
        let auto = app.session.prefs.units.is_none();
        let name = UNITS.iter().find(|(u, _)| *u == units).map_or("Inches", |(_, n)| n);
        let shown = if auto { format!("{} ({})", tl!("Automatic"), tl!(name)) } else { tl!(name).to_string() };
        let combo = craft_ui::ComboBox::from_id_salt("options_units").selected_text(shown).width(200.0).show_ui(ui, |ui| {
            if ui.selectable_label(auto, tl!("Automatic")).clicked() {
                set(app, json!({"units": "auto"}));
            }
            for (u, label) in UNITS {
                if ui.selectable_label(!auto && units == u, tl!(label)).clicked() {
                    set(app, json!({"units": u}));
                }
            }
        });
        combo.response.labelled_by(units_label.id);
        ui.end_row();
        // The region new documents, units and the decimal separator follow; Automatic is the
        // system's (#838).
        let region_label = ui.label(tl!("Region:"));
        let chosen = app.session.prefs.region.clone();
        let system = &app.session.system_region;
        let system_tag = if system.territory.is_empty() { system.language.clone() } else { format!("{}-{}", system.language, system.territory) };
        let automatic = crate::i18n::fmt(tl!("Automatic ({region})"), &[("region", system_tag.as_str())]);
        let shown = chosen.clone().unwrap_or_else(|| automatic.clone());
        let combo = craft_ui::ComboBox::from_id_salt("options_region").selected_text(shown).width(200.0).show_ui(ui, |ui| {
            if ui.selectable_label(chosen.is_none(), automatic.as_str()).clicked() {
                set(app, json!({"region": "auto"}));
            }
            for tag in REGIONS {
                if ui.selectable_label(chosen.as_deref() == Some(tag), tag).clicked() {
                    set(app, json!({"region": tag}));
                }
            }
        });
        combo.response.labelled_by(region_label.id);
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
    let id = egui::Id::unique("options_user_templates_folder");
    let mut folder = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| saved.clone());
    let default = app.services.templates_dir.as_ref().map(|d| d.to_string_lossy().into_owned());
    let templates_label = ui.label(tl!("User templates:")).id;
    ui.horizontal(|ui| {
        let hint = default.clone().unwrap_or_else(|| tl!("Not set").to_string());
        let r = ui.add(egui::TextEdit::singleline(&mut folder).desired_width(320.0).hint_text(hint)).labelled_by(templates_label);
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
        let label = ui.label(tl!("Interface theme:"));
        let combo = craft_ui::ComboBox::from_id_salt("interface_theme").selected_text(tl!(app.ui.theme.label())).width(220.0).show_ui(ui, |ui| {
            for a in Appearance::ALL {
                if ui.selectable_label(app.ui.theme == a, tl!(a.label())).clicked() {
                    let _ = app.run("ui.theme", json!({"value": a.code()}));
                }
            }
        });
        combo.response.labelled_by(label.id);
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
        let label = ui.label(tl!("Interface size:"));
        let combo = craft_ui::ComboBox::from_id_salt("interface_size").selected_text(selected).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(size.is_auto(), auto).clicked() {
                let _ = app.run("ui.interfaceSize", json!({"value": "auto"}));
            }
            for pct in InterfaceSize::PRESETS {
                if ui.selectable_label(size.percent() == Some(pct), format!("{pct}%")).clicked() {
                    let _ = app.run("ui.interfaceSize", json!({"percent": pct}));
                }
            }
        });
        combo.response.labelled_by(label.id);
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
        let label = ui.label(tl!("Animation effects:"));
        let combo =
            craft_ui::ComboBox::from_id_salt("animation_effects").selected_text(tl!(app.ui.animations.label())).width(220.0).show_ui(ui, |ui| {
                for a in Animations::ALL {
                    if ui.selectable_label(app.ui.animations == a, tl!(a.label())).clicked() {
                        let _ = app.run("ui.animations", json!({"value": a.code()}));
                    }
                }
            });
        combo.response.labelled_by(label.id);
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
        let label = ui.label(tl!("Interface language:"));
        let combo = craft_ui::ComboBox::from_id_salt("interface_language").selected_text(current).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(app.ui.language == AUTO, auto_label.as_str()).clicked() {
                let _ = app.run("ui.language", json!({"value": AUTO}));
            }
            for lang in Lang::for_picker() {
                if ui.selectable_label(app.ui.language == lang.code(), lang.name()).clicked() {
                    let _ = app.run("ui.language", json!({"value": lang.code()}));
                }
            }
        });
        combo.response.labelled_by(label.id);
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
        let label = ui.label(tl!("Numerals:"));
        let mut sel = app.session.view.numeral;
        let combo = craft_ui::ComboBox::from_id_salt("numerals").selected_text(tl!(sel.label())).width(220.0).show_ui(ui, |ui| {
            for m in [NumeralMode::Arabic, NumeralMode::Hindi, NumeralMode::Context, NumeralMode::System] {
                ui.selectable_value(&mut sel, m, tl!(m.label()));
            }
        });
        combo.response.labelled_by(label.id);
        if sel != app.session.view.numeral {
            let _ = app.run("view.numeral", json!({ "value": sel }));
        }
    });
}

#[cfg(test)]
mod tests;
