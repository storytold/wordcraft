//! A run's languages through RTF, ODT and HTML: mixed runs, document defaults, hostile tags and
//! language IDs, and unchanged output for documents without languages.

use wordcraft_doc::{CharProps, Document, Paragraph, para_block};

use crate::lang::Slot;
use crate::{export, import};

fn cp(lang: Option<&str>, bidi: Option<&str>, ea: Option<&str>) -> CharProps {
    CharProps { lang: lang.map(Into::into), lang_bidi: bidi.map(Into::into), lang_ea: ea.map(Into::into), ..Default::default() }
}

/// One paragraph of runs in US English, Nepali, Serbian Cyrillic, Arabic (complex script) and
/// Japanese (East Asian), in a new document (US English by default).
fn mixed() -> Document {
    let mut d = Document::new();
    let mut p = Paragraph::with_text("Hello ", cp(Some("en-US"), None, None));
    for (text, props) in [
        ("नमस्ते ", cp(Some("ne-NP"), None, None)),
        ("Здраво ", cp(Some("sr-Cyrl-RS"), None, None)),
        ("مرحبا ", cp(None, Some("ar-SA"), None)),
        ("日本語", cp(None, None, Some("ja-JP"))),
    ] {
        let n = p.len();
        p.insert_text(n, text, &props).unwrap();
    }
    d.body = vec![para_block(p)];
    d
}

/// The language of slot `slot` of the text holding `needle`: its run's, else the document's.
fn lang_of(doc: &Document, needle: &str, slot: Slot) -> Option<String> {
    let d = &doc.styles.default_chr;
    for p in doc.body.iter().filter_map(|b| b.as_para()) {
        let mut at = 0;
        for r in &p.runs {
            let text = p.text.get(at..at + r.len).unwrap_or("");
            at += r.len;
            if text.contains(needle) {
                let (own, def) = match slot {
                    Slot::Latin => (&r.props.lang, &d.lang),
                    Slot::Bidi => (&r.props.lang_bidi, &d.lang_bidi),
                    Slot::Ea => (&r.props.lang_ea, &d.lang_ea),
                };
                return own.clone().or_else(|| def.clone());
            }
        }
    }
    None
}

fn round_trip(ext: &str, doc: &Document) -> Document {
    let bytes = export(ext, doc).unwrap().unwrap();
    import(ext, &bytes).unwrap().unwrap()
}

/// Each text's language in the slot its script uses.
const EXPECTED: [(&str, Slot, &str); 5] = [
    ("Hello", Slot::Latin, "en-US"),
    ("नमस्ते", Slot::Latin, "ne-NP"),
    ("Здраво", Slot::Latin, "sr-Cyrl-RS"),
    ("مرحبا", Slot::Bidi, "ar-SA"),
    ("日本語", Slot::Ea, "ja-JP"),
];

#[test]
fn mixed_run_languages_survive_rtf_odt_and_html() {
    let doc = mixed();
    for ext in ["rtf", "odt", "html"] {
        let back = round_trip(ext, &doc);
        for (needle, slot, want) in EXPECTED {
            assert_eq!(lang_of(&back, needle, slot).as_deref(), Some(want), "{ext}: {needle}");
        }
    }
}

#[test]
fn rtf_and_odt_keep_all_three_languages_of_a_run() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("x", cp(Some("ne-NP"), Some("ar-SA"), Some("ja-JP"))))];
    for ext in ["rtf", "odt"] {
        let back = round_trip(ext, &d);
        assert_eq!(lang_of(&back, "x", Slot::Latin).as_deref(), Some("ne-NP"), "{ext}");
        assert_eq!(lang_of(&back, "x", Slot::Bidi).as_deref(), Some("ar-SA"), "{ext}");
        assert_eq!(lang_of(&back, "x", Slot::Ea).as_deref(), Some("ja-JP"), "{ext}");
    }
}

