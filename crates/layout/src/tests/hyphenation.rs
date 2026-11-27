use super::*;

fn automatic_breaks(text: &str) -> Vec<usize> {
    automatic_breaks_in(text, "en-US")
}

fn automatic_breaks_in(text: &str, lang: &str) -> Vec<usize> {
    let mut d = pagination_fixture_document(text);
    Arc::make_mut(&mut d.styles).default_chr.lang = Some(lang.into());
    d.settings.auto_hyphenation = true;
    let l = lay(&d);
    let p = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .find_map(|it| match it {
            Placed::Lines { para, .. } => Some(para),
            _ => None,
        })
        .unwrap();
    p.hyph_after.iter().map(|&i| p.clusters[i as usize].end).collect()
}

#[test]
fn unchecked_text_suppresses_automatic_but_not_optional_hyphens() {
    crate::test_fonts();
    let breaks = |text: &str, no_proof: bool, link: bool| {
        let mut d = pagination_fixture_document(text);
        d.settings.auto_hyphenation = true;
        d.last_section.page_w = d.last_section.margin_left + d.last_section.margin_right + 45.0;
        Arc::make_mut(&mut d.styles).default_chr.no_proof = Some(no_proof);
        Arc::make_mut(&mut d.styles).default_chr.link = link.then(|| "#target".into());
        lay(&d)
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { para, .. } => Some(para.lines.iter().filter(|line| line.hyphen.is_some()).count()),
                _ => None,
            })
            .sum::<usize>()
    };
    let ordinary = breaks("extraordinarily", false, false);
    assert!(ordinary > 0);
    assert_eq!(breaks("extraordinarily", false, true), ordinary, "links still permit automatic hyphens");
    for link in [false, true] {
        assert_eq!(breaks("extraordinarily", true, link), 0, "unchecked words have no automatic hyphens");
        assert_eq!(breaks("extra\u{ad}ordinarily", true, link), 1, "explicit optional hyphens remain available");
    }
}

