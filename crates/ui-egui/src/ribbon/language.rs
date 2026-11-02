use super::{Ui, Value, WordApp, json, vec2};

/// The proofing languages the Language menu offers: tag and English name (translated when drawn).
pub(crate) const PROOFING_LANGUAGES: [(&str, &str); 16] = [
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en-AU", "English (Australia)"),
    ("en-CA", "English (Canada)"),
    ("pl-PL", "Polish"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("it-IT", "Italian"),
    ("hr-HR", "Croatian"),
    ("tr-TR", "Turkish"),
    ("ru-RU", "Russian"),
    ("cs-CZ", "Czech"),
    ("id-ID", "Indonesian"),
    ("es-ES", "Spanish (Spain)"),
    ("es-MX", "Spanish (Mexico)"),
    ("es-419", "Spanish (Latin America)"),
    ("es-UY", "Spanish (Uruguay)"),
];

/// The English name of a proofing language tag, as the status bar shows it (no tag: WordCraft's
/// default, US English): a listed language by its name; another region of one by the language
/// and the region ("French (Canada)"); a bare language subtag by its usual variant; else the tag.
pub(crate) fn language_name(tag: Option<&str>) -> String {
    let tag = tag.unwrap_or("en-US");
    let named =
        |t: &str| PROOFING_LANGUAGES.iter().chain(crate::dialogs_layout::LANGUAGES.iter()).find(|(l, _)| l.eq_ignore_ascii_case(t)).map(|(_, n)| *n);
    // Any language the Language dialog offers (French, German…) by its name, not its tag (#860).
    if let Some(name) = named(tag) {
        return name.to_string();
    }
    let mut parts = tag.split(['-', '_']);
    let primary = parts.next().unwrap_or("");
    // The region: the first two-letter or three-digit subtag after the language (and script).
    let region =
        parts.find(|p| (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic())) || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())));
    match region {
        // A region the list doesn't have: the language's name without its own region or variant.
        Some(r) => {
            let language = crate::dialogs_layout::LANGUAGES
                .iter()
                .find(|(l, _)| l.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(primary)))
                .map(|(_, n)| n.split(" (").next().unwrap_or(n));
            match language {
                Some(language) => format!("{language} ({})", region_name(r)),
                None => tag.to_string(),
            }
        }
        // A bare language subtag ("pt", "pl"): the variant WordCraft proofs.
        None => match wordcraft_engine::proof::ProofLang::from_tag(Some(tag)) {
            wordcraft_engine::proof::ProofLang::Pl => "Polish".into(),
            wordcraft_engine::proof::ProofLang::Hr => "Croatian".into(),
            wordcraft_engine::proof::ProofLang::Pt => "Portuguese (Brazil)".into(),
            wordcraft_engine::proof::ProofLang::It => "Italian".into(),
            wordcraft_engine::proof::ProofLang::Tr => "Turkish".into(),
            wordcraft_engine::proof::ProofLang::Ru => "Russian".into(),
            wordcraft_engine::proof::ProofLang::Cs => "Czech".into(),
            wordcraft_engine::proof::ProofLang::Id => "Indonesian".into(),
            wordcraft_engine::proof::ProofLang::Es => "Spanish (Spain)".into(),
            wordcraft_engine::proof::ProofLang::De => "German (Germany)".into(),
            wordcraft_engine::proof::ProofLang::En if tag.eq_ignore_ascii_case("en") => "English (United States)".into(),
            _ => tag.to_string(),
        },
    }
}

