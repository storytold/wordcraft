//! The run's language reaches the shaper (#5068): text is drawn with the font's forms for its
//! language (OpenType `locl`), as Word draws it. The bundled Source Serif 4 has Serbian and
//! Macedonian italic б, г, д, п, т, so the tests don't depend on the installed fonts.

use crate::*;
use wordcraft_doc::{Pos, StoryRef};

const FONT: &str = "Source Serif 4";

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn para_layout(l: &DocLayout) -> &ParaLayout {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                return para;
            }
        }
    }
    panic!("no paragraph laid out");
}

/// `text` in Source Serif 4 Italic, then each `(byte range, language)` set in turn.
fn doc(text: &str, langs: &[(std::ops::Range<usize>, Option<&str>)]) -> Document {
    let mut d = Document::from_text(text);
    d.format_range(&Pos::body(0, 0), &Pos::body(0, text.len()), &|c| {
        c.font = Some(FONT.into());
        c.italic = Some(true);
    })
    .unwrap();
    for (r, lang) in langs {
        d.format_range(&Pos::body(0, r.start), &Pos::body(0, r.end), &|c| c.lang = lang.map(str::to_string)).unwrap();
    }
    d
}

/// Each cluster's first character and glyph ids.
fn glyphs(text: &str, langs: &[(std::ops::Range<usize>, Option<&str>)]) -> Vec<(char, Vec<u32>)> {
    let l = lay(&doc(text, langs));
    let pl = para_layout(&l);
    pl.clusters
        .iter()
        .filter_map(|c| {
            let ch = text.get(c.start..)?.chars().next()?;
            let g = pl.glyphs.get(c.g0 as usize..c.g1 as usize)?;
            Some((ch, g.iter().map(|g| g.gid).collect()))
        })
        .collect()
}

/// Serbian text gets the Serbian italic д, т, п, г, б; Russian text and text without a language
/// get the default ones. The other letters, the space and the mark are the same.
#[test]
fn serbian_text_gets_the_serbian_forms() {
    let text = "Одштампај грб!";
    let all = 0..text.len();
    let none = glyphs(text, &[]);
    let russian = glyphs(text, &[(all.clone(), Some("ru-RU"))]);
    let serbian = glyphs(text, &[(all, Some("sr-Cyrl-RS"))]);
    assert_eq!(none.len(), text.chars().count(), "{none:?}");
    assert_eq!(russian, none);
    let changed: String = serbian.iter().zip(&none).filter(|(s, n)| s.1 != n.1).map(|(s, _)| s.0).collect();
    assert_eq!(changed, "дтпгб", "{serbian:?} {none:?}");
}

/// Each run is shaped in its own language: the same word in a Serbian run, then a Russian one.
#[test]
fn each_run_keeps_its_language() {
    let word = "грб";
    let n = word.len();
    let serbian = glyphs(word, &[(0..n, Some("sr-Cyrl-RS"))]);
    let russian = glyphs(word, &[(0..n, Some("ru-RU"))]);
    assert_ne!(serbian, russian);
    let both = glyphs("грб грб", &[(0..n, Some("sr-Cyrl-RS")), (n + 1..2 * n + 1, Some("ru-RU"))]);
    assert_eq!(both.get(..3), serbian.get(..3), "{both:?}");
    assert_eq!(both.get(4..7), russian.get(..3), "{both:?}");
}