#[test]
fn hyphenation_uses_the_first_character_of_the_remaining_word_fragment() {
    crate::test_fonts();
    let signature = |d: &Document| {
        lay(d)
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .filter_map(|item| match item {
                Placed::Lines { para, .. } => {
                    Some(para.lines.iter().map(|line| (para.clusters[line.c1 - 1].end, line.hyphen.is_some())).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
    };
    let text = "extraordinarily";
    let mut first_line_checked = false;
    let mut continuation_checked = false;
    for width in (20..76).step_by(2) {
        let mut d = pagination_fixture_document(text);
        d.settings.auto_hyphenation = true;
        d.last_section.page_w = d.last_section.margin_left + d.last_section.margin_right + width as f32;
        let ordinary = signature(&d);
        for range in [5..10, 10..text.len(), text.len() - 1..text.len()] {
            let mut partial = d.clone();
            partial.format_range(&Pos::body(0, range.start), &Pos::body(0, range.end), &|c| c.no_proof = Some(true)).unwrap();
            assert_eq!(signature(&partial)[0], ordinary[0], "unchecked interior text must not change the first fragment");
        }
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 1), &|c| c.no_proof = Some(true)).unwrap();
        let unchecked_start = signature(&d);
        assert!(!unchecked_start[0].1, "the unchecked initial character suppresses the first automatic hyphen");
        first_line_checked |= ordinary[0].1;
        continuation_checked |= unchecked_start.iter().skip(1).any(|line| line.1);
    }
    assert!(first_line_checked && continuation_checked, "exercise automatic hyphens both before and after a forced fragment break");
}

#[test]
fn spanish_yhe_breaks_do_not_depend_on_capital_boundaries() {
    crate::test_fonts();
    for word in ["copyhere", "CopyHere", "COPYHERE", "macyhetera", "macyHetera", "macyheTera", "macyhero"] {
        let h = word.to_ascii_lowercase().find("he").unwrap();
        assert!(automatic_breaks_in(word, "es-ES").contains(&h), "Spanish permits y-he in {word}");
    }
}

#[test]
fn spanish_suffixes_with_only_y_still_allow_explicit_optional_hyphens() {
    crate::test_fonts();
    for (word, cut) in [("rafesytr", 4), ("rafetry", 4)] {
        assert!(!automatic_breaks_in(word, "es-ES").contains(&cut), "{word} must not leave a suffix with only y");
    }
    assert!(automatic_breaks_in("rafe\u{ad}sytr", "es-ES").contains(&6), "an explicit optional hyphen is retained");
}

#[test]
fn automatic_hyphens_leave_three_final_letters() {
    crate::test_fonts();
    let words = ["hello", "happy", "family", "complicated", "responsibilities", "extraordinarily"];
    let general = wordcraft_proof::hyphen::Limits::default();
    assert!(
        words.iter().any(|w| wordcraft_proof::hyphen::hyphen_points(w, &general).contains(&(w.len() - 2))),
        "the fixture must exercise the general-purpose two-letter ending"
    );
    let mut found = false;
    for word in words {
        let breaks = automatic_breaks(word);
        found |= !breaks.is_empty();
        assert!(breaks.iter().all(|&i| i >= 2 && word.len() - i >= 3), "{word}: {breaks:?}");
    }
    assert!(found, "long prose words still have automatic hyphenation points");
}

#[test]
fn an_alphabetic_prefix_keeps_its_hyphenation_before_digits() {
    crate::test_fonts();
    assert!(!automatic_breaks("extraordinarily").is_empty());
    assert_eq!(automatic_breaks("extraordinarily123"), automatic_breaks("extraordinarily"));
    assert!(!automatic_breaks("extra\u{ad}ordinarily123").is_empty(), "an explicit optional hyphen remains usable");
}

#[test]
fn the_document_caps_option_controls_all_caps_hyphenation() {
    crate::test_fonts();
    let lower = automatic_breaks("responsibilities");
    assert!(!lower.is_empty());
    assert_eq!(automatic_breaks("RESPONSIBILITIES"), lower, "the enabled option uses the same patterns for capitals");
    let mut d = pagination_fixture_document("RESPONSIBILITIES");
    d.settings.auto_hyphenation = true;
    d.settings.hyphenate_caps = false;
    let l = lay(&d);
    assert!(l.pages.iter().flat_map(|p| &p.items).all(|it| match it {
        Placed::Lines { para, .. } => para.hyph_after.is_empty(),
        _ => true,
    }));
}

#[test]
fn justified_hyphen_prefixes_only_use_a_small_compression_budget() {
    crate::test_fonts();
    let text = format!("{}internationalization extraordinarily complicated responsibilities.", "a ".repeat(18));
    let mut compressed = false;
    for width in (100..280).step_by(3) {
        let mut d = pagination_fixture_document(&text);
        d.settings.auto_hyphenation = true;
        d.last_section.page_w = width as f32 + d.last_section.margin_left + d.last_section.margin_right;
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.align = Some(Align::Justify);
        let l = lay(&d);
        for item in l.pages.iter().flat_map(|p| &p.items) {
            let Placed::Lines { para, .. } = item else { continue };
            for line in &para.lines {
                let Some((style, _, hyphen)) = line.hyphen else { continue };
                let natural: f32 = para.clusters[line.c0..line.c1].iter().map(|c| c.adv).sum::<f32>() + hyphen;
                let overrun = natural - (line.right - line.left);
                compressed |= overrun > 0.1;
                assert!(
                    overrun <= para.styles[style as usize].size / 2.0,
                    "a hyphenated prefix must not use the whole-word squeezing budget: {overrun}"
                );
            }
        }
    }
    assert!(compressed, "justified prefixes can still compress spaces a little");
}

#[test]
fn internal_path_separators_keep_their_neighbouring_text_together() {
    crate::test_fonts();
    let text = "see example/filename or example\\filename and %PUBLIC%\\report123.exe next";
    let d = pagination_fixture_document(text);
    let l = lay(&d);
    let p = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .find_map(|it| match it {
            Placed::Lines { para, .. } => Some(para),
            _ => None,
        })
        .unwrap();
    assert!(p.clusters.iter().any(|c| c.break_after), "ordinary word boundaries remain usable");
    for (i, _) in text.match_indices(['/', '\\']) {
        for end in [i, i + 1] {
            let c = p.clusters.iter().find(|c| c.end == end).unwrap();
            assert!(!c.break_after, "an internal path separator is not a normal break: {end}");
        }
    }
}

#[test]
fn modern_hyphenation_ignores_the_saved_legacy_zone() {
    crate::test_fonts();
    let text = "International considerations permit extraordinarily complicated responsibilities. ".repeat(8);
    let breaks = |compat_mode, align, zone| {
        let mut d = pagination_fixture_document(&text);
        d.settings.auto_hyphenation = true;
        d.settings.compat_mode = compat_mode;
        d.settings.hyphenation_zone = zone;
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.align = Some(align);
        lay(&d)
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { para, .. } => Some(para.lines.iter().map(|l| (l.c0, l.c1, l.hyphen.is_some())).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
    };
    for align in [Align::Left, Align::Justify] {
        let narrow = breaks(wordcraft_doc::COMPAT_MODE_CURRENT, align, 0.0);
        let wide = breaks(wordcraft_doc::COMPAT_MODE_CURRENT, align, 72.0);
        assert!(narrow.iter().any(|l| l.2), "the example must exercise automatic hyphenation");
        assert_eq!(narrow, wide, "modern line breaks must not depend on the legacy zone");
    }
    assert_ne!(breaks(14, Align::Left, 0.0), breaks(14, Align::Left, 72.0), "older documents retain their zone");
}

#[test]
fn explicit_optional_hyphens_ignore_the_zone_in_both_layout_modes() {
    crate::test_fonts();
    let breaks = |compat_mode, align, zone| {
        let mut d = pagination_fixture_document(&"hello infor\u{ad}mation ".repeat(8));
        d.last_section.page_w = 142.0;
        d.last_section.margin_left = 36.0;
        d.last_section.margin_right = 36.0;
        d.settings.auto_hyphenation = false;
        d.settings.compat_mode = compat_mode;
        d.settings.hyphenation_zone = zone;
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.align = Some(align);
        lay(&d)
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { para, .. } => Some(para.lines.iter().map(|l| (l.c0, l.c1, l.hyphen.is_some())).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
    };
    for mode in [14, wordcraft_doc::COMPAT_MODE_CURRENT] {
        for align in [Align::Left, Align::Justify] {
            let narrow = breaks(mode, align, 0.0);
            let wide = breaks(mode, align, 72.0);
            assert!(narrow.iter().any(|l| l.2), "the optional hyphen must be used");
            assert_eq!(narrow, wide, "an explicit optional hyphen bypasses the zone");
        }
    }
}
