//! File › Options (#485): a page list on the left (General, Display, Proofing, Save, Language,
//! Accessibility, Advanced, Agents), the chosen page's settings on the right, shown through the
//! framework's `PreferencesPage`. Only settings WordCraft carries out are shown. Every row applies
//! at once through the command agents use (`file.options` and the `ui.*`/`view.*` commands), so a
//! setting an agent changes and the same setting changed here take one path. Settings the app keeps
//! outside the engine are written directly by [`OptionsModel::set`].

use craft::behavior::{EditEvent, MotionMode, keyed};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show, ShowCollection, ShowContainer};
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::preferences::{GroupSpec, PageSpec, PrefValue, PreferencesModel, PreferencesPage, RowKind, RowSpec};
use craft::ui::runtime::env::{LengthUnits, UnitKind, Units};
use craft::ui::runtime::memory::Remember;
#[cfg(not(target_arch = "wasm32"))]
use craft::ui::runtime::windows::WindowHost;
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};
use wordcraft_engine::cmd::options::{MAX_INITIALS_CHARS, MAX_WORD_CHARS, PANES};
use wordcraft_geom::Unit;

use crate::WordApp;

/// `v` points in `unit`, written with `fmt`'s decimal separator (`2,54 cm`), with the unit named in
/// the interface language (`2,54 см` in Russian).
pub fn length(v: f32, unit: Unit, fmt: &craft::i18n::Format) -> String {
    format!("{}{}", unit.number(v, fmt.separators().0), unit_suffix(unit))
}

/// [`length`] under the name dialogs use for a length in a chosen unit.
pub fn length_in(unit: Unit, v: f32, fmt: &craft::i18n::Format) -> String {
    length(v, unit, fmt)
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

/// The framework's unit kind of an interface length unit: what a number field counts in.
pub fn unit_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Inches => UnitKind::Inches,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Points => UnitKind::Points,
        Unit::Picas => UnitKind::Picas,
    }
}

/// The one [`LengthUnits`] every length field reads: stored in points, shown in `unit`, at the
/// document's 96 pixels per inch.
pub fn lengths(unit: Unit) -> LengthUnits {
    LengthUnits::new(UnitKind::Points, unit_kind(unit), 96.0)
}

/// A length typed into a field in `unit`, as a number of that unit: either decimal separator,
/// another unit (`1 in` in a centimetre field), and the units' names in the interface language as
/// well as in English (`3 см`). `units` is the scope's (`Env::of(ui).units()`).
pub fn parse_length(text: &str, unit: Unit, units: &Units) -> Option<f32> {
    units.reader(Some(unit_kind(unit))).read(text, None).ok().map(|v| v as f32)
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

/// The id a value has in a choice row: its serialized string (what the commands take back).
fn id_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// A value's default, found from a value of its type.
fn default_of<T: Default>(_: &T) -> T {
    T::default()
}

/// Choice entries from (id, label) pairs.
fn entries<'a>(items: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<Entry<&'a str>> {
    items.into_iter().map(|(id, label)| Entry::new(id, label.to_string())).collect()
}

/// Choice entries from owned pairs.
fn owned(items: &[(String, String)]) -> Vec<Entry<&str>> {
    entries(items.iter().map(|(id, label)| (id.as_str(), label.as_str())))
}

/// A page of titled groups.
fn page_of<'a>(id: &'a str, title: &'a str, groups: Vec<GroupSpec<'a>>) -> PageSpec<'a> {
    let mut page = PageSpec::new(id, title, Vec::new());
    page.groups = groups;
    page
}

/// Labels and ids built from the session for one pass; a row borrows them.
struct Strings {
    sizes: Vec<(String, String)>,
    size_id: String,
    languages: Vec<(String, String)>,
    units: Vec<(String, String)>,
    unit_id: String,
    regions: Vec<(String, String)>,
    region_id: String,
    paper: Vec<(String, String)>,
    paper_id: String,
    numerals: Vec<(String, String)>,
    numeral_id: String,
    proofing_language: String,
    autosave_reason: String,
}

