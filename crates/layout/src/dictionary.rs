//! Adapter for dictionary-script wrapping. The shared core owns Khmer lexical decisions;
//! layout owns visible-text offsets, grapheme safety and explicit word-joiner controls.
//! The narrow adapter follows the separation proposed in #1579; #3110 informed the
//! repetition-sign and tab/justification regression review, without combining lexical engines.

/// Internal UTF-8 offsets supplied by the Khmer core, without initializing its data for
/// paragraphs in other scripts. Do not union competing lexical segmenters here.
pub(crate) fn breaks(text: &str) -> Vec<usize> {
    if !text.chars().any(|c| matches!(u32::from(c), 0x1780..=0x17FF)) {
        return Vec::new();
    }
    wordcraft_proof::khmer::line_breaks(text)
}

/// Boundaries glued by WJ or a Khmer repetition sign, kept separately from caret stops.
pub(crate) fn joined_boundaries(text: &str) -> Vec<usize> {
    let mut joined = Vec::new();
    for (i, c) in text.char_indices() {
        match c {
            '\u{2060}' => joined.extend([i, i + c.len_utf8()]),
            'ៗ' if i > 0 => joined.push(i),
            _ => {}
        }
    }
    joined
}

/// Apply layout's safety policy to opportunities from every source, including manual ZWSP.
/// Compute graphemes once: repeatedly querying a boundary can rescan long combining sequences.
pub(crate) fn retain_safe(text: &str, opportunities: &mut Vec<usize>) {
    if text.is_ascii() {
        let bytes = text.as_bytes();
        opportunities.retain(|&i| i <= bytes.len() && !(i > 0 && bytes.get(i - 1) == Some(&b'\r') && bytes.get(i) == Some(&b'\n')));
        opportunities.sort_unstable();
        opportunities.dedup();
        return;
    }
    let clusters = wordcraft_doc::cluster::cluster_starts(text);
    let joined = joined_boundaries(text);
    opportunities.retain(|&i| (i == text.len() || clusters.binary_search(&i).is_ok()) && joined.binary_search(&i).is_err());
    opportunities.sort_unstable();
    opportunities.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_safety_keeps_crlf_whole_and_sorts_valid_offsets() {
        let mut opportunities = vec![5, 3, 2, 1, 0, 2, 4, usize::MAX];
        retain_safe("a\r\nb", &mut opportunities);
        assert_eq!(opportunities, vec![0, 1, 3, 4]);
    }

    #[test]
    fn unrelated_text_gets_no_dictionary_boundaries() {
        for text in ["", "Hello wonderful world", "日本語", "ภาษาไทย"] {
            assert!(breaks(text).is_empty());
        }
    }

    #[test]
    fn controls_keep_caret_and_wrap_policies_separate() {
        let text = "មូល\u{200b}ដ្ឋាន\u{2060}ខ្មែរ";
        let manual = "មូល\u{200b}".len();
        let joined = joined_boundaries(text);
        let mut candidates: Vec<usize> = (0..=text.len()).collect();
        retain_safe(text, &mut candidates);
        assert!(candidates.contains(&manual));
        assert!(joined.iter().all(|i| !candidates.contains(i)));
        assert!(candidates.iter().all(|&i| wordcraft_doc::cluster::is_cluster_boundary(text, i)));
        assert!(joined.iter().all(|&i| text.is_char_boundary(i)));
    }
}
