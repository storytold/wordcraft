//! A bullet in a missing font that the font table classes as roman, without an alternate name.
//! Its own test binary: it registers a document font named `Cambria` (the roman fallback) for the
//! whole process.

use wordcraft_doc::Document;
use wordcraft_doc::Pos;
use wordcraft_layout::{LayoutCache, LayoutOptions, Placed, layout};

/// Only a font table alternate changes which font a bullet's private-use code is checked in:
/// the roman fallback (Cambria) doesn't, so the bullet is the level's default one, as before.
/// Cambria here is JetBrains Mono, which has U+E0A0.
#[test]
fn a_private_use_bullet_in_a_missing_roman_font_without_an_alternate_shows_the_default_bullet() {
    let mono = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/JetBrainsMono-Regular.ttf")).unwrap();
    wordcraft_fonts::FontDb::global().add_document_fonts(vec![(mono, "Cambria".to_string())]);
    let cambria = wordcraft_fonts::word::resolve("Cambria", false, false);
    assert!(!cambria.substituted && cambria.face.covers('\u{E0A0}'), "Cambria is the registered JetBrains Mono");

    const MISSING: &str = "No Such Roman Bullet Font";
    let label = |alt: Option<&str>| {
        let mut d = Document::from_text("one");
        d.settings.roman_fonts = vec![MISSING.into()];
        d.settings.font_alt_names = alt.map(|a| vec![(MISSING.to_string(), a.to_string())]).unwrap_or_default();
        let id = d.numbering.add_list(wordcraft_doc::ListKind::Bullet);
        for a in &mut d.numbering.abstracts {
            if let Some(l) = a.levels.get_mut(0) {
                l.text = "\u{E0A0}".into();
                l.chr.font = Some(MISSING.into());
            }
        }
        d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })).unwrap();
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let label = l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { para.label.clone() } else { None });
        label.expect("a label").text
    };
    assert_eq!(label(None), "\u{2022}");
    assert_eq!(label(Some("Also Not Installed")), "\u{2022}");
    assert_eq!(label(Some("JetBrains Mono")), "\u{E0A0}");
}
