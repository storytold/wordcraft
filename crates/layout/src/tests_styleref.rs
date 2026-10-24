//! STYLEREF fields in headers and footers (#775): each page shows its own result.

use super::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, NumRef};
use wordcraft_doc::{ListKind, PartKind, Pos, para_block};

fn field(instr: &str) -> InlineObject {
    InlineObject::Field {
        instr: instr.into(),
        result: "stale".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
        code_xml: String::new(),
    }
}

/// Body paragraphs: `(style, text, page break before)`; a header with one STYLEREF per code.
fn doc(paras: &[(&str, &str, bool)], codes: &[&str]) -> Document {
    let mut d = Document::from_text(&paras.iter().map(|p| p.1).collect::<Vec<_>>().join("\n"));
    for (i, (style, _, brk)) in paras.iter().enumerate() {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.props.style = Some((*style).to_string());
        p.props.page_break_before = Some(*brk);
    }
    let blocks = codes
        .iter()
        .map(|c| {
            let mut p = Paragraph::new();
            p.insert_object(0, field(c), &CharProps::default()).unwrap();
            para_block(p)
        })
        .collect();
    let id = d.add_part(PartKind::Header, blocks);
    d.last_section.headers.default = Some(id);
    d
}

fn shown(items: &[Placed]) -> Vec<String> {
    items.iter().flat_map(|it| if let Placed::Lines { para, .. } = it { para.shown.iter().map(|s| s.1.clone()).collect() } else { vec![] }).collect()
}

fn headers(d: &Document) -> Vec<Vec<String>> {
    let mut c = LayoutCache::new();
    layout(d, &mut c, &LayoutOptions::default()).pages.iter().map(|p| shown(&p.header)).collect()
}

const CHAPTERS: &[(&str, &str, bool)] = &[
    ("Heading1", "Beginnings", false),
    ("Normal", "Text of one.", false),
    ("Heading1", "Middles", true),
    ("Normal", "Text of two.", false),
    // A page with no heading: the header keeps the chapter before it.
    ("Normal", "More of two.", true),
    ("Heading1", "Endings", true),
    ("Heading2", "A section", false),
];