impl Strings {
    fn build(app: &mut WordApp, native: f32) -> Strings {
        use crate::i18n::{AUTO, Lang};
        use crate::theme::InterfaceSize;
        use wordcraft_engine::cmd::view::PaperColor;
        use wordcraft_layout::NumeralMode;

        let drawn = InterfaceSize::AUTO.drawn_percent_within(native, app.auto_scale_limit);
        // The template keeps the percent sign outside its placeholder, so `fmt` isolates the number
        // without it and a right-to-left line draws "(%200)": the sign goes inside the isolate.
        let auto = crate::i18n::fmt(tl!("Auto ({percent}%)"), &[("percent", &drawn.to_string())]).replace("\u{2069}%", "%\u{2069}");
        let mut sizes = vec![("auto".to_string(), auto)];
        sizes.extend(InterfaceSize::PRESETS.iter().map(|p| (p.to_string(), format!("{p}%"))));
        let size_id = app.ui.interface_size.percent().map_or_else(|| "auto".to_string(), |p| p.to_string());

        let system = crate::i18n::system_lang();
        let mut languages = vec![(AUTO.to_string(), crate::i18n::fmt(tl!("Automatic ({language})"), &[("language", system.name())]))];
        languages.extend(Lang::for_picker().into_iter().map(|l| (l.code().to_string(), l.name().to_string())));

        let units_now = app.session.units();
        let auto_units = app.session.prefs.units.is_none();
        let name_of = UNITS.iter().find(|(u, _)| *u == units_now).map_or("Inches", |(_, n)| n);
        let mut units = vec![("auto".to_string(), format!("{} ({})", tl!("Automatic"), tl!(name_of)))];
        units.extend(UNITS.iter().map(|(u, n)| (id_of(u), tl!(n).to_string())));
        let unit_id = if auto_units { "auto".to_string() } else { id_of(&units_now) };

        let system_region = &app.session.system_region;
        let tag = if system_region.territory.is_empty() {
            system_region.language.clone()
        } else {
            format!("{}-{}", system_region.language, system_region.territory)
        };
        let mut regions = vec![("auto".to_string(), crate::i18n::fmt(tl!("Automatic ({region})"), &[("region", tag.as_str())]))];
        regions.extend(REGIONS.iter().map(|t| (t.to_string(), t.to_string())));
        let region_id = app.session.prefs.region.clone().unwrap_or_else(|| "auto".to_string());

        let paper = PaperColor::ALL.iter().map(|c| (id_of(c), tl!(c.label()).to_string())).collect();
        let paper_id = id_of(&app.session.prefs.paper_color);
        let numerals = [NumeralMode::Arabic, NumeralMode::Hindi, NumeralMode::Context, NumeralMode::System]
            .iter()
            .map(|m| (id_of(m), tl!(m.label()).to_string()))
            .collect();
        let numeral_id = id_of(&app.session.view.numeral);

        let lang = crate::ribbon::caret_language(app);
        let proofing_language = if crate::ribbon::caret_no_proof(app) {
            tl!("The text at the insertion point is marked Do not check spelling or grammar.").to_string()
        } else if !crate::ribbon::caret_language_proofed(app) {
            crate::i18n::fmt(tl!("{language}: WordCraft doesn't check spelling or grammar in this language yet."), &[("language", &lang)])
        } else {
            let unreadable = crate::ribbon::caret_unreadable_dictionary(app);
            crate::chrome::dictionary_problem(&lang, &unreadable, crate::ribbon::caret_missing_dictionary(app))
                .unwrap_or_else(|| crate::i18n::fmt(tl!("Spelling and grammar are checked in {language}."), &[("language", &lang)]))
        };
        Strings {
            sizes,
            size_id,
            languages,
            units,
            unit_id,
            regions,
            region_id,
            paper,
            paper_id,
            numerals,
            numeral_id,
            proofing_language,
            autosave_reason: crate::AutoSaveBlock::Browser.reason().to_string(),
        }
    }
}

/// The Options page's side of the preferences page: the rows built from the app, and the commands
/// each edit runs. Built and dropped within one pass.
pub(crate) struct OptionsModel<'a> {
    app: &'a mut WordApp,
    strings: Strings,
    /// Whether the runtime's resolved motion runs animations in full: what "Use system setting"
    /// comes to while the app's choice is System.
    motion_full: bool,
}

impl<'a> OptionsModel<'a> {
    /// The model over `app`; `native` is the display's native pixels per point and `motion_full`
    /// the runtime's resolved motion.
    pub(crate) fn new(app: &'a mut WordApp, native: f32, motion_full: bool) -> Self {
        let strings = Strings::build(app, native);
        OptionsModel { app, strings, motion_full }
    }

    /// Runs a command; a refusal shows in the status bar.
    fn run(&mut self, id: &str, params: Value) {
        if let Err(e) = self.app.run(id, params) {
            self.app.status(e);
        }
    }

    fn options(&mut self, params: Value) {
        self.run("file.options", params);
    }