/// The English name of a region subtag the language list has no entry for (else the subtag).
fn region_name(code: &str) -> String {
    const REGIONS: [(&str, &str); 62] = [
        ("AE", "U.A.E."),
        ("AO", "Angola"),
        ("AR", "Argentina"),
        ("AT", "Austria"),
        ("AU", "Australia"),
        ("BA", "Bosnia and Herzegovina"),
        ("BE", "Belgium"),
        ("BO", "Bolivia"),
        ("BR", "Brazil"),
        ("BY", "Belarus"),
        ("CA", "Canada"),
        ("CH", "Switzerland"),
        ("CL", "Chile"),
        ("CN", "China"),
        ("CO", "Colombia"),
        ("CR", "Costa Rica"),
        ("CV", "Cabo Verde"),
        ("CY", "Cyprus"),
        ("DE", "Germany"),
        ("DO", "Dominican Republic"),
        ("EC", "Ecuador"),
        ("EG", "Egypt"),
        ("ES", "Spain"),
        ("FR", "France"),
        ("GB", "United Kingdom"),
        ("GT", "Guatemala"),
        ("HK", "Hong Kong SAR"),
        ("IE", "Ireland"),
        ("IN", "India"),
        ("IQ", "Iraq"),
        ("IT", "Italy"),
        ("JM", "Jamaica"),
        ("JO", "Jordan"),
        ("KW", "Kuwait"),
        ("KZ", "Kazakhstan"),
        ("LB", "Lebanon"),
        ("LI", "Liechtenstein"),
        ("LU", "Luxembourg"),
        ("MA", "Morocco"),
        ("MC", "Monaco"),
        ("ME", "Montenegro"),
        ("MO", "Macao SAR"),
        ("MX", "Mexico"),
        ("MY", "Malaysia"),
        ("MZ", "Mozambique"),
        ("NL", "Netherlands"),
        ("NZ", "New Zealand"),
        ("PE", "Peru"),
        ("PH", "Philippines"),
        ("PR", "Puerto Rico"),
        ("PT", "Portugal"),
        ("PY", "Paraguay"),
        ("QA", "Qatar"),
        ("RU", "Russia"),
        ("SG", "Singapore"),
        ("SY", "Syria"),
        ("TW", "Taiwan"),
        ("UA", "Ukraine"),
        ("US", "United States"),
        ("UY", "Uruguay"),
        ("VE", "Venezuela"),
        ("ZA", "South Africa"),
    ];
    REGIONS.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map_or_else(|| code.to_ascii_uppercase(), |(_, n)| (*n).to_string())
}

/// The language at the caret (resolved through the styles), translated, for the status bar.
pub(crate) fn caret_language(app: &WordApp) -> String {
    tl!(&language_name(caret_lang_tag(app).as_deref())).to_string()
}

/// The formatting at the caret, resolved through the styles and the document default.
fn caret_char(app: &WordApp) -> wordcraft_doc::resolve::ResolvedChar {
    let s = &app.session;
    let style = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.style.clone());
    s.doc.styles.resolve_char(style.as_deref(), &s.typing_props())
}

/// The language tag at the caret, resolved through the styles and the document default.
fn caret_lang_tag(app: &WordApp) -> Option<String> {
    caret_char(app).lang
}

/// Is the text at the caret itself marked Do not check spelling or grammar? (`format.state`
/// looks at the whole word around a caret, so a partly marked word would read as checked.)
pub(crate) fn caret_no_proof(app: &WordApp) -> bool {
    caret_char(app).no_proof
}

/// Does WordCraft check spelling and grammar in the caret's language at all (Bulgarian: no)?
pub(crate) fn caret_language_proofed(app: &WordApp) -> bool {
    wordcraft_engine::proof::has_dictionary(caret_lang_tag(app).as_deref())
}

/// The Hunspell dictionary the caret's language needs but doesn't have (`pl_PL`), if any (#1167).
pub(crate) fn caret_missing_dictionary(app: &WordApp) -> Option<&'static str> {
    wordcraft_engine::proof::missing_dictionary(caret_lang_tag(app).as_deref())
}

/// The dictionary file and reason when the caret's language has a dictionary installed that
/// couldn't be read, so "no dictionary" would send the user to reinstall it for nothing (#2830).
pub(crate) fn caret_unreadable_dictionary(app: &WordApp) -> Option<(std::path::PathBuf, String)> {
    wordcraft_engine::proof::unreadable_dictionary(caret_lang_tag(app).as_deref()).map(|(_, file, reason)| (file, reason))
}

