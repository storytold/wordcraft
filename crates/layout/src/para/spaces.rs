//! Legacy East Asian space balancing, after run shaping and before grid/tracking layout.

use super::builder::Builder;
use super::grid::east_asian;

fn narrow_space(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}')
}

/// Word preserves an ordinary space between Latin words, but balances repeated spaces,
/// paragraph-edge spaces and spaces touching East Asian text. Inspect the paragraph rather
/// than a formatting run so splitting a space sequence does not change its width.
pub(super) fn balance(b: &mut Builder, text: &str, base: usize, from: usize) {
    if !b.env.doc.settings.balance_single_byte_double_byte_width {
        return;
    }
    for c in b.clusters.iter_mut().skip(from) {
        let Some((start, end)) = c.start.checked_sub(base).zip(c.end.checked_sub(base)) else { continue };
        let Some(s) = text.get(start..end) else { continue };
        let mut chars = s.chars();
        if !chars.next().is_some_and(narrow_space) || chars.next().is_some() {
            continue;
        }
        let before = text.get(..start).and_then(|s| s.chars().next_back());
        let after = text.get(end..).and_then(|s| s.chars().next());
        let boundary = |ch: Option<char>| ch.is_none_or(|ch| narrow_space(ch) || ch == '\u{3000}' || ch.is_control() || east_asian(ch));
        if !boundary(before) && !boundary(after) {
            continue;
        }
        let Some(st) = b.styles.get(c.style as usize) else { continue };
        let spacing = if st.rc.fit_text.is_some_and(|fit| fit.width > 0.0) { 0.0 } else { st.rc.spacing };
        // The compatibility rule reserves half an em, independent of a proportional font's
        // native space advance. Word's measured expanded/condensed tracking on these spaces
        // follows the full-width spacing unit; ordinary Latin letters retain one unit.
        let bold = if st.synth_bold { st.size * st.rc.scale / 200.0 * wordcraft_fonts::word::BOLD_WIDTH_EM } else { 0.0 };
        let advance = st.size * st.rc.scale / 200.0 + bold + spacing * 2.0;
        if advance.is_finite() {
            c.adv = advance;
        }
    }
}