    /// The values `reset_page` restores, as (row key, value).
    fn defaults(&self, page: &str) -> Vec<(&'static str, PrefValue)> {
        use PrefValue::{Bool, Choice, Float, Int, Text};
        let ui = default_of(&self.app.ui);
        let prefs = default_of(&self.app.session.prefs);
        let view = default_of(&self.app.session.view);
        let size = ui.interface_size.percent().map_or_else(|| "auto".to_string(), |p| p.to_string());
        match page {
            "general" => vec![
                ("general.theme", Choice(ui.theme.code().to_string())),
                ("general.interfaceSize", Choice(size)),
                ("general.paperColor", Choice(id_of(&prefs.paper_color))),
                ("general.miniToolbar", Bool(ui.mini_toolbar)),
                ("general.darkPage", Bool(view.dark_mode)),
                ("general.showCommunity", Bool(ui.show_discord)),
                ("general.fontDownloadNotice", Bool(ui.font_download_notice)),
                ("general.startScreen", Bool(ui.start_screen)),
                ("general.initials", Text(String::new())),
            ],
            "display" => vec![
                ("display.tabs", Bool(prefs.marks.tabs)),
                ("display.spaces", Bool(prefs.marks.spaces)),
                ("display.paragraphs", Bool(prefs.marks.paragraphs)),
                ("display.hidden", Bool(prefs.marks.hidden)),
                ("display.allMarks", Bool(view.marks)),
                ("display.rulers", Bool(view.ruler)),
                ("display.numerals", Choice(id_of(&view.numeral))),
                ("display.statusCount", Choice(ui.status_count.code().to_string())),
                ("display.printBackgrounds", Bool(ui.print_backgrounds)),
                ("display.updateFieldsBeforePrint", Bool(ui.update_fields_before_print)),
            ],
            "proofing" => vec![
                ("proofing.ignoreUppercase", Bool(prefs.ignore_uppercase)),
                ("proofing.ignoreNumbers", Bool(prefs.ignore_numbers)),
                ("proofing.ignoreInternet", Bool(prefs.ignore_internet)),
                ("proofing.checkSpelling", Bool(view.proofing)),
                ("proofing.markGrammar", Bool(prefs.mark_grammar)),
                ("proofing.customDictionaryFile", Text(String::new())),
            ],
            "save" => vec![("save.removePersonalInfo", Bool(false)), ("save.removeDateTime", Bool(false))],
            "language" => vec![("language.interface", Choice(crate::i18n::AUTO.to_string())), ("language.dictionariesFolder", Text(String::new()))],
            "accessibility" => vec![
                ("accessibility.animations", Choice(ui.animations.code().to_string())),
                ("accessibility.readAloudSpeed", Float(f64::from(default_of(&self.app.session.read_aloud).rate()))),
                ("accessibility.skipCitations", Bool(default_of(&self.app.session.read_aloud).skip_citations)),
            ],
            "advanced" => vec![
                ("advanced.typingReplacesSelection", Bool(prefs.typing_replaces_selection)),
                ("advanced.insertKeyOvertype", Bool(prefs.insert_key_overtype)),
                ("advanced.overtype", Bool(prefs.overtype)),
                ("advanced.alignmentGuides", Bool(ui.alignment_guides)),
                ("advanced.recentCount", Int(ui.recent_shown() as i64)),
                ("advanced.units", Choice("auto".to_string())),
                ("advanced.region", Choice("auto".to_string())),
                ("advanced.keytips", Bool(ui.keytips_enabled)),
                ("advanced.caretAnimation", Bool(ui.caret_animation)),
                ("advanced.displayServer", Choice(ui.display_server.code().to_string())),
                ("advanced.templatesFolder", Text(String::new())),
                ("advanced.scrollSpeed", Choice(ui.scroll_speed.code().to_string())),
                ("advanced.touchpadScrollSpeed", Choice(ui.touchpad_scroll_speed.code().to_string())),
            ],
            _ => Vec::new(),
        }
    }
}