#[test]
fn each_page_s_header_shows_its_chapter() {
    let d = doc(CHAPTERS, &[r#"STYLEREF "Heading 1" \* MERGEFORMAT"#, "STYLEREF 1", "STYLEREF heading1 \\l"]);
    let h = headers(&d);
    assert_eq!(h.len(), 4);
    for (page, want) in ["Beginnings", "Middles", "Middles", "Endings"].iter().enumerate() {
        assert_eq!(h[page], [*want, *want, *want], "page {}", page + 1);
    }
}

#[test]
fn search_from_the_bottom_with_l() {
    let paras = [("Heading2", "First", false), ("Heading2", "Second", false), ("Normal", "Body", false), ("Heading2", "Third", true)];
    let d = doc(&paras, &[r#"STYLEREF "Heading 2""#, r#"STYLEREF "Heading 2" \l"#]);
    assert_eq!(headers(&d), [vec!["First", "Second"], vec!["Third", "Third"]]);
}

#[test]
fn a_missing_style_or_no_paragraph_of_it_is_an_error() {
    let d = doc(CHAPTERS, &[r#"STYLEREF "No Such Style""#, r#"STYLEREF "Heading 3""#, "STYLEREF 12"]);
    let err = styleref::NO_TEXT;
    assert_eq!(headers(&d)[0], [err, err, err]);
    assert_eq!(err, "Error: no paragraph in this style.");
}

#[test]
fn before_the_first_heading_the_next_one_shows() {
    let paras = [("Normal", "Preface", false), ("Heading1", "Chapter", true)];
    let d = doc(&paras, &[r#"STYLEREF "Heading 1""#, r#"STYLEREF "Heading 1" \p"#]);
    assert_eq!(headers(&d), [vec!["Chapter", "below"], vec!["Chapter", "below"]]);
}

#[test]
fn character_styles_and_paragraph_numbers() {
    let mut d = doc(CHAPTERS, &[r#"STYLEREF Strong"#, r#"STYLEREF "Heading 1" \n"#, r#"STYLEREF "Heading 2" \w"#, r#"STYLEREF "Heading 1" \s"#]);
    // Bold a word on page 2 with the Strong character style.
    d.format_range(&Pos::body(3, 8), &Pos::body(3, 11), &|c: &mut CharProps| c.style = Some("Strong".into())).unwrap();
    // Number the headings 1., 2., 3. and the section under them 3.1 (full context).
    let num = d.numbering.add_list(ListKind::Numbered);
    for (i, lvl) in [(0, 0u8), (2, 0), (5, 0), (6, 1)] {
        d.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.numbering = Some(NumRef { num, level: lvl });
    }
    let h = headers(&d);
    assert_eq!(h[1], ["two", "2", "3.a", "2"]);
    assert_eq!(h[3], ["two", "3", "3.a", "3"]);
}

/// Editing a heading updates the headers that show it, through the paragraph cache.
#[test]
fn headers_follow_heading_edits() {
    let mut d = doc(CHAPTERS, &[r#"STYLEREF "Heading 1""#]);
    let mut c = LayoutCache::new();
    let first = layout(&d, &mut c, &LayoutOptions::default());
    assert_eq!(shown(&first.pages[1].header), ["Middles"]);
    let p = d.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    p.insert_text(0, "The ", &CharProps::default()).unwrap();
    let again = layout(&d, &mut c, &LayoutOptions::default());
    assert_eq!(shown(&again.pages[1].header), ["The Middles"]);
    assert_eq!(shown(&again.pages[0].header), ["Beginnings"]);
}

/// The style index is built once and reused while the body and the styles stay the same (long
/// documents lay out on every keystroke); an edited heading or a changed style rebuilds it.
#[test]
fn style_index_is_reused_until_the_body_or_styles_change() {
    let mut d = doc(CHAPTERS, &[r#"STYLEREF "Heading 1""#]);
    let mut c = LayoutCache::new();
    let index = |c: &LayoutCache| c.styleref.as_ref().map(|(_, _, ix)| ix.clone()).unwrap();
    let pages =
        |d: &Document, c: &mut LayoutCache| layout(d, c, &LayoutOptions::default()).pages.iter().map(|p| shown(&p.header)).collect::<Vec<_>>();
    let before = pages(&d, &mut c);
    let first = index(&c);
    assert_eq!(pages(&d, &mut c), before);
    assert!(Arc::ptr_eq(&first, &index(&c)), "nothing changed: the same index");
    // Rename the first chapter: rebuilt, and the header shows the new title.
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.text = "Openings".into();
    p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: Default::default() }];
    let after = pages(&d, &mut c);
    assert!(!Arc::ptr_eq(&first, &index(&c)), "an edited heading rebuilds it");
    assert!(after[0].iter().any(|t| t.contains("Openings")), "{after:?}");
    // A style change rebuilds it too.
    let second = index(&c);
    if let Some(st) = d.styles.styles.iter_mut().find(|s| s.id == "Heading1") {
        st.name = "Chapter Title".into();
    }
    let _ = pages(&d, &mut c);
    assert!(!Arc::ptr_eq(&second, &index(&c)), "a style change rebuilds it");
}

/// `STYLEREF 1` finds the style named `heading 1` when its id is localized (#1286).
#[test]
fn heading_level_matches_a_localized_style_id() {
    let paras: Vec<(&str, &str, bool)> = CHAPTERS.iter().map(|(s, t, b)| (if *s == "Heading1" { "berschrift1" } else { *s }, *t, *b)).collect();
    let mut d = doc(&paras, &["STYLEREF 1"]);
    let st = d.styles.styles.iter_mut().find(|s| s.id == "Heading1").unwrap();
    st.id = "berschrift1".into();
    st.name = "heading 1".into();
    let h = headers(&d);
    assert_eq!(h[0], ["Beginnings"]);
    assert_eq!(h[3], ["Endings"]);
}

/// Typing in body text that no STYLEREF shows keeps every header paragraph cached (#1286); an
/// edited heading still updates the headers that show it.
#[test]
fn body_edits_keep_styleref_headers_cached() {
    let mut d = doc(CHAPTERS, &[r#"STYLEREF "Heading 1""#, r#"STYLEREF "Heading 1" \p"#]);
    let mut c = LayoutCache::new();
    let opts = LayoutOptions::default();
    let _ = layout(&d, &mut c, &opts);
    // Type in a body paragraph: only that paragraph is laid out again.
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_text(0, "More ", &CharProps::default()).unwrap();
    let misses = c.misses;
    let again = layout(&d, &mut c, &opts);
    assert_eq!(c.misses, misses + 1, "only the edited body paragraph misses the cache");
    assert_eq!(shown(&again.pages[1].header), ["Middles", "below"]);
    // Edit a heading: the headers showing it change.
    d.para_mut(StoryRef::Body, &Path::top(2)).unwrap().insert_text(0, "The ", &CharProps::default()).unwrap();
    let edited = layout(&d, &mut c, &opts);
    assert_eq!(shown(&edited.pages[1].header), ["The Middles", "below"]);
    // Page 3 has no heading: the one before it shows, above the header.
    assert_eq!(shown(&edited.pages[2].header), ["The Middles", "above"]);
    assert_eq!(shown(&edited.pages[0].header), ["Beginnings", "below"]);
}

/// The x of each shown character's first glyph in a header field result, in text order.
fn field_char_xs(d: &Document) -> Vec<(String, Vec<f32>)> {
    let mut c = LayoutCache::new();
    let l = layout(d, &mut c, &LayoutOptions::default());
    let mut out = Vec::new();
    for it in &l.pages[0].header {
        let Placed::Lines { para, .. } = it else { continue };
        for (k, text, ranges) in &para.shown {
            let cl = &para.clusters[*k];
            let glyphs = &para.glyphs[cl.g0 as usize..cl.g1 as usize];
            let xs = text
                .char_indices()
                .map(|(i, _)| glyphs.iter().zip(ranges).find(|(_, r)| r.start == i).map(|(g, _)| g.dx).unwrap_or(f32::NAN))
                .collect();
            out.push((text.clone(), xs));
        }
    }
    out
}

/// A STYLEREF showing a Hebrew heading reads right to left in the header like ordinary text
/// (#1670): "פרק א" draws פ rightmost, not reversed as "א קרפ". Latin results keep their order,
/// also in a right-to-left header paragraph.
#[test]
fn hebrew_styleref_result_reads_right_to_left() {
    let d = doc(&[("Heading1", "פרק א", false), ("Normal", "Text.", false)], &[r#"STYLEREF "Heading 1""#]);
    let got = field_char_xs(&d);
    let (text, xs) = &got[0];
    assert_eq!(text, "פרק א");
    assert!(xs.iter().all(|x| x.is_finite()), "every letter has a glyph: {xs:?}");
    assert!(xs.windows(2).all(|w| w[0] > w[1]), "letters in text order run right to left: {xs:?}");
    // A Latin heading in a right-to-left header paragraph still reads left to right.
    let mut d = doc(&[("Heading1", "Part one", false)], &[r#"STYLEREF "Heading 1""#]);
    let hdr = d.last_section.headers.default.unwrap();
    d.para_mut(StoryRef::Part(hdr), &Path::top(0)).unwrap().props.bidi = Some(true);
    let got = field_char_xs(&d);
    let (text, xs) = &got[0];
    assert_eq!(text, "Part one");
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "Latin letters run left to right: {xs:?}");
}

/// What each header field result draws, read left to right: each glyph's character, a bracket
/// read as the bracket its glyph shows (a mirrored `(` reads `)`).
fn field_drawn(d: &Document, opts: &LayoutOptions) -> Vec<String> {
    let mirror = |c: char| match c {
        '(' => ')',
        ')' => '(',
        '{' => '}',
        '}' => '{',
        '[' => ']',
        ']' => '[',
        c => c,
    };
    let mut c = LayoutCache::new();
    let l = layout(d, &mut c, opts);
    let mut out = Vec::new();
    for it in &l.pages[0].header {
        let Placed::Lines { para, .. } = it else { continue };
        for (k, text, ranges) in &para.shown {
            let cl = &para.clusters[*k];
            // A fallback face's glyphs share the cluster, so a glyph is matched in every face.
            let gids = |ch: char| -> Vec<Option<u32>> {
                para.styles.iter().map(|st| wordcraft_fonts::shape_run(&st.face, &ch.to_string(), &[], |c| c, false).first().map(|g| g.gid)).collect()
            };
            let glyphs = &para.glyphs[cl.g0 as usize..cl.g1 as usize];
            let mut drawn: Vec<(f32, char)> = glyphs
                .iter()
                .zip(ranges)
                .map(|(g, r)| {
                    let ch = text[r.start..].chars().next().unwrap();
                    let m = mirror(ch);
                    let is = |ch: char| gids(ch).contains(&Some(g.gid));
                    let shown = if m != ch && is(m) && !is(ch) { m } else { ch };
                    (g.dx, shown)
                })
                .collect();
            drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.push(drawn.into_iter().map(|d| d.1).collect());
        }
    }
    out
}

fn rtl_header(d: &mut Document) {
    let hdr = d.last_section.headers.default.unwrap();
    d.para_mut(StoryRef::Part(hdr), &Path::top(0)).unwrap().props.bidi = Some(true);
}

/// Brackets in a field result are mirrored and placed by the same bidi run (#1670 review): a
/// field code or a Latin result in a right-to-left header keeps its brackets as written, and a
/// Hebrew word in brackets in a left-to-right result draws its brackets the right way round.
#[test]
fn field_results_mirror_brackets_with_their_bidi_runs() {
    let mut got = Vec::new();
    // The field-code view of a PAGE field in a right-to-left header.
    let mut d = doc(&[("Normal", "Text.", false)], &["PAGE"]);
    rtl_header(&mut d);
    got.extend(field_drawn(&d, &LayoutOptions { field_codes: true, ..LayoutOptions::default() }));
    // A Latin result with brackets, and digits, in a right-to-left header.
    for heading in ["(Chapter 1)", "12"] {
        let mut d = doc(&[("Heading1", heading, false)], &[r#"STYLEREF "Heading 1""#]);
        rtl_header(&mut d);
        got.extend(field_drawn(&d, &LayoutOptions::default()));
    }
    // A Hebrew word in brackets at the end of a left-to-right result, and the Hebrew heading.
    for heading in ["Part 1 (פרק)", "פרק א"] {
        let d = doc(&[("Heading1", heading, false)], &[r#"STYLEREF "Heading 1""#]);
        got.extend(field_drawn(&d, &LayoutOptions::default()));
    }
    assert_eq!(got, ["{ PAGE }", "(Chapter 1)", "12", "Part 1 (קרפ)", "א קרפ"]);
}
