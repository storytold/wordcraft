use super::entity::{unescape, unescape_attr};

// ---------------------------------------------------------------------------------------------
// Tokenizer

#[derive(Debug)]
pub(super) enum Tok {
    Text(String),
    Start { name: String, attrs: Vec<(String, String)>, self_close: bool },
    End(String),
}

pub(super) struct Lexer<'a> {
    pub(super) s: &'a str,
    pub(super) pos: usize,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        self.s.get(self.pos..).unwrap_or("")
    }

    /// Skip past the next occurrence of `pat` (ASCII case-insensitive), or to the end.
    fn skip_past(&mut self, pat: &str) -> usize {
        let start = self.pos;
        let hay = self.rest().as_bytes();
        let p = pat.as_bytes();
        let found = hay.windows(p.len().max(1)).position(|w| w.eq_ignore_ascii_case(p));
        match found {
            Some(k) => self.pos += k + p.len(),
            None => self.pos = self.s.len(),
        }
        found.map(|k| start + k).unwrap_or(self.s.len())
    }

    /// Skip past the next matching end tag `</tag...>` (ASCII case-insensitive), or to the end.
    pub(super) fn skip_past_end_tag(&mut self, tag: &str) {
        let prefix = format!("</{tag}");
        let p = prefix.as_bytes();
        while self.pos < self.s.len() {
            let hay = self.rest().as_bytes();
            let found = hay.windows(p.len().max(1)).position(|w| w.eq_ignore_ascii_case(p));
            let Some(k) = found else {
                self.pos = self.s.len();
                return;
            };
            let after_tag = k + p.len();
            let next_byte = hay.get(after_tag);
            if next_byte.is_none_or(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/') {
                self.pos += after_tag;
                self.skip_past(">");
                return;
            }
            self.pos += k + 2;
        }
    }

    pub(super) fn next(&mut self) -> Option<Tok> {
        let rest = self.rest();
        if rest.is_empty() {
            return None;
        }
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let t = unescape(&rest[..end]);
            self.pos += end;
            return Some(Tok::Text(t));
        }
        let b = rest.as_bytes();
        if rest.starts_with("<!--") {
            self.pos += 4;
            self.skip_past("-->");
            return Some(Tok::Text(String::new()));
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            self.skip_past(">");
            return Some(Tok::Text(String::new()));
        }
        if rest.starts_with("</") {
            let name: String = rest
                .strip_prefix("</")
                .unwrap_or("")
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-')
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if name.is_empty() {
                self.pos += 2;
                return Some(Tok::Text("</".into()));
            }
            self.skip_past(">");
            return Some(Tok::End(name));
        }
        if !b.get(1).is_some_and(u8::is_ascii_alphabetic) {
            self.pos += 1;
            return Some(Tok::Text("<".into()));
        }
        // Start tag.
        let name: String =
            rest[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').map(|c| c.to_ascii_lowercase()).collect();
        let mut i = 1 + name.len();
        let mut attrs = Vec::new();
        let mut self_close = false;
        loop {
            while b.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            match b.get(i) {
                None => break,
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(b'/') => {
                    if b.get(i + 1) == Some(&b'>') {
                        self_close = true;
                        i += 2;
                        break;
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let a0 = i;
            while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'=' | b'>' | b'/')) {
                i += 1;
            }
            let an = rest.get(a0..i).unwrap_or("").to_ascii_lowercase();
            if an.is_empty() {
                i += 1;
                continue;
            }
            while b.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            let mut val = String::new();
            if b.get(i) == Some(&b'=') {
                i += 1;
                while b.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                match b.get(i) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let v0 = i + 1;
                        let e = rest.get(v0..).and_then(|r| r.find(q as char)).map(|k| v0 + k).unwrap_or(rest.len());
                        val = unescape_attr(rest.get(v0..e).unwrap_or(""));
                        i = e + 1;
                    }
                    _ => {
                        let v0 = i;
                        while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>') {
                            i += 1;
                        }
                        val = unescape_attr(rest.get(v0..i).unwrap_or(""));
                    }
                }
            }
            if attrs.len() < 64 {
                attrs.push((an, val));
            }
        }
        self.pos += i.min(rest.len());
        Some(Tok::Start { name, attrs, self_close })
    }
}