impl PreferencesModel for OptionsModel<'_> {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        use crate::StatusCount;
        use crate::scroll::ScrollSpeed;
        use crate::theme::{Animations, Appearance};
        use wordcraft_engine::speech::{MAX_RATE, MIN_RATE};
        let app = &*self.app;
        let s = &self.strings;
        let prefs = &app.session.prefs;
        let marks = prefs.marks;
        let protected = wordcraft_engine::cmd::protect::enforced(&app.session.doc);
        let settings = &app.session.doc.settings;
        let browser = app.autosave_block() == Some(crate::AutoSaveBlock::Browser);
        let b = |key, label, on| RowSpec::new(key, label, RowKind::Bool(on));
        let custom = |key, label| RowSpec::new(key, label, RowKind::Custom);
        #[cfg(not(target_arch = "wasm32"))]
        let can_default = app.services.make_default_app.is_some() || cfg!(target_os = "macos");
        #[cfg(target_arch = "wasm32")]
        let can_default = false;

        let general = page_of(
            "general",
            tl!("General"),
            vec![
                GroupSpec::new(
                    Some(tl!("Interface")),
                    vec![
                        RowSpec::new(
                            "general.theme",
                            tl!("Interface theme:"),
                            RowKind::choice(app.ui.theme.code(), entries(Appearance::ALL.iter().map(|a| (a.code(), tl!(a.label()))))),
                        ),
                        RowSpec::new("general.interfaceSize", tl!("Interface size:"), RowKind::choice(&s.size_id, owned(&s.sizes))).help(tl!(
                            "Makes the ribbon, menus and dialogs larger or smaller. Auto follows your system's display scaling; 100% is one screen pixel per point. Document zoom is separate."
                        )),
                        RowSpec::new("general.paperColor", tl!("Screen paper color:"), RowKind::choice(&s.paper_id, owned(&s.paper))).help(tl!(
                            "Screen only. Document backgrounds, printing and exports keep their colours. Dark page and explicit Read Mode colours take precedence."
                        )),
                        b("general.miniToolbar", tl!("Show the mini toolbar when text is selected"), app.ui.mini_toolbar),
                        b("general.darkPage", tl!("Dark page (white text on black)"), app.session.view.dark_mode)
                            .help(tl!("Show documents with their colours inverted, like View › Switch Modes. Saving, printing and PDFs are unchanged.")),
                        b("general.showCommunity", tl!("Show the community button in the title bar"), app.ui.show_discord),
                        b(
                            "general.fontDownloadNotice",
                            tl!("Tell me when a document's missing fonts can be downloaded"),
                            app.ui.font_download_notice,
                        ),
                        custom("general.defaultApp", tl!("Make WordCraft the default for Word documents…")).visible(can_default),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("Start-up")),
                    vec![b("general.startScreen", tl!("Show the Start screen when WordCraft starts"), app.ui.start_screen).help(tl!(
                        "On: WordCraft opens on File › Home, with a blank document, the templates and your recent documents. Off: it opens a blank document."
                    ))],
                ),
                GroupSpec::new(
                    Some(tl!("Personalize")),
                    vec![
                        RowSpec::new("general.userName", tl!("User name:"), RowKind::Text(&app.session.author)).enabled(!protected),
                        RowSpec::new("general.initials", tl!("Initials:"), RowKind::Text(&app.session.initials)),
                    ],
                ),
            ],
        )
        .footnote(tl!("New comments and tracked changes are recorded under this name and these initials."));

        let display = page_of(
            "display",
            tl!("Display"),
            vec![
                GroupSpec::new(
                    Some(tl!("Formatting marks")),
                    vec![
                        b("display.tabs", tl!("Tab characters"), marks.tabs),
                        b("display.spaces", tl!("Spaces"), marks.spaces),
                        b("display.paragraphs", tl!("Paragraph marks"), marks.paragraphs),
                        b("display.hidden", tl!("Hidden text"), marks.hidden),
                        b("display.allMarks", tl!("Show all formatting marks"), app.session.view.marks),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("Page display")),
                    vec![
                        b("display.rulers", tl!("Show rulers"), app.session.view.ruler),
                        RowSpec::new("display.numerals", tl!("Numerals:"), RowKind::choice(&s.numeral_id, owned(&s.numerals))),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("Status bar")),
                    vec![
                        RowSpec::new(
                            "display.statusCount",
                            tl!("Status bar count:"),
                            RowKind::choice(app.ui.status_count.code(), entries(StatusCount::ALL.iter().map(|c| (c.code(), tl!(c.label()))))),
                        )
                        .help(tl!("You can also right-click the count in the status bar to change it.")),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("Printing")),
                    vec![
                        b("display.printBackgrounds", tl!("Print the page color"), app.ui.print_backgrounds),
                        b("display.updateFieldsBeforePrint", tl!("Update fields before printing"), app.ui.update_fields_before_print),
                    ],
                ),
            ],
        )
        .footnote(tl!("These marks show even when Show/Hide ¶ is off."));

        let proofing = page_of(
            "proofing",
            tl!("Proofing"),
            vec![
                GroupSpec::new(Some(tl!("AutoCorrect")), vec![custom("proofing.autoCorrect", tl!("AutoCorrect Options"))]),
                GroupSpec::new(
                    Some(tl!("Spelling")),
                    vec![
                        b("proofing.ignoreUppercase", tl!("Ignore words in capitals"), prefs.ignore_uppercase),
                        b("proofing.ignoreNumbers", tl!("Ignore words with numbers"), prefs.ignore_numbers),
                        b("proofing.ignoreInternet", tl!("Ignore web and file addresses"), prefs.ignore_internet),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("As you type")),
                    vec![
                        b("proofing.checkSpelling", tl!("Check spelling as you type"), app.session.view.proofing),
                        b("proofing.markGrammar", tl!("Mark grammar errors as you type"), prefs.mark_grammar).enabled(app.session.view.proofing),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("Custom dictionary")),
                    vec![
                        RowSpec::new("proofing.customDictionaryFile", tl!("Dictionary file:"), RowKind::Custom)
                            .help(tl!("A custom-word dictionary file (.dic or plain text) used in conjunction with the main dictionary. Empty uses custom-dictionary.txt.")),
                        custom("proofing.customDictionary", tl!("Custom dictionary")),
                    ],
                ),
            ],
        );

        let save = page_of(
            "save",
            tl!("Save"),
            vec![
                GroupSpec::new(Some(tl!("Save documents")), {
                    // The browser can't write to the user's files, so AutoSave can't be turned on there (#176).
                    let row = b("save.autosave", tl!("AutoSave documents you have saved in WordCraft"), app.autosave && !browser).enabled(!browser);
                    vec![if browser { row.help(&s.autosave_reason) } else { row }]
                }),
                // The open document's `file.privacyOptions` (#1062): saved with it, honoured when it is saved.
                GroupSpec::new(
                    Some(tl!("Privacy for this document")),
                    vec![
                        b(
                            "save.removePersonalInfo",
                            tl!("Remove my name from comments, changes and file properties when saving"),
                            settings.remove_personal_info,
                        ),
                        b(
                            "save.removeDateTime",
                            tl!("Leave out the date and time of comments and tracked changes when saving"),
                            settings.remove_date_time,
                        ),
                    ],
                ),
            ],
        )
        .footnote(tl!("AutoSave writes your changes to the file a moment after you stop typing."));

        let language = page_of(
            "language",
            tl!("Language"),
            vec![
                GroupSpec::new(
                    Some(tl!("Interface language")),
                    vec![
                        RowSpec::new("language.interface", tl!("Interface language:"), RowKind::choice(&app.ui.language, owned(&s.languages)))
                            .help(tl!("Automatic follows your system's language. Menus and commands change; your documents don't.")),
                    ],
                ),
                GroupSpec::new(
                    Some(tl!("Proofing language")),
                    vec![custom("language.proofingLanguage", tl!("Proofing language")).keywords(&s.proofing_language)],
                ),
                GroupSpec::new(Some(tl!("Your languages")), vec![custom("language.yourLanguages", tl!("Your languages"))]),
                GroupSpec::new(
                    Some(tl!("Dictionaries")),
                    vec![
                        RowSpec::new(
                            "language.dictionariesFolder",
                            tl!("Dictionaries folder:"),
                            RowKind::Text(prefs.dictionaries_folder.as_deref().unwrap_or("")),
                        )
                        .help(tl!("Hunspell .aff and .dic files here (pl_PL, pt_BR, it_IT, hr_HR, tr_TR) are used before the system's.\nAn installed Language pack is used after this folder.")),
                        custom("language.dictionaryFiles", tl!("Dictionary files")),
                    ],
                ),
            ],
        );

        let animations_help = if app.ui.animations == Animations::System {
            Some(if self.motion_full {
                tl!("Your system has animation effects turned on.")
            } else {
                tl!("Your system has animation effects turned off.")
            })
        } else {
            None
        };
        let mut animations = RowSpec::new(
            "accessibility.animations",
            tl!("Animation effects:"),
            RowKind::choice(app.ui.animations.code(), entries(Animations::ALL.iter().map(|a| (a.code(), tl!(a.label()))))),
        );
        animations = animations.help(
            animations_help.unwrap_or(tl!("Smooth scrolling and zooming, and menus and buttons that fade in. Off shows every change at once.")),
        );
        let accessibility = page_of(
            "accessibility",
            tl!("Accessibility"),
            vec![
                GroupSpec::new(Some(tl!("Visual effects")), vec![animations]),
                GroupSpec::new(
                    Some(tl!("Read Aloud")),
                    vec![
                        RowSpec::new(
                            "accessibility.readAloudSpeed",
                            tl!("Speed"),
                            RowKind::float(app.session.read_aloud.rate() as f64, (MIN_RATE as f64)..=(MAX_RATE as f64), 2),
                        )
                        .step(0.05),
                        b("accessibility.skipCitations", tl!("Skip citations & bibliography"), app.session.read_aloud.skip_citations),
                    ],
                ),
            ],
        );

        let display_group = vec![
            RowSpec::new(
                "advanced.recentCount",
                tl!("Recent documents to show:"),
                RowKind::int(app.ui.recent_shown() as i64, 0..=(crate::MAX_RECENT as i64)),
            ),
            RowSpec::new("advanced.units", tl!("Measurement units:"), RowKind::choice(&s.unit_id, owned(&s.units))),
            RowSpec::new("advanced.region", tl!("Region:"), RowKind::choice(&s.region_id, owned(&s.regions))),
            b("advanced.keytips", tl!("Show KeyTips (Alt or F10)"), app.ui.keytips_enabled),
        ];

        let mut groups = vec![
            GroupSpec::new(
                Some(tl!("Editing")),
                vec![
                    b("advanced.typingReplacesSelection", tl!("Typing replaces selected text"), prefs.typing_replaces_selection),
                    b("advanced.insertKeyOvertype", tl!("Use the Insert key to switch overtype on and off"), prefs.insert_key_overtype),
                    b("advanced.overtype", tl!("Overtype (typing replaces the characters after the caret)"), prefs.overtype),
                    b("advanced.alignmentGuides", tl!("Show alignment guides when moving pictures and shapes"), app.ui.alignment_guides).help(tl!(
                        "Moved objects snap to the page's edges and centre, the margins and other objects. Hold Alt to move freely."
                    )),
                ],
            ),
            GroupSpec::new(Some(tl!("Display")), display_group),
            GroupSpec::new(
                Some(tl!("Animation")),
                vec![b("advanced.caretAnimation", tl!("Animate the caret (it glides and fades)"), app.ui.caret_animation).help(tl!(
                    "On: the caret glides to its new place when you type or use the arrow keys, and its blink fades. Off: it jumps and blinks on and off."
                ))],
            ),
        ];
        if cfg!(all(unix, not(target_os = "macos"))) {
            use crate::DisplayServer;
            let name = |d: &DisplayServer| if *d == DisplayServer::Auto { tl!(d.label()) } else { d.label() };
            groups.push(GroupSpec::new(
                Some(tl!("Linux display server")),
                vec![
                    RowSpec::new(
                        "advanced.displayServer",
                        tl!("Display server:"),
                        RowKind::choice(app.ui.display_server.code(), entries(DisplayServer::ALL.iter().map(|d| (d.code(), name(d))))),
                    )
                    .help(tl!("Applies the next time WordCraft starts.")),
                ],
            ));
        }
        // The web app has no folders to name.
        if !cfg!(target_arch = "wasm32") {
            groups.push(GroupSpec::new(
                Some(tl!("File locations")),
                vec![
                    RowSpec::new(
                        "advanced.templatesFolder",
                        tl!("User templates:"),
                        RowKind::Text(prefs.user_templates_folder.as_deref().unwrap_or("")),
                    )
                    .help(tl!("File › New lists the .dotx and .dotm templates here under Personal; Save as Word Template starts here.")),
                ],
            ));
        }

        let mut scrolling = vec![RowSpec::new(
            "advanced.scrollSpeed",
            tl!("Mouse wheel scroll speed:"),
            RowKind::choice(app.ui.scroll_speed.code(), entries(ScrollSpeed::ALL.iter().map(|s| (s.code(), tl!(s.label()))))),
        )];
        // Windows and browsers do not report touchpads in points.
        if !cfg!(any(target_os = "windows", target_arch = "wasm32")) {
            let mut row = RowSpec::new(
                "advanced.touchpadScrollSpeed",
                tl!("Touchpad scroll speed:"),
                RowKind::choice(app.ui.touchpad_scroll_speed.code(), entries(ScrollSpeed::ALL.iter().map(|s| (s.code(), tl!(s.label()))))),
            );
            if !cfg!(target_os = "macos") {
                row = row.help(tl!("On X11, the touchpad follows the mouse wheel speed."));
            }
            scrolling.push(row);
        }
        groups.push(GroupSpec::new(Some(tl!("Scrolling")), scrolling));
        groups.push(GroupSpec::new(
            Some(tl!("Customize")),
            vec![
                custom("advanced.customizeKeyboard", tl!("Keyboard shortcuts:")),
                custom("advanced.customizeRibbon", tl!("Ribbon and Quick Access Toolbar:")),
            ],
        ));
        let advanced = page_of("advanced", tl!("Advanced"), groups);

        let agents = PageSpec::custom("agents", tl!("Agents"));
        vec![general, display, proofing, save, language, accessibility, advanced, agents]
    }

    fn set(&mut self, key: &str, value: PrefValue) {
        use PrefValue::{Bool, Choice, Float, Int, Text};
        match (key, value) {
            ("general.theme", Choice(c)) => self.run("ui.theme", json!({"value": c})),
            ("general.interfaceSize", Choice(c)) => match c.parse::<u16>() {
                Ok(percent) => self.run("ui.interfaceSize", json!({"percent": percent})),
                Err(_) => self.run("ui.interfaceSize", json!({"value": "auto"})),
            },
            ("general.paperColor", Choice(c)) => self.run("view.paperColor", json!({"value": c})),
            ("general.miniToolbar", Bool(v)) => self.app.ui.mini_toolbar = v,
            ("general.showCommunity", Bool(v)) => self.app.ui.show_discord = v,
            ("general.fontDownloadNotice", Bool(v)) => self.app.ui.font_download_notice = v,
            ("general.darkPage", Bool(v)) => self.run("view.darkMode", json!({"value": v})),
            ("general.startScreen", Bool(v)) => self.run("ui.startScreen", json!({"value": v})),
            ("general.userName", Text(t)) => {
                if !wordcraft_engine::cmd::protect::enforced(&self.app.session.doc) {
                    self.app.session.author = t;
                }
            }
            ("general.initials", Text(t)) => {
                let t: String = t.chars().take(MAX_INITIALS_CHARS).collect();
                self.options(json!({"initials": t}));
            }
            ("display.tabs", Bool(v)) => self.options(json!({"marks": {"tabs": v}})),
            ("display.spaces", Bool(v)) => self.options(json!({"marks": {"spaces": v}})),
            ("display.paragraphs", Bool(v)) => self.options(json!({"marks": {"paragraphs": v}})),
            ("display.hidden", Bool(v)) => self.options(json!({"marks": {"hidden": v}})),
            ("display.allMarks", Bool(v)) => self.run("view.marks", json!({"value": v})),
            ("display.rulers", Bool(v)) => self.run("view.ruler", json!({"value": v})),
            ("display.numerals", Choice(c)) => self.run("view.numeral", json!({"value": c})),
            ("display.statusCount", Choice(c)) => self.run("ui.statusCount", json!({"value": c})),
            ("display.printBackgrounds", Bool(v)) => self.app.ui.print_backgrounds = v,
            ("display.updateFieldsBeforePrint", Bool(v)) => self.app.ui.update_fields_before_print = v,
            ("proofing.ignoreUppercase", Bool(v)) => self.options(json!({"ignoreUppercase": v})),
            ("proofing.ignoreNumbers", Bool(v)) => self.options(json!({"ignoreNumbers": v})),
            ("proofing.ignoreInternet", Bool(v)) => self.options(json!({"ignoreInternet": v})),
            ("proofing.checkSpelling", Bool(v)) => self.options(json!({"checkSpelling": v})),
            ("proofing.markGrammar", Bool(v)) => self.options(json!({"markGrammar": v})),
            ("save.autosave", Bool(v)) => {
                if self.app.autosave_block() != Some(crate::AutoSaveBlock::Browser) {
                    self.app.autosave = v;
                    self.app.session.autosave = v;
                }
            }
            ("save.removePersonalInfo", Bool(v)) => self.run("file.privacyOptions", json!({"removePersonalInfo": v})),
            ("save.removeDateTime", Bool(v)) => self.run("file.privacyOptions", json!({"removeDateTime": v})),
            ("language.interface", Choice(c)) => self.run("ui.language", json!({"value": c})),
            ("proofing.customDictionaryFile", Text(t)) => {
                let t = t.trim();
                self.options(json!({"customDictionary": if t.is_empty() { "auto" } else { t }}));
            }
            ("language.dictionariesFolder", Text(t)) => {
                let t = t.trim();
                self.options(json!({"dictionariesFolder": if t.is_empty() { "auto" } else { t }}));
            }
            ("accessibility.animations", Choice(c)) => self.run("ui.animations", json!({"value": c})),
            ("accessibility.readAloudSpeed", Float(v)) => self.run("readAloud.speed", json!({"value": v})),
            ("accessibility.skipCitations", Bool(v)) => self.run("readAloud.skipCitations", json!({"value": v})),
            ("advanced.typingReplacesSelection", Bool(v)) => self.options(json!({"typingReplacesSelection": v})),
            ("advanced.insertKeyOvertype", Bool(v)) => self.options(json!({"insertKeyOvertype": v})),
            ("advanced.overtype", Bool(v)) => self.options(json!({"overtype": v})),
            ("advanced.alignmentGuides", Bool(v)) => self.run("ui.alignmentGuides", json!({"value": v})),
            ("advanced.recentCount", Int(n)) => self.app.ui.recent_count = n.clamp(0, crate::MAX_RECENT as i64) as _,
            ("advanced.units", Choice(c)) => self.options(json!({"units": c})),
            ("advanced.region", Choice(c)) => self.options(json!({"region": c})),
            ("advanced.keytips", Bool(v)) => self.run("ui.keytips", json!({"value": v})),
            ("advanced.caretAnimation", Bool(v)) => self.run("ui.caretAnimation", json!({"value": v})),
            ("advanced.displayServer", Choice(c)) => self.run("ui.displayServer", json!({"value": c})),
            ("advanced.templatesFolder", Text(t)) => {
                let t = t.trim();
                self.options(json!({"userTemplatesFolder": if t.is_empty() { "auto" } else { t }}));
            }
            ("advanced.scrollSpeed", Choice(c)) => self.run("ui.scrollSpeed", json!({"value": c})),
            ("advanced.touchpadScrollSpeed", Choice(c)) => self.run("ui.touchpadScrollSpeed", json!({"value": c})),
            _ => {}
        }
    }

    fn custom_row(&mut self, ui: &mut Ui, key: &str) {
        match key {
            "general.defaultApp" => default_app_row(self.app, ui),
            "proofing.autoCorrect" => {
                if Button::new(tl!("AutoCorrect Options")).show(ui).clicked() {
                    self.run("tools.autocorrect", json!({}));
                }
            }
            "proofing.customDictionaryFile" => self.custom_dictionary_file(ui),
            "proofing.customDictionary" => self.custom_dictionary(ui),
            "language.proofingLanguage" => {
                Text::new(self.strings.proofing_language.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
            }
            // PLAN: assumed `your_languages(app: &mut WordApp, ui: &mut Ui)` (chrome-w12)
            "language.yourLanguages" => crate::language_packs::your_languages(self.app, ui),
            "language.dictionaryFiles" => dictionary_files(ui),
            "advanced.customizeKeyboard" => {
                if Button::new(tl!("Customize…")).show(ui).clicked() {
                    self.run("tools.customizeKeyboard", json!({}));
                }
            }
            "advanced.customizeRibbon" if Button::new(tl!("Customize…")).show(ui).clicked() => {
                self.run("tools.customizeRibbon", json!({}));
            }
            _ => {}
        }
    }

    fn custom_page(&mut self, ui: &mut Ui, page: &str) {
        if page == "agents" {
            Text::new(tl!("Agents")).role(TypeRole::Heading).show(ui);
            Text::new(tl!("Every command is available to scripts and AI agents: run `wordcraft-cli mcp` for an MCP server, or start the app with `--control <port>` for the JSON control channel."))
                .show(ui);
        }
    }

    fn reset_page(&mut self, page: &str) {
        for (key, value) in self.defaults(page) {
            self.set(key, value);
        }
    }
}

