//! Mentions, handles, one-line agent text and the brake limit.

/// Agent messages in a row before agents must wait for the owner.
pub const BRAKE_LIMIT: usize = 8;
/// Handles nobody may take. `@you` and `@me` also never count as mentions: `[@you]` is the
/// client's marker for "this line is for you".
pub const RESERVED: [&str; 4] = ["@owner", "@all", "@you", "@me"];
const NOT_MENTIONS: [&str; 2] = ["@you", "@me"];
/// Mentions that address every member.
pub const EVERYONE: [&str; 1] = ["@all"];
/// Distinct mentions kept per message: later ones are not mentions.
pub const MAX_MENTIONS: usize = 32;

/// The mentions address every member (`@all`).
pub fn addresses_everyone(mentions: &[String]) -> bool {
    mentions.iter().any(|m| EVERYONE.contains(&m.as_str()))
}

/// `@name` tokens not preceded by a letter, digit, `.` or `_` (so e-mail addresses don't count),
/// lowercased, in order, without duplicates; `@you` and `@me` are not mentions. Only the first
/// [`MAX_MENTIONS`] distinct ones count, so one message cannot make the work grow with its length
/// squared.
pub fn mentions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '.' || *c == '_');
    let handle_char = |c: &&char| c.is_ascii_alphanumeric() || **c == '-' || **c == '_';
    let mut out: Vec<String> = Vec::new();
    let mut i: usize = 0;
    while let Some(c) = chars.get(i) {
        let prev = i.checked_sub(1).and_then(|p| chars.get(p));
        if *c == '@' && !word(prev) {
            let len = chars.iter().skip(i.saturating_add(1)).take_while(handle_char).count();
            if len > 0 {
                let h: String =
                    std::iter::once('@').chain(chars.iter().skip(i.saturating_add(1)).take(len).copied()).collect::<String>().to_ascii_lowercase();
                if !out.contains(&h) && !NOT_MENTIONS.contains(&h.as_str()) {
                    out.push(h);
                    if out.len() >= MAX_MENTIONS {
                        break;
                    }
                }
            }
            i = i.saturating_add(len).saturating_add(1);
        } else {
            i = i.saturating_add(1);
        }
    }
    out
}

/// `@` + 1..=23 of `[a-z0-9_-]`, not reserved.
pub fn valid_handle(h: &str) -> bool {
    let Some(rest) = h.strip_prefix('@') else { return false };
    !rest.is_empty()
        && rest.len() <= 23
        && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !RESERVED.contains(&h)
}

/// Shown for a line break in agent text.
pub const LINE_MARK: &str = " \u{23CE} ";

/// Bidirectional-text controls (embeddings, overrides, isolates, marks): they can make a line
/// display in another order than it is stored.
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// Invisible characters: they can hide inside a word so that two lines look the same and are
/// not, or carry text that a model reads and a person does not see.
/// - zero-width space, non-joiner and joiner, word joiner, BOM, soft hyphen, the invisible math
///   operators (U+2061..=U+2064) and the Mongolian vowel separator;
/// - the Tags block (U+E0000..=U+E007F) and the variation selectors (U+FE00..=U+FE0F,
///   U+E0100..=U+E01EF);
/// - blank "filler" letters (Hangul fillers U+115F, U+1160, U+3164, U+FFA0; Braille blank U+2800)
///   and the interlinear annotation marks (U+FFF9..=U+FFFB).
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{2060}'
            | '\u{FEFF}'
            | '\u{AD}'
            | '\u{2061}'..='\u{2064}'
            | '\u{180E}'
            | '\u{E0000}'..='\u{E007F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{E0100}'..='\u{E01EF}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{3164}'
            | '\u{FFA0}'
            | '\u{2800}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Agent text on one line: line breaks (CR LF, CR, LF, U+2028, U+2029, U+0085) become " ⏎ " and
/// other control characters except tab are dropped, bidi controls and invisible characters
/// ([`is_invisible`]) too, so an agent cannot fake an "OWNER …" line in another agent's `listen`
/// output or in the pane.
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str(LINE_MARK);
            }
            '\n' | '\u{2028}' | '\u{2029}' | '\u{85}' => out.push_str(LINE_MARK),
            '\t' => out.push('\t'),
            c if c.is_control() || is_bidi_control(c) || is_invisible(c) => {}
            c => out.push(c),
        }
    }
    out
}