#[test]
fn what_each_format_writes() {
    let doc = mixed();
    let rtf = String::from_utf8(export("rtf", &doc).unwrap().unwrap()).unwrap();
    // US English is the document's: said once, in `\deflang`, not on its run.
    assert!(rtf.contains("\\deflang1033"), "{rtf}");
    assert_eq!(rtf.matches("\\lang1033").count(), 0, "{rtf}");
    for word in ["\\lang1121", "\\lang10266", "\\alang1025", "\\langfe1041"] {
        assert!(rtf.contains(word), "{word} in {rtf}");
    }
    let html = String::from_utf8(export("html", &doc).unwrap().unwrap()).unwrap();
    assert!(html.contains("<html lang=\"en-US\">"), "{html}");
    for attr in ["<span lang=\"ne-NP\">", "<span lang=\"sr-Cyrl-RS\">", "<span lang=\"ar-SA\">", "<span lang=\"ja-JP\">"] {
        assert!(html.contains(attr), "{attr} in {html}");
    }
    assert!(!html.contains("<span lang=\"en-US\">"), "{html}");
    let odt = export("odt", &doc).unwrap().unwrap();
    let content = odt_entry(&odt, "content.xml");
    for attr in [
        "fo:language=\"ne\" fo:country=\"NP\"",
        "fo:language=\"sr\" fo:script=\"Cyrl\" fo:country=\"RS\"",
        "style:language-complex=\"ar\" style:country-complex=\"SA\"",
        "style:language-asian=\"ja\" style:country-asian=\"JP\"",
    ] {
        assert!(content.contains(attr), "{attr} in {content}");
    }
    let styles = odt_entry(&odt, "styles.xml");
    assert!(styles.contains("fo:language=\"en\" fo:country=\"US\"/></style:default-style>"), "{styles}");
}

fn odt_entry(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name(name).unwrap(), &mut s).unwrap();
    s
}

#[test]
fn document_languages_survive() {
    let mut d = Document::new();
    d.styles.default_chr.lang = Some("ne-NP".into());
    d.styles.default_chr.lang_bidi = Some("ar-SA".into());
    d.styles.default_chr.lang_ea = Some("ja-JP".into());
    d.body = vec![para_block(Paragraph::with_text("नमस्ते مرحبا", CharProps::default()))];
    for ext in ["rtf", "odt"] {
        let back = round_trip(ext, &d);
        let c = &back.styles.default_chr;
        assert_eq!((c.lang.as_deref(), c.lang_bidi.as_deref(), c.lang_ea.as_deref()), (Some("ne-NP"), Some("ar-SA"), Some("ja-JP")), "{ext}");
    }
    // HTML names one language per element: the document's is the root's.
    let back = round_trip("html", &d);
    assert_eq!(back.styles.default_chr.lang.as_deref(), Some("ne-NP"));
}

#[test]
fn a_tag_rtf_cannot_name_keeps_the_others() {
    // Klingon has no Windows language ID: RTF can't say it, so the text has the document's.
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("x", cp(Some("tlh"), None, Some("ja-JP"))))];
    let back = round_trip("rtf", &d);
    assert_eq!(lang_of(&back, "x", Slot::Latin).as_deref(), Some("en-US"));
    assert_eq!(lang_of(&back, "x", Slot::Ea).as_deref(), Some("ja-JP"));
    // ODT and HTML say any tag.
    for ext in ["odt", "html"] {
        let back = round_trip(ext, &d);
        assert_eq!(lang_of(&back, "x", Slot::Latin).as_deref(), Some("tlh"), "{ext}");
    }
    // ODT keeps a tag its attributes can't split whole.
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("x", cp(Some("es-419"), None, None)))];
    assert_eq!(lang_of(&round_trip("odt", &d), "x", Slot::Latin).as_deref(), Some("es-419"));
}

#[test]
fn hostile_languages_are_ignored() {
    let rtf = format!(
        "{{\\rtf1\\deflang99999999\\deflangfe-7\\adeflang2147483647 {{\\lang-5 a}}{{\\lang2147483647 b}}{{\\langfe1024 c}}{{\\alang65535 d}}{{\\lang0 e}}{{\\lang{} f}}\\par}}",
        "9".repeat(400)
    );
    let doc = import("rtf", rtf.as_bytes()).unwrap().unwrap();
    for needle in ["a", "b", "c", "d", "e", "f"] {
        assert_eq!(lang_of(&doc, needle, Slot::Latin).as_deref(), Some("en-US"), "{needle}");
        assert_eq!(lang_of(&doc, needle, Slot::Bidi), None, "{needle}");
        assert_eq!(lang_of(&doc, needle, Slot::Ea), None, "{needle}");
    }
    let long = "a".repeat(100_000);
    let html = format!(
        "<html lang=\"{long}\"><body><p><span lang=\"&lt;script&gt;\">a</span><span lang=\"en--US\">b</span><span xml:lang=\"x\" lang=\"de\">c</span><span lang=\"de-DE-{long}\">d</span></p></body></html>"
    );
    let doc = import("html", html.as_bytes()).unwrap().unwrap();
    for needle in ["a", "b", "c", "d"] {
        assert_eq!(lang_of(&doc, needle, Slot::Latin).as_deref(), Some("en-US"), "{needle}");
    }
    // A hand-made document may hold anything: an exporter writes only well-formed tags.
    let mut d = Document::new();
    d.styles.default_chr.lang = Some("\"><script>".into());
    d.body = vec![para_block(Paragraph::with_text("x", cp(Some(&long), Some("<b>"), Some("ja JP"))))];
    for ext in ["rtf", "odt", "html"] {
        let bytes = export(ext, &d).unwrap().unwrap();
        let back = import(ext, &bytes).unwrap().unwrap();
        assert_eq!(lang_of(&back, "x", Slot::Ea), None, "{ext}");
        if ext != "odt" {
            let s = String::from_utf8(bytes).unwrap();
            assert!(!s.contains("<script>") && !s.contains("<b>") && !s.contains(&long), "{ext}");
        }
    }
}