/// Rows of the custom dictionary's list shown before it scrolls.
const CUSTOM_WORDS_SHOWN: u16 = 8;

/// The word being typed into the custom dictionary's new-word field, kept in the session tier of
/// the row's container so an edit in progress survives passes.
#[derive(Clone, Default)]
struct NewWord(String);

impl OptionsModel<'_> {
    /// The Dictionary file field. A text row reports every typed draft, and each would bind (and
    /// later write to) whatever file the half-typed path names, so the path is committed only on
    /// Enter or focus loss. The field is given the saved path and never writes it: the framework
    /// holds the draft while an edit is open and drops it when the saved path changes (an agent's
    /// `file.options`) or the page is left, so no edit open always shows the saved path.
    fn custom_dictionary_file(&mut self, ui: &mut Ui) {
        let saved = self.app.session.prefs.custom_dictionary.clone().unwrap_or_default();
        let out = TextField::new(tl!("Dictionary file:"), Bind::given(saved)).limit(None).show(ui);
        let committed = out.events.into_iter().find_map(|event| match event {
            EditEvent::Commit { after, .. } => Some(after),
            _ => None,
        });
        if let Some(path) = committed {
            self.set("proofing.customDictionaryFile", PrefValue::Text(path));
        }
    }

    /// The custom dictionary: the words Add to Dictionary kept, each with a Remove button, and a
    /// field to add one.
    fn custom_dictionary(&mut self, ui: &mut Ui) {
        let words = wordcraft_engine::proof::user_dictionary();
        if words.is_empty() {
            Text::new(tl!("No words yet. Add to Dictionary on a misspelled word puts it here.")).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
        }
        let removed = std::cell::Cell::new(None::<String>);
        let removed_ref = &removed;
        let source = keyed(&words, |w: &String| w.clone());
        // A collection owns its scroll and the page scrolls, so the list caps its rows (the dictionary can hold any number).
        List::new(tl!("Custom dictionary")).max_rows(CUSTOM_WORDS_SHOWN).show_items(ui, &source, &mut |word, _| {
            let word = word.clone();
            ListRow::new(word.clone()).trailing_field(Button::new(tl!("Remove")), move |out| {
                if out.clicked() {
                    removed_ref.set(Some(word));
                }
            })
        });
        if let Some(w) = removed.take() {
            self.options(json!({"removeWords": [w]}));
        }
        // Enter or Add files the draft and clears it.
        Row::new().show(ui, |ui, cx| {
            let draft = &mut cx.keep::<NewWord>().0;
            let out = TextField::new(tl!("New word"), Bind::bound(&mut *draft)).limit(MAX_WORD_CHARS).placeholder(tl!("New word")).submits().show(ui);
            let word = draft.trim().to_string();
            let ok = !word.is_empty() && !word.chars().any(char::is_whitespace);
            let add = Button::new(tl!("Add")).enabled(ok).show(ui).clicked();
            if ok && (add || out.submitted) {
                self.options(json!({"addWords": [word]}));
                draft.clear();
            }
        });
    }
}