/// Review › Language (and the status bar's language): set the selection's proofing language, or
/// leave it unchecked. The current choice has a check mark; a language outside the short list
/// (French, German…) shows checked above it. Set Proofing Language… opens the Language dialog
/// with every language and Set As Default, and Language Preferences… opens Options › Language
/// (#860: the short list alone left no way to pick French).
pub(crate) fn language_menu(ui: &mut Ui, app: &mut WordApp) {
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    // The selection's own language; else (nothing set on the text) the one it gets from its
    // style or the document default, as the status bar shows.
    let current = match st.get("lang") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) if app.session.sel.is_collapsed() => caret_lang_tag(app).unwrap_or_else(|| "en-US".into()),
        _ => String::new(),
    };
    let no_proof = st.get("noProof").and_then(Value::as_bool).unwrap_or(false);
    let listed = |tag: &str| {
        current.eq_ignore_ascii_case(tag)
            || (tag == "pl-PL" && current.eq_ignore_ascii_case("pl"))
            || (tag == "pt-BR" && current.eq_ignore_ascii_case("pt"))
            || (tag == "it-IT" && current.eq_ignore_ascii_case("it"))
            || (tag == "hr-HR" && current.eq_ignore_ascii_case("hr"))
            || (tag == "tr-TR" && current.eq_ignore_ascii_case("tr"))
            || (tag == "cs-CZ" && wordcraft_engine::proof::ProofLang::from_tag(Some(&current)) == wordcraft_engine::proof::ProofLang::Cs)
            || (tag == "id-ID" && wordcraft_engine::proof::ProofLang::from_tag(Some(&current)) == wordcraft_engine::proof::ProofLang::Id)
            || (tag == "es-ES" && current.eq_ignore_ascii_case("es"))
            || (tag == "en-US" && current.eq_ignore_ascii_case("en"))
    };
    let checks = language_checks(&current, listed);
    if let Some(name) = checks.extra {
        mi_check(ui, app, &name, true, "review.language", json!({"lang": current}));
    }
    for ((tag, name), on) in PROOFING_LANGUAGES.iter().zip(checks.listed) {
        mi_check(ui, app, name, on, "review.language", json!({"lang": tag}));
    }
    ui.separator();
    mi_check(ui, app, "Do Not Check Spelling or Grammar", no_proof, "review.language", json!({"noProof": !no_proof}));
    ui.separator();
    mi_check(ui, app, "Set Proofing Language…", false, "review.language", json!({}));
    mi_check(ui, app, "Language Preferences…", false, "file.options", json!({"pane": "language"}));
}

/// The Language menu's check marks for the `current` tag: which short-list entries are checked
/// (`listed` matches a tag), and the name of the current language when it is none of them,
/// shown checked above. A language named like a short-list entry checks that entry, so no name
/// shows twice.
pub(super) struct LanguageChecks {
    pub(super) extra: Option<String>,
    pub(super) listed: [bool; PROOFING_LANGUAGES.len()],
}

pub(super) fn language_checks(current: &str, listed: impl Fn(&str) -> bool) -> LanguageChecks {
    let name = (!current.is_empty()).then(|| language_name(Some(current)));
    let on = PROOFING_LANGUAGES.map(|(t, n)| listed(t) || name.as_deref() == Some(n));
    LanguageChecks { extra: name.filter(|_| !on.contains(&true)), listed: on }
}

/// A menu item with a check mark when `checked` (the current choice of several).
pub(super) fn mi_check(ui: &mut Ui, app: &mut WordApp, label: &str, checked: bool, id: &str, params: Value) {
    // A vector mark needs no font glyph; the empty atom keeps labels aligned.
    let mark = egui::Atom::paint(vec2(14.0, 14.0), move |ui, args| {
        if checked {
            let r = args.rect;
            let points = vec![r.min + r.size() * vec2(0.15, 0.5), r.min + r.size() * vec2(0.4, 0.75), r.min + r.size() * vec2(0.85, 0.2)];
            ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.5, args.fallback_text_color)));
        }
    });
    let enabled = crate::widgets::enabled(app, id);
    let resp = ui.add_enabled(enabled, egui::Button::new((mark, tl!(label))).selected(checked).min_size(vec2(200.0, 0.0)));
    if resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
}
