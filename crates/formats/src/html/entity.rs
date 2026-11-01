// ---------------------------------------------------------------------------------------------
// Entities and decoding

/// Decode an entity body (`amp`, `#38`, `#x26`) to its text.
pub fn decode_entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let (digits, radix) = match num.strip_prefix(['x', 'X']) {
            Some(h) => (h, 16),
            None => (num, 10),
        };
        if !digits.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        let v = u32::from_str_radix(digits, radix).ok()?;
        let c = match v {
            0 => '\u{FFFD}',
            0x80..=0x9F => crate::txt::cp1252(v as u8),
            _ => char::from_u32(v).unwrap_or('\u{FFFD}'),
        };
        return Some(c.to_string());
    }
    let c = match name {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" | "APOS" => '\'',
        "nbsp" | "NBSP" => '\u{00A0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "shy" => '\u{00AD}',
        "copy" | "COPY" => '©',
        "reg" | "REG" => '®',
        "trade" | "TRADE" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "minus" => '−',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "frac13" => '⅓',
        "frac23" => '⅔',
        "frac18" => '⅛',
        "frac38" => '⅜',
        "frac58" => '⅝',
        "frac78" => '⅞',
        "sup1" => '¹',
        "sup2" => '²',
        "sup3" => '³',
        "micro" => 'µ',
        "permil" => '‰',
        "prime" => '′',
        "Prime" => '″',
        "oline" => '‾',
        "frasl" => '⁄',
        "sect" => '§',
        "para" => '¶',
        "euro" | "EURO" => '€',
        "pound" | "POUND" => '£',
        "yen" | "YEN" => '¥',
        "cent" | "CENT" => '¢',
        "curren" => '¤',
        "dagger" => '†',
        "Dagger" => '‡',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "lArr" => '⇐',
        "rArr" => '⇒',
        "uArr" => '⇑',
        "dArr" => '⇓',
        "hArr" => '⇔',
        "crarr" => '↵',
        "forall" => '∀',
        "part" => '∂',
        "exist" => '∃',
        "empty" => '∅',
        "nabla" => '∇',
        "isin" => '∈',
        "notin" => '∉',
        "ni" => '∋',
        "prod" => '∏',
        "sum" => '∑',
        "radic" => '√',
        "prop" => '∝',
        "ang" => '∠',
        "and" => '∧',
        "or" => '∨',
        "cap" => '∩',
        "cup" => '∪',
        "int" => '∫',
        "there4" => '∴',
        "sim" => '∼',
        "cong" => '≅',
        "asymp" | "approx" => '≈',
        "ne" => '≠',
        "equiv" => '≡',
        "le" => '≤',
        "ge" => '≥',
        "sub" => '⊂',
        "sup" => '⊃',
        "nsub" => '⊄',
        "sube" => '⊆',
        "supe" => '⊇',
        "oplus" => '⊕',
        "otimes" => '⊗',
        "perp" => '⊥',
        "sdot" => '⋅',
        "infin" => '∞',
        "loz" => '◊',
        "spades" => '♠',
        "clubs" => '♣',
        "hearts" => '♥',
        "diams" => '♦',
        "check" => '✓',
        "iexcl" => '¡',
        "iquest" => '¿',
        "szlig" => 'ß',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "eacute" => 'é',
        "egrave" => 'è',
        "ecirc" => 'ê',
        "euml" => 'ë',
        "Eacute" => 'É',
        "Egrave" => 'È',
        "Ecirc" => 'Ê',
        "Euml" => 'Ë',
        "aacute" => 'á',
        "agrave" => 'à',
        "acirc" => 'â',
        "atilde" => 'ã',
        "Agrave" => 'À',
        "Aacute" => 'Á',
        "Acirc" => 'Â',
        "Atilde" => 'Ã',
        "iacute" => 'í',
        "igrave" => 'ì',
        "icirc" => 'î',
        "iuml" => 'ï',
        "Iacute" => 'Í',
        "Igrave" => 'Ì',
        "Icirc" => 'Î',
        "Iuml" => 'Ï',
        "oacute" => 'ó',
        "ograve" => 'ò',
        "ocirc" => 'ô',
        "otilde" => 'õ',
        "Oacute" => 'Ó',
        "Ograve" => 'Ò',
        "Ocirc" => 'Ô',
        "Otilde" => 'Õ',
        "uacute" => 'ú',
        "ugrave" => 'ù',
        "ucirc" => 'û',
        "Uacute" => 'Ú',
        "Ugrave" => 'Ù',
        "Ucirc" => 'Û',
        "yacute" => 'ý',
        "yuml" => 'ÿ',
        "Yacute" => 'Ý',
        "ntilde" => 'ñ',
        "Ntilde" => 'Ñ',
        "ccedil" => 'ç',
        "Ccedil" => 'Ç',
        "aring" => 'å',
        "Aring" => 'Å',
        "oslash" => 'ø',
        "Oslash" => 'Ø',
        "aelig" => 'æ',
        "AElig" => 'Æ',
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "epsilon" => 'ε',
        "zeta" => 'ζ',
        "eta" => 'η',
        "theta" => 'θ',
        "iota" => 'ι',
        "kappa" => 'κ',
        "lambda" => 'λ',
        "mu" => 'μ',
        "nu" => 'ν',
        "xi" => 'ξ',
        "pi" => 'π',
        "rho" => 'ρ',
        "sigma" => 'σ',
        "tau" => 'τ',
        "upsilon" => 'υ',
        "phi" => 'φ',
        "chi" => 'χ',
        "psi" => 'ψ',
        "omega" => 'ω',
        "Gamma" => 'Γ',
        "Delta" => 'Δ',
        "Theta" => 'Θ',
        "Lambda" => 'Λ',
        "Xi" => 'Ξ',
        "Pi" => 'Π',
        "Sigma" => 'Σ',
        "Upsilon" => 'Υ',
        "Phi" => 'Φ',
        "Psi" => 'Ψ',
        "Omega" => 'Ω',
        "zwj" => '\u{200D}',
        "zwnj" => '\u{200C}',
        _ => return None,
    };
    Some(c.to_string())
}