/// The dictionary each language found (#1063): its file, "no dictionary found", or why the file
/// couldn't be read. The engine keeps the report until the dictionaries change.
fn dictionary_files(ui: &mut Ui) {
    let direction = if crate::i18n::current().rtl() { craft::i18n::Direction::Rtl } else { craft::i18n::Direction::Ltr };
    for found in wordcraft_engine::packs::dictionaries_found().iter() {
        let language = language_name(found.lang);
        let file = found.file.as_ref().map(|f| craft::i18n::path(&f.to_string_lossy(), direction));
        let line = match (file, found.error.as_deref()) {
            (Some(f), Some(e)) => {
                crate::i18n::fmt(tl!("{language}: {file} (unreadable: {reason})"), &[("language", language), ("file", &f), ("reason", e)])
            }
            (Some(f), None) => crate::i18n::fmt(tl!("{language}: {file}"), &[("language", language), ("file", &f)]),
            (None, _) => crate::i18n::fmt(tl!("{language}: no dictionary found"), &[("language", language)]),
        };
        Text::new(line).role(TypeRole::Caption).shade(Shade::Faint).elide(Elide::End).show(ui);
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
        ProofLang::En
        | ProofLang::EnGb
        | ProofLang::EnCa
        | ProofLang::EnAu
        | ProofLang::Bg
        | ProofLang::De
        | ProofLang::Pack(_)
        | ProofLang::Other => "",
    }
}

