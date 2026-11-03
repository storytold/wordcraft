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

/// Every language WordCraft has an English name for, by tag: the Language menu's short list, the
/// Language dialog's built-in list, then the Language pack languages neither has (Bulgarian…).
fn known() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    PROOFING_LANGUAGES.iter().chain(crate::dialogs_layout::LANGUAGES.iter()).chain(crate::language_packs::MORE_LANGUAGES.iter())
}

/// The language tag a document stores for text in a Language pack's language: the tag WordCraft
/// already names for it (`nl` → `nl-NL`, `sr-latn` → `sr-Latn-RS`, `bg` → `bg-BG`), else the
/// locale in BCP 47 case (`de-at` → `de-AT`). [`wordcraft_engine::packs::pack_for_tag`] finds the
/// same pack again from it, so text marked in a pack's language is proofed by that pack.
pub(crate) fn tag_for_pack(locale: &str) -> String {
    let exact = known().find(|(t, _)| t.eq_ignore_ascii_case(locale));
    let prefixed = || {
        known().find(|(t, _)| {
            t.len() > locale.len()
                && t.get(..locale.len()).is_some_and(|p| p.eq_ignore_ascii_case(locale))
                && t.as_bytes().get(locale.len()) == Some(&b'-')
        })
    };
    // A variant WordCraft names with its region (`ca-valencia` is `ca-ES-valencia`).
    let regioned =
        || known().find(|(t, _)| t.contains('-') && without_region(&t.to_ascii_lowercase()) == locale.to_ascii_lowercase().replace('_', "-"));
    if let Some((t, _)) = exact.or_else(prefixed).or_else(regioned) {
        return (*t).to_string();
    }
    let parts: Vec<String> = locale
        .split(['-', '_'])
        .enumerate()
        .map(|(i, p)| {
            let alpha = p.chars().all(|c| c.is_ascii_alphabetic());
            if i > 0 && alpha && p.len() == 2 {
                p.to_ascii_uppercase()
            } else if i > 0 && alpha && p.len() == 4 {
                let lower = p.to_ascii_lowercase();
                let mut c = lower.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect();
    parts.join("-")
}

/// A pack language's name for the lists, in English: the name WordCraft has for its tag, else the
/// pack's own name (as shown, without format characters), else the tag.
fn pack_language_name(tag: &str, endonym: &str) -> String {
    let named = language_name(Some(tag));
    if !named.eq_ignore_ascii_case(tag) || endonym.trim().is_empty() {
        return named;
    }
    crate::language_packs::shown(endonym, 40)
}

/// The installed and available Language packs, as (document tag, pack's own name), installed first.
fn pack_languages(available: bool) -> Vec<(String, String)> {
    let installed = wordcraft_engine::packs::installed().into_iter().map(|i| (i.locale, i.name));
    let index: Vec<(String, String)> =
        if available { wordcraft_engine::packs::available().into_iter().map(|p| (p.locale, p.name)).collect() } else { Vec::new() };
    installed.chain(index).map(|(locale, name)| (tag_for_pack(&locale), name)).collect()
}

/// Review › Language's and the status bar's short list, as (tag, English name): the languages
/// WordCraft proofs itself, then each installed Language pack's language not among them, by name.
pub(crate) fn short_languages() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = PROOFING_LANGUAGES.iter().map(|(t, n)| ((*t).to_string(), (*n).to_string())).collect();
    let mut more: Vec<(String, String)> = Vec::new();
    for (tag, endonym) in pack_languages(false) {
        if !out.iter().chain(&more).any(|(t, _)| t.eq_ignore_ascii_case(&tag)) {
            let name = pack_language_name(&tag, &endonym);
            more.push((tag, name));
        }
    }
    more.sort_by_cached_key(|(_, n)| sort_key(tl!(n)));
    out.extend(more);
    out
}

/// The Set Proofing Language dialog's list, as (tag, name in the interface language), sorted by
/// name: every language WordCraft names, every installed Language pack's and every available
/// one's, each once.
pub(crate) fn all_languages() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let builtin = PROOFING_LANGUAGES.iter().chain(crate::dialogs_layout::LANGUAGES.iter()).map(|(t, n)| ((*t).to_string(), (*n).to_string()));
    let packs = pack_languages(true).into_iter().map(|(tag, endonym)| {
        let name = pack_language_name(&tag, &endonym);
        (tag, name)
    });
    for (tag, name) in builtin.chain(packs) {
        if !out.iter().any(|(t, _)| t.eq_ignore_ascii_case(&tag)) {
            out.push((tag, tl!(&name).to_string()));
        }
    }
    out.sort_by_cached_key(|(_, n)| sort_key(n));
    out
}

/// The Set Proofing Language dialog's list: [`all_languages`], with the `current` language when
/// it is none of them ([`same_language`]: a row is checked for it then).
pub(crate) fn language_list(current: &str) -> Vec<(String, String)> {
    let mut list = all_languages();
    // A tag the list has under another spelling (`nl` for `nl-NL`) is that row, not another.
    if !current.is_empty() && !list.iter().any(|(t, _)| same_language(current, t)) {
        list.push((current.to_string(), tl!(&language_name(Some(current))).to_string()));
        list.sort_by_cached_key(|(_, n)| sort_key(n));
    }
    list
}

/// How a language name sorts: case-insensitively, a Latin letter with a mark beside the plain one
/// (Čeština with Czech, Ελληνικά and other scripts after Latin, by code point).
pub(crate) fn sort_key(name: &str) -> (String, String) {
    const FOLD: [(&str, &str); 22] = [
        ("àáâãäåāăą", "a"),
        ("æ", "ae"),
        ("çćĉċč", "c"),
        ("ďđð", "d"),
        ("èéêëēĕėęě", "e"),
        ("ĝğġģ", "g"),
        ("ĥħ", "h"),
        ("ìíîïĩīĭįı", "i"),
        ("ĵ", "j"),
        ("ķ", "k"),
        ("ĺļľŀł", "l"),
        ("ñńņňŉ", "n"),
        ("òóôõöøōŏő", "o"),
        ("œ", "oe"),
        ("ŕŗř", "r"),
        ("śŝşšș", "s"),
        ("ß", "ss"),
        ("ţťŧțþ", "t"),
        ("ùúûüũūŭůűų", "u"),
        ("ŵ", "w"),
        ("ýÿŷ", "y"),
        ("źżž", "z"),
    ];
    let lower = name.to_lowercase();
    let mut folded = String::with_capacity(lower.len());
    for c in lower.chars() {
        match FOLD.iter().find(|(from, _)| from.contains(c)) {
            Some((_, to)) => folded.push_str(to),
            None => folded.push(c),
        }
    }
    (folded, lower)
}

/// The English name of a proofing language tag, as the status bar shows it (no tag: WordCraft's
/// default, US English): a listed language by its name; another region of one by the language
/// and the region ("French (Canada)"); a bare language subtag by its usual variant; else the tag.
pub(crate) fn language_name(tag: Option<&str>) -> String {
    let tag = tag.unwrap_or("en-US");
    let named = |t: &str| known().find(|(l, _)| l.eq_ignore_ascii_case(t)).map(|(_, n)| *n);
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
            let language = known()
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
            // A language (and script) without its region: the named tag it starts (`bg` is
            // Bulgarian, `sr-Latn` Serbian (Latin)).
            _ => known()
                .find(|(t, _)| {
                    t.len() > tag.len()
                        && t.get(..tag.len()).is_some_and(|p| p.eq_ignore_ascii_case(tag))
                        && t.as_bytes().get(tag.len()) == Some(&b'-')
                })
                .or_else(|| known().find(|(t, _)| t.contains('-') && without_region(&t.to_ascii_lowercase()).eq_ignore_ascii_case(tag)))
                .map_or_else(|| tag.to_string(), |(_, n)| (*n).to_string()),
        },
    }
}