/// Replace entity references in `s`.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        let after = &rest[k + 1..];
        // An entity name is short; looking further would rescan the whole text for every stray `&`.
        let mut cut = after.len().min(33);
        while !after.is_char_boundary(cut) {
            cut -= 1;
        }
        let end = after.get(..cut).and_then(|w| w.find(';'));
        match end.and_then(|e| decode_entity(&after[..e]).map(|t| (t, e))) {
            Some((t, e)) => {
                out.push_str(&t);
                rest = &after[e + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn sniff_charset(head: &str) -> Option<&str> {
    let (i, n) = head.find("charset").map(|i| (i, 7)).or_else(|| head.find("encoding").map(|i| (i, 8)))?;
    let after = head.get(i + n..)?.trim_start().strip_prefix('=')?;
    let trimmed = after.trim_start().trim_start_matches(['"', '\'']);
    let end = trimmed.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ';' | '>' | '/')).unwrap_or(trimmed.len());
    let label = trimmed[..end].trim();
    if label.is_empty() { None } else { Some(label) }
}

/// Decode HTML bytes: BOM, then a `charset` in the first 2 KB, else UTF-8 with a legacy fallback.
pub fn decode(b: &[u8]) -> String {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return crate::txt::decode(b);
    }
    let head = String::from_utf8_lossy(b.get(..b.len().min(2048)).unwrap_or(b)).to_ascii_lowercase();
    if let Some(label) = sniff_charset(&head)
        && let Some(s) = crate::txt::decode_as(b, label)
    {
        return s;
    }
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    crate::txt::decode(b)
}