/// File ▸ Options ▸ General: make WordCraft the default app for Word documents (#295). Only where
/// the host can (`Services::make_default_app`: Windows opens Default apps, Linux and BSD use
/// `xdg-mime`); macOS shows how to do it in Finder, the web shows nothing.
#[cfg(not(target_arch = "wasm32"))]
fn default_app_row(app: &mut WordApp, ui: &mut Ui) {
    if app.services.make_default_app.is_some() {
        let hint = if cfg!(target_os = "windows") {
            tl!("Opens Settings › Default apps, where you choose WordCraft for .docx and the other document types.")
        } else {
            tl!("Opens .docx, .doc, .rtf and .odt files and Word templates with WordCraft from now on.")
        };
        let clicked = Button::new(tl!("Make WordCraft the default for Word documents…")).tooltip(hint).show(ui).clicked();
        // Only a person's click changes system defaults: an automation client's pass does not.
        if clicked && !WindowHost::of(ui.ctx()).injected_pass() {
            let _ = app.make_default_app();
        }
    } else if cfg!(target_os = "macos") {
        Text::new(tl!("To open Word documents with WordCraft by default: select one in Finder, choose File › Get Info, pick WordCraft under Open with, then click Change All."))
            .role(TypeRole::Caption)
            .shade(Shade::Faint)
            .show(ui);
    }
}