/// The English name of a region subtag the language list has no entry for (else the subtag).
fn region_name(code: &str) -> String {
    const REGIONS: [(&str, &str); 69] = [
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
        ("CU", "Cuba"),
        ("CV", "Cabo Verde"),
        ("CY", "Cyprus"),
        ("DE", "Germany"),
        ("DO", "Dominican Republic"),
        ("EC", "Ecuador"),
        ("EG", "Egypt"),
        ("ES", "Spain"),
        ("FI", "Finland"),
        ("FR", "France"),
        ("GB", "United Kingdom"),
        ("GQ", "Equatorial Guinea"),
        ("GT", "Guatemala"),
        ("HK", "Hong Kong SAR"),
        ("HN", "Honduras"),
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
        ("NI", "Nicaragua"),
        ("NL", "Netherlands"),
        ("NZ", "New Zealand"),
        ("PA", "Panama"),
        ("PE", "Peru"),
        ("PH", "Philippines"),
        ("PR", "Puerto Rico"),
        ("PT", "Portugal"),
        ("PY", "Paraguay"),
        ("QA", "Qatar"),
        ("RU", "Russia"),
        ("SG", "Singapore"),
        ("SV", "El Salvador"),
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
    wordcraft_engine::packs::missing_dictionary(caret_lang_tag(app).as_deref())
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
    let entries = short_languages();
    let checks = language_checks(&current, &entries);
    if let Some(name) = checks.extra {
        mi_check(ui, app, &name, true, "review.language", json!({"lang": current}));
    }
    for ((tag, name), on) in entries.iter().zip(checks.listed) {
        mi_check(ui, app, name, on, "review.language", json!({"lang": tag}));
    }
    ui.separator();
    mi_check(ui, app, "Do Not Check Spelling or Grammar", no_proof, "review.language", json!({"noProof": !no_proof}));
    ui.separator();
    mi_check(ui, app, "Set Proofing Language…", false, "review.language", json!({}));
    mi_check(ui, app, "Language Preferences…", false, "file.options", json!({"pane": "language"}));
}

/// The Language menu's check marks for the `current` tag: which `entries` (tag, English name) are
/// checked, and the name of the current language when it is none of them, shown checked above.
/// An entry is checked for its own tag (any case), or for a tag WordCraft proofs the same way and
/// names alike (`pl` for Polish, `ces` for Czech, `PT-br-x-test` for Portuguese (Brazil)), so no
/// name shows twice; English (New Zealand) doesn't check English (United Kingdom).
pub(super) struct LanguageChecks {
    pub(super) extra: Option<String>,
    pub(super) listed: Vec<bool>,
}

pub(super) fn language_checks(current: &str, entries: &[(String, String)]) -> LanguageChecks {
    let name = (!current.is_empty()).then(|| language_name(Some(current)));
    let on: Vec<bool> =
        entries.iter().map(|(t, n)| t.eq_ignore_ascii_case(current) || (name.as_deref() == Some(n.as_str()) && same_proofing(current, t))).collect();
    LanguageChecks { extra: name.filter(|_| !on.contains(&true)), listed: on }
}

/// Are `a` and `b` the same language as the lists show it: the same tag (any case), or named
/// alike and proofed alike ([`same_proofing`]): `nl` is the `nl-NL` row, `pl` the `pl-PL` one.
pub(crate) fn same_language(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (!a.is_empty() && language_name(Some(a)) == language_name(Some(b)) && same_proofing(a, b))
}

/// Does WordCraft proof `a` and `b` the same way: the same built-in language, or two Language
/// pack languages read from the same pack ([`wordcraft_engine::packs::pack_for_tag`]: `nl` and
/// `nl-NL` are both the `nl` pack's, though their pack locales differ); with neither pack known,
/// the same language and script whatever the region.
fn same_proofing(a: &str, b: &str) -> bool {
    use wordcraft_engine::packs;
    use wordcraft_engine::proof::ProofLang;
    let (la, lb) = (ProofLang::from_tag(Some(a)), ProofLang::from_tag(Some(b)));
    if la == lb {
        return true;
    }
    let (Some(pa), Some(pb)) = (la.pack_locale(), lb.pack_locale()) else { return false };
    let mut known = packs::available();
    for i in packs::installed() {
        if !known.iter().any(|p| p.locale == i.locale) {
            known.push(packs::Pack {
                locale: i.locale,
                name: i.name,
                version: String::new(),
                url: String::new(),
                source: None,
                size: 0,
                licence: i.licence,
                class: String::new(),
                spelling: false,
                hyphenation: false,
            });
        }
    }
    match (packs::pack_for_tag(a, &known), packs::pack_for_tag(b, &known)) {
        (Some(x), Some(y)) => x.locale == y.locale,
        (None, None) => without_region(pa.as_str()) == without_region(pb.as_str()),
        _ => false,
    }
}

/// A lowercase locale without its region subtag (`nl-nl` → `nl`, `sr-latn-rs` → `sr-latn`).
fn without_region(locale: &str) -> String {
    locale
        .split('-')
        .enumerate()
        .filter(|(i, p)| {
            *i == 0 || !((p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic())) || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit())))
        })
        .map(|(_, p)| p)
        .collect::<Vec<&str>>()
        .join("-")
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