/// `Agent` / `@Agent` → `@agent`.
pub fn normalize_handle(h: &str) -> String {
    let t = h.trim().trim_start_matches('@').to_ascii_lowercase();
    format!("@{t}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_lowercase_handles() {
        assert_eq!(mentions("@Claude review clause 3, @pi too"), vec!["@claude", "@pi"]);
    }

    #[test]
    fn email_is_not_a_mention() {
        assert!(mentions("write to ann@example.com").is_empty());
    }

    #[test]
    fn all_is_a_mention() {
        assert_eq!(mentions("@all stop"), vec!["@all"]);
    }

    #[test]
    fn one_line_rules() {
        assert_eq!(one_line("a\r\nb\rc\nd"), "a \u{23CE} b \u{23CE} c \u{23CE} d");
        assert_eq!(one_line("x\u{2028}y\u{2029}z\u{85}w"), "x \u{23CE} y \u{23CE} z \u{23CE} w");
        assert_eq!(one_line("\u{1b}[1m\u{7f}\u{80}\u{9f}\u{0}ok\tt"), "[1mok\tt");
        assert_eq!(one_line("café ✅"), "café ✅");
    }

    #[test]
    fn reserved_names_are_english_only() {
        for h in ["@owner", "@all", "@you", "@me"] {
            assert!(!valid_handle(h), "{h}");
        }
        assert_eq!(mentions("[@you] @me and @You, but @pi"), vec!["@pi"]);
        assert!(addresses_everyone(&mentions("@all stop")));
        assert!(!addresses_everyone(&mentions("@pi go")));
    }

    #[test]
    fn mentions_at_the_edges_never_index_out_of_range() {
        assert_eq!(mentions("@"), Vec::<String>::new());
        assert_eq!(mentions("x@"), Vec::<String>::new());
        assert_eq!(mentions("@a"), vec!["@a"]);
        assert_eq!(mentions(&"@".repeat(10_000)), Vec::<String>::new());
    }

    #[test]
    fn one_line_drops_bidi_controls() {
        let raw = "a\u{202A}b\u{202B}c\u{202C}d\u{202D}e\u{202E}f\u{2066}g\u{2067}h\u{2068}i\u{2069}j\u{200E}k\u{200F}l\u{061C}m";
        assert_eq!(one_line(raw), "abcdefghijklm");
        assert_eq!(one_line("OWNER \u{202E}]uoy@[ #5"), "OWNER ]uoy@[ #5");
    }

    #[test]
    fn one_line_drops_zero_width_characters() {
        assert_eq!(one_line("o\u{200B}k\u{200C} \u{200D}O\u{2060}WNER\u{FEFF}"), "ok OWNER");
    }

    #[test]
    fn one_line_drops_tags_variation_selectors_and_fillers() {
        // Tag characters spell "OWNER" for a model and show nothing to a person.
        let tags: String = "OWNER".chars().filter_map(|c| char::from_u32(0xE0000 + c as u32)).collect();
        assert_eq!(one_line(&format!("ok\u{E0001}{tags}\u{E007F}")), "ok");
        assert_eq!(one_line("a\u{FE00}b\u{FE0F}c\u{E0100}d\u{E01EF}e"), "abcde");
        assert_eq!(one_line("f\u{115F}g\u{1160}h\u{3164}i\u{FFA0}j\u{2800}k\u{AD}l"), "fghijkl");
        assert_eq!(one_line("m\u{2061}n\u{2062}o\u{2063}p\u{2064}q\u{180E}r\u{FFF9}s\u{FFFA}t\u{FFFB}u"), "mnopqrstu");
        assert_eq!(one_line("café ✅ 中文"), "café ✅ 中文");
    }

    #[test]
    fn mentions_are_capped() {
        let text: String = (0..5_000).map(|i| format!("@n{i} ")).collect();
        let m = mentions(&text);
        assert_eq!(m.len(), MAX_MENTIONS);
        assert_eq!(m.first().map(String::as_str), Some("@n0"));
        assert_eq!(m.last().map(String::as_str), Some("@n31"));
        // Repeats do not use up the cap.
        let text = format!("{} @pi", "@claude ".repeat(5_000));
        assert_eq!(mentions(&text), vec!["@claude", "@pi"]);
    }

    #[test]
    fn handle_rules() {
        assert!(valid_handle("@claude"));
        assert!(valid_handle("@agent-2_x"));
        assert!(!valid_handle("claude"));
        assert!(!valid_handle("@"));
        assert!(!valid_handle("@this-handle-is-far-too-long"));
        assert!(!valid_handle("@Ünicode"));
        assert_eq!(normalize_handle("Claude"), "@claude");
        assert_eq!(normalize_handle("@PI"), "@pi");
    }
}