/// The web has no system default apps to change: nothing to show.
#[cfg(target_arch = "wasm32")]
fn default_app_row(_app: &mut WordApp, _ui: &mut Ui) {}

/// The Options page.
pub fn page(app: &mut WordApp, ui: &mut Ui) {
    // The page title, as the other File pages' (production's 26 pt semibold).
    Text::new(tl!("Options")).role(TypeRole::Display).show(ui);
    let current = if PANES.contains(&app.ui.options_pane.as_str()) { app.ui.options_pane.clone() } else { "general".to_string() };
    let native = ui.ctx().native_pixels_per_point().unwrap_or(1.0);
    // The page says what the system answered, so it reads the resolved choice: the env's own
    // motion still holds `System` while that is followed.
    let motion_full = app.resolved_root().motion == MotionMode::Full;
    // The page list is the state's; a command that set `options_pane` since the last pass wins.
    // PLAN: assumed field `options_state: PreferencesState` on WordApp (app-hub)
    let mut state = std::mem::take(&mut app.options_state);
    state.selected = current;
    {
        let mut model = OptionsModel::new(app, native, motion_full);
        PreferencesPage::new(Bind::bound(&mut state)).show(ui, &mut model);
    }
    if PANES.contains(&state.selected.as_str()) {
        app.ui.options_pane = state.selected.clone();
    }
    app.options_state = state;
}

#[cfg(test)]
mod tests;
