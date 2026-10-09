//! Mentions, handles and the brake limit.

/// Agent messages in a row before agents must wait for the owner.
pub const BRAKE_LIMIT: usize = 8;
/// Handles nobody may take, in both chat languages. `@you`/`@ti` and `@me`/`@eu` also never
/// count as mentions: `[@you]` (`[@ti]`) is the client's marker for "this line is for you".
pub const RESERVED: [&str; 8] = ["@owner", "@dono", "@all", "@todos", "@you", "@ti", "@me", "@eu"];
const NOT_MENTIONS: [&str; 4] = ["@you", "@ti", "@me", "@eu"];
/// Mentions that address every member.
pub const EVERYONE: [&str; 2] = ["@all", "@todos"];

/// The mentions address every member (`@all` or `@todos`).
pub fn addresses_everyone(mentions: &[String]) -> bool {
    mentions.iter().any(|m| EVERYONE.contains(&m.as_str()))
}

/// `@name` tokens not preceded by a letter, digit, `.` or `_` (so e-mail addresses don't count),
/// lowercased, in order, without duplicates; `@you`, `@ti`, `@me` and `@eu` are not mentions.
pub fn mentions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let prev_ok = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '.' || chars[i - 1] == '_');
        if chars[i] == '@' && prev_ok {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
                j += 1;
            }
            if j > i + 1 {
                let h: String = chars[i..j].iter().collect::<String>().to_ascii_lowercase();
                if !out.contains(&h) && !NOT_MENTIONS.contains(&h.as_str()) {
                    out.push(h);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
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

/// Invisible characters (zero-width space, non-joiner, joiner, word joiner, BOM): they can hide
/// inside a word so that two lines look the same and are not.
pub fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

/// Agent text on one line: line breaks (CR LF, CR, LF, U+2028, U+2029, U+0085) become " ⏎ " and
/// other control characters except tab are dropped, bidi controls and zero-width characters
/// too, so an agent cannot fake a "DONO …" line in another agent's `listen` output or in the
/// pane.
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

/// `Claude` / `@Claude` → `@claude`.
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
        assert!(mentions("write to joao@example.com").is_empty());
    }

    #[test]
    fn todos_is_a_mention() {
        assert_eq!(mentions("@todos stop"), vec!["@todos"]);
    }

    #[test]
    fn one_line_rules() {
        assert_eq!(one_line("a\r\nb\rc\nd"), "a \u{23CE} b \u{23CE} c \u{23CE} d");
        assert_eq!(one_line("x\u{2028}y\u{2029}z\u{85}w"), "x \u{23CE} y \u{23CE} z \u{23CE} w");
        assert_eq!(one_line("\u{1b}[1m\u{7f}\u{80}\u{9f}\u{0}ok\tt"), "[1mok\tt");
        assert_eq!(one_line("olá ✅"), "olá ✅");
    }

    #[test]
    fn ti_and_eu_are_never_mentions_or_handles() {
        assert_eq!(mentions("[@ti] ok, @eu e @Ti, mas @claude"), vec!["@claude"]);
        assert!(!valid_handle("@ti"));
        assert!(!valid_handle("@eu"));
        assert_eq!(normalize_handle("TI"), "@ti");
    }

    #[test]
    fn reserved_handles_in_both_languages() {
        for h in ["@dono", "@owner", "@todos", "@all", "@ti", "@you", "@eu", "@me"] {
            assert!(!valid_handle(h), "{h}");
        }
        assert_eq!(mentions("[@you] @me and @You, but @pi"), vec!["@pi"]);
        assert_eq!(mentions("@all stop, @todos parem"), vec!["@all", "@todos"]);
        assert!(addresses_everyone(&mentions("@all stop")));
        assert!(addresses_everyone(&mentions("@todos parem")));
        assert!(!addresses_everyone(&mentions("@pi go")));
    }

    #[test]
    fn one_line_drops_bidi_controls() {
        let raw = "a\u{202A}b\u{202B}c\u{202C}d\u{202D}e\u{202E}f\u{2066}g\u{2067}h\u{2068}i\u{2069}j\u{200E}k\u{200F}l\u{061C}m";
        assert_eq!(one_line(raw), "abcdefghijklm");
        assert_eq!(one_line("DONO \u{202E}]it@[ #5"), "DONO ]it@[ #5");
    }

    #[test]
    fn one_line_drops_zero_width_characters() {
        assert_eq!(one_line("o\u{200B}k\u{200C} \u{200D}D\u{2060}ONO\u{FEFF}"), "ok DONO");
    }

    #[test]
    fn handle_rules() {
        assert!(valid_handle("@claude"));
        assert!(valid_handle("@big-pickle_2"));
        assert!(!valid_handle("claude"));
        assert!(!valid_handle("@todos"));
        assert!(!valid_handle("@dono"));
        assert!(!valid_handle("@"));
        assert!(!valid_handle("@this-handle-is-far-too-long"));
        assert!(!valid_handle("@Ünicode"));
        assert_eq!(normalize_handle("Claude"), "@claude");
        assert_eq!(normalize_handle("@PI"), "@pi");
    }
}
