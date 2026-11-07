//! Thai and Lao line-breaking tests: ICU4X word boundaries, word_wrap=false behaviour,
//! and force_char_break (vertical/stacked text).

#[test]
fn word_wrap_false_preserves_thai_icu4x_boundaries() {
    // word_wrap=false (the OOXML `w:wordWrap` property) means Latin words may break
    // mid-character.  Thai/Lao characters are alphanumeric and non-East-Asian, so they would
    // fall into the Latin break category — incorrectly inserting a break between every Thai
    // character and discarding the ICU4X dictionary boundaries.  The fix: Thai/Lao chars are
    // excluded from the !word_wrap path; their breaks come from thai_lao_breaks() only.
    let thai = "อนุรักษ์ภาษาไทย";
    let mut opps: Vec<usize> = unicode_linebreak::linebreaks(thai).map(|(i, _)| i).filter(|&i| i < thai.len()).collect();
    crate::kinsoku::apply(thai, &mut opps, true, false, false); // word_wrap=false, NOT stacked
    // Neither UAX #14 (SA resolves to AL without a dictionary) nor kinsoku's !word_wrap path may
    // add a break between two Thai characters: those come from thai_lao_breaks() alone.
    for &o in &opps {
        let prev = thai.get(..o).and_then(|s| s.chars().next_back());
        let next = thai.get(o..).and_then(|s| s.chars().next());
        let both_thai = matches!((prev, next), (Some(p), Some(n))
            if wordcraft_doc::cluster::is_thai_lao(p) && wordcraft_doc::cluster::is_thai_lao(n));
        assert!(!both_thai, "kinsoku added a Thai–Thai break at {o}: {opps:?}");
    }
}

#[test]
fn force_char_break_breaks_between_every_thai_char() {
    // force_char_break is set by textbox::lay for stacked (vertical CJK-style) text:
    // every character must be on its own line.  For Thai, this means a break between
    // every adjacent Thai grapheme, overriding the ICU4X-only restriction.
    let thai = "ภาษา"; // 4 Thai chars, 12 bytes, 3 inter-char positions
    let mut opps: Vec<usize> = unicode_linebreak::linebreaks(thai).map(|(i, _)| i).filter(|&i| i < thai.len()).collect();
    crate::kinsoku::apply(thai, &mut opps, true, false, true); // force_char_break=true
    // All three inter-character positions must be present.
    assert!(opps.contains(&3), "break after ภ: {opps:?}");
    assert!(opps.contains(&6), "break after า: {opps:?}");
    assert!(opps.contains(&9), "break after ษ: {opps:?}");
}