#[test]
fn xhtml_language_inheritance() {
    let html = "<html xml:lang=\"de-DE\"><body><div lang=\"fr-FR\"><p>Bonjour <span lang=\"ja-JP\">日本</span> <span lang=\"\">inconnu</span></p></div><p>Hallo</p></body></html>";
    let doc = import("html", html.as_bytes()).unwrap().unwrap();
    assert_eq!(doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
    assert_eq!(lang_of(&doc, "Bonjour", Slot::Latin).as_deref(), Some("fr-FR"));
    // A Japanese span in French text: Japanese for its East Asian text, still French otherwise.
    assert_eq!(lang_of(&doc, "日本", Slot::Ea).as_deref(), Some("ja-JP"));
    assert_eq!(lang_of(&doc, "日本", Slot::Latin).as_deref(), Some("fr-FR"));
    // An empty `lang` is an unknown language: the document's.
    assert_eq!(lang_of(&doc, "inconnu", Slot::Latin).as_deref(), Some("de-DE"));
    assert_eq!(lang_of(&doc, "Hallo", Slot::Latin).as_deref(), Some("de-DE"));
}

#[test]
fn rtf_reads_word_style_language_words() {
    // As Word writes them: non-propagating copies, associated languages, `\plain`, and no
    // language (1024).
    let rtf = "{\\rtf1\\ansi\\deflang1033\\deflangfe1041\\adeflang1025 \\pard\\plain \\rtlch\\alang1037 \\ltrch\\lang1049\\langnp1049\\langfe2052\\langfenp2052 one\\plain two {\\lang1121 three {\\lang1024 four}}\\par}";
    let doc = import("rtf", rtf.as_bytes()).unwrap().unwrap();
    assert_eq!(lang_of(&doc, "one", Slot::Latin).as_deref(), Some("ru-RU"));
    assert_eq!(lang_of(&doc, "one", Slot::Bidi).as_deref(), Some("he-IL"));
    assert_eq!(lang_of(&doc, "one", Slot::Ea).as_deref(), Some("zh-CN"));
    assert_eq!(lang_of(&doc, "two", Slot::Latin).as_deref(), Some("en-US"));
    assert_eq!(lang_of(&doc, "two", Slot::Bidi).as_deref(), Some("ar-SA"));
    assert_eq!(lang_of(&doc, "two", Slot::Ea).as_deref(), Some("ja-JP"));
    assert_eq!(lang_of(&doc, "three", Slot::Latin).as_deref(), Some("ne-NP"));
    assert_eq!(lang_of(&doc, "four", Slot::Latin).as_deref(), Some("en-US"));
}

/// A document like [`mixed`] with no language anywhere, not even the defaults.
fn without_languages() -> Document {
    let mut d = Document::new();
    d.styles.default_chr.lang = None;
    d.body = vec![para_block(Paragraph::with_text("Hello नमस्ते Здраво مرحبا 日本語", CharProps::default()))];
    d
}

#[test]
fn documents_without_languages_are_written_as_before() {
    let d = without_languages();
    let rtf = String::from_utf8(export("rtf", &d).unwrap().unwrap()).unwrap();
    assert!(rtf.starts_with("{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1033\\uc1\n"), "{rtf}");
    for word in ["\\lang", "\\langfe", "\\alang", "\\deflangfe", "\\adeflang"] {
        assert!(!rtf.contains(word), "{word} in {rtf}");
    }
    let html = String::from_utf8(export("html", &d).unwrap().unwrap()).unwrap();
    assert!(html.contains("<html lang=\"en\">"), "{html}");
    assert_eq!(html.matches("lang=").count(), 1, "{html}");
    assert!(!html.contains("<span"), "{html}");
    let odt = export("odt", &d).unwrap().unwrap();
    for entry in ["content.xml", "styles.xml"] {
        let xml = odt_entry(&odt, entry);
        for attr in ["language", "country", "script", "rfc-language-tag"] {
            assert!(!xml.contains(attr), "{attr} in {entry}: {xml}");
        }
    }
    assert!(
        odt_entry(&odt, "styles.xml").contains(
            "<style:default-style style:family=\"paragraph\"><style:paragraph-properties fo:margin-bottom=\"8pt\"/><style:text-properties fo:font-family=\"'Aptos'\" fo:font-size=\"12pt\"/></style:default-style>"
        )
    );
}
