#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
    /// `\name` (a starred form keeps its `*`) or a control symbol like `\&`.
    Cmd(String),
    Open,
    Close,
    /// Text (spaces collapsed to one).
    Text(String),
    /// A blank line.
    Par,
    Amp,
    /// `~`: a non-breaking space.
    Tie,
    /// Math source, and whether it is displayed.
    Math(String, bool),
    /// The content of a verbatim-like environment.
    Verbatim(String),
    /// `\verb|…|`.
    Verb(String),
    /// `\begin{name}` and `\end{name}`.
    Begin(String),
    End(String),
}

const VERBATIM_ENVS: &[&str] = &["verbatim", "verbatim*", "Verbatim", "lstlisting", "minted", "alltt", "comment"];

const MATH_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "alignat",
    "alignat*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "flalign",
    "flalign*",
    "displaymath",
    "math",
    "eqnarray",
    "eqnarray*",
];

struct Lexer<'a> {
    s: &'a str,
    i: usize,
    out: Vec<Tok>,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i..).and_then(|r| r.chars().next())
    }
    fn rest(&self) -> &str {
        self.s.get(self.i..).unwrap_or("")
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }
    fn text(&mut self, t: &str) {
        if let Some(Tok::Text(s)) = self.out.last_mut() {
            s.push_str(t);
        } else {
            self.out.push(Tok::Text(t.to_string()));
        }
    }
    /// Skip the rest of a line (a comment), the line end and the next line's leading blanks.
    fn comment(&mut self) {
        while let Some(c) = self.bump() {
            if c == '\n' {
                break;
            }
        }
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }
    /// Read until `end` (not included; consumed) and return what came before.
    fn until(&mut self, end: &str) -> String {
        match self.rest().find(end) {
            Some(at) => {
                let s = self.rest().get(..at).unwrap_or("").to_string();
                self.i += at + end.len();
                s
            }
            None => {
                let s = self.rest().to_string();
                self.i = self.s.len();
                s
            }
        }
    }
    /// After `\begin` / `\end`: the `{name}`, if well formed.
    fn env_name(&mut self) -> Option<String> {
        let save = self.i;
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
        if self.peek() != Some('{') {
            self.i = save;
            return None;
        }
        self.bump();
        // Only the next 65 bytes are searched: looking through the rest of the file for a `}`
        // made a run of unclosed `\\begin{` quadratic.
        let Some(end) = self.rest().bytes().take(65).position(|b| b == b'}') else {
            // Unclosed or overlong: leave the `{` to be read as ordinary input.
            self.i = save;
            return None;
        };
        let r = self.rest();
        let name = r.get(..end).unwrap_or("").trim().to_string();
        self.i += end + 1;
        Some(name)
    }

    fn run(mut self) -> Vec<Tok> {
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    self.bump();
                    let r = self.rest();
                    let n = r.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(r.len());
                    if n == 0 {
                        let Some(sym) = self.bump() else { break };
                        match sym {
                            '(' => {
                                let m = self.until("\\)");
                                self.out.push(Tok::Math(m, false));
                            }
                            '[' => {
                                let m = self.until("\\]");
                                self.out.push(Tok::Math(m, true));
                            }
                            _ => self.out.push(Tok::Cmd(sym.to_string())),
                        }
                        continue;
                    }
                    let mut name = r.get(..n).unwrap_or("").to_string();
                    self.i += n;
                    if self.peek() == Some('*') {
                        self.bump();
                        name.push('*');
                    }
                    match name.as_str() {
                        "begin" => match self.env_name() {
                            Some(env) if VERBATIM_ENVS.contains(&env.as_str()) => {
                                // `minted{lang}` / `lstlisting[options]`: skip to the line end.
                                let _options = self.until("\n");
                                let v = self.until(&format!("\\end{{{env}}}"));
                                if env != "comment" {
                                    self.out.push(Tok::Verbatim(v));
                                }
                            }
                            Some(env) if MATH_ENVS.contains(&env.as_str()) => {
                                let m = self.until(&format!("\\end{{{env}}}"));
                                self.out.push(Tok::Math(m, env != "math"));
                            }
                            Some(env) => self.out.push(Tok::Begin(env)),
                            None => self.out.push(Tok::Cmd(name)),
                        },
                        "end" => match self.env_name() {
                            Some(env) => self.out.push(Tok::End(env)),
                            None => self.out.push(Tok::Cmd(name)),
                        },
                        "verb" | "verb*" => match self.bump() {
                            Some(delim) if !delim.is_alphabetic() && !delim.is_whitespace() => {
                                let v = self.until(&delim.to_string());
                                self.out.push(Tok::Verb(v));
                            }
                            _ => {}
                        },
                        _ => {
                            self.out.push(Tok::Cmd(name));
                            // A control word swallows the spaces (and one line end) after it.
                            let mut newline = false;
                            while let Some(c) = self.peek() {
                                if c == ' ' || c == '\t' || (c == '\n' && !newline) {
                                    newline |= c == '\n';
                                    self.bump();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
                '%' => self.comment(),
                '{' => {
                    self.bump();
                    self.out.push(Tok::Open);
                }
                '}' => {
                    self.bump();
                    self.out.push(Tok::Close);
                }
                '&' => {
                    self.bump();
                    self.out.push(Tok::Amp);
                }
                '~' => {
                    self.bump();
                    self.out.push(Tok::Tie);
                }
                '$' => {
                    self.bump();
                    if self.peek() == Some('$') {
                        self.bump();
                        let m = self.until("$$");
                        self.out.push(Tok::Math(m, true));
                    } else {
                        let m = self.until("$");
                        self.out.push(Tok::Math(m, false));
                    }
                }
                '\n' | '\r' | ' ' | '\t' => {
                    // Whitespace: a blank line ends the paragraph, anything else is one space.
                    let mut lines = 0;
                    while let Some(c) = self.peek() {
                        match c {
                            '\n' => lines += 1,
                            ' ' | '\t' | '\r' => {}
                            _ => break,
                        }
                        self.bump();
                    }
                    if lines >= 2 {
                        self.out.push(Tok::Par);
                    } else {
                        self.text(" ");
                    }
                }
                _ => {
                    let r = self.rest();
                    let n = r.find(['\\', '%', '{', '}', '&', '~', '$', '\n', '\r', ' ', '\t']).unwrap_or(r.len()).max(c.len_utf8());
                    let t = r.get(..n).unwrap_or("").to_string();
                    self.i += n;
                    self.text(&t);
                }
            }
        }
        self.out
    }
}

pub(super) fn tokenize(src: &str) -> Vec<Tok> {
    Lexer { s: src, i: 0, out: Vec::new() }.run()
}

/// TeX's text ligatures: dashes and quotes.
pub(super) fn ligatures(s: &str) -> String {
    s.replace("---", "—").replace("--", "–").replace("``", "“").replace("''", "”").replace('`', "‘").replace("<<", "«").replace(">>", "»")
}

/// Character for a symbol command, if it is one.
pub(super) fn text_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "&" => "&",
        "%" => "%",
        "$" => "$",
        "#" => "#",
        "_" => "_",
        "{" => "{",
        "}" => "}",
        " " => " ",
        "," | ";" | ":" | "enspace" | "thinspace" => " ",
        "quad" => "\u{2003}",
        "qquad" => "\u{2003}\u{2003}",
        "-" | "/" | "@" | "!" | "noindent" | "indent" | "relax" | "protect" | "selectfont" | "nobreak" | "sloppy" | "fussy" | "centering"
        | "raggedright" | "raggedleft" | "normalfont" | "item" => "",
        "textbackslash" | "backslash" => "\\",
        "textasciitilde" => "~",
        "textasciicircum" => "^",
        "textless" => "<",
        "textgreater" => ">",
        "textbar" => "|",
        "textbraceleft" => "{",
        "textbraceright" => "}",
        "textunderscore" => "_",
        "textasciigrave" => "`",
        "textquotesingle" => "'",
        "textquotedbl" => "\"",
        "ldots" | "dots" | "textellipsis" => "…",
        "textendash" => "–",
        "textemdash" => "—",
        "textbullet" => "•",
        "textperiodcentered" => "·",
        "textquoteleft" => "‘",
        "textquoteright" => "’",
        "textquotedblleft" => "“",
        "textquotedblright" => "”",
        "guillemotleft" | "guillemetleft" => "«",
        "guillemotright" | "guillemetright" => "»",
        "copyright" | "textcopyright" => "©",
        "textregistered" => "®",
        "texttrademark" => "™",
        "S" | "textsection" => "§",
        "P" | "textparagraph" => "¶",
        "dag" | "textdagger" => "†",
        "ddag" | "textdaggerdbl" => "‡",
        "pounds" | "textsterling" => "£",
        "euro" | "texteuro" => "€",
        "textyen" => "¥",
        "textcent" => "¢",
        "textdegree" | "degree" => "°",
        "textmu" => "µ",
        "texttimes" => "×",
        "textdiv" => "÷",
        "textpm" => "±",
        "textonehalf" => "½",
        "textonequarter" => "¼",
        "textthreequarters" => "¾",
        "textexclamdown" => "¡",
        "textquestiondown" => "¿",
        "ss" => "ß",
        "SS" => "SS",
        "ae" => "æ",
        "AE" => "Æ",
        "oe" => "œ",
        "OE" => "Œ",
        "o" => "ø",
        "O" => "Ø",
        "aa" => "å",
        "AA" => "Å",
        "l" => "ł",
        "L" => "Ł",
        "i" => "ı",
        "j" => "ȷ",
        "LaTeX" => "LaTeX",
        "LaTeXe" => "LaTeX2ε",
        "TeX" => "TeX",
        "XeTeX" => "XeTeX",
        "LuaTeX" => "LuaTeX",
        "BibTeX" => "BibTeX",
        _ => return None,
    })
}

/// Combining mark for an accent command.
pub(super) fn accent_mark(name: &str) -> Option<char> {
    Some(match name {
        "'" => '\u{301}',
        "`" => '\u{300}',
        "^" => '\u{302}',
        "\"" => '\u{308}',
        "~" => '\u{303}',
        "=" => '\u{304}',
        "." => '\u{307}',
        "u" => '\u{306}',
        "v" => '\u{30C}',
        "H" => '\u{30B}',
        "c" => '\u{327}',
        "k" => '\u{328}',
        "r" => '\u{30A}',
        "d" => '\u{323}',
        "b" => '\u{331}',
        _ => return None,
    })
}

/// Precomposed letters for the common accents (base + combining mark otherwise).
pub(super) fn compose(base: char, mark: char) -> String {
    const TABLE: &[(char, char, char)] = &[
        ('a', '\u{301}', 'á'),
        ('e', '\u{301}', 'é'),
        ('i', '\u{301}', 'í'),
        ('ı', '\u{301}', 'í'),
        ('o', '\u{301}', 'ó'),
        ('u', '\u{301}', 'ú'),
        ('y', '\u{301}', 'ý'),
        ('c', '\u{301}', 'ć'),
        ('n', '\u{301}', 'ń'),
        ('s', '\u{301}', 'ś'),
        ('z', '\u{301}', 'ź'),
        ('A', '\u{301}', 'Á'),
        ('E', '\u{301}', 'É'),
        ('I', '\u{301}', 'Í'),
        ('O', '\u{301}', 'Ó'),
        ('U', '\u{301}', 'Ú'),
        ('Y', '\u{301}', 'Ý'),
        ('a', '\u{300}', 'à'),
        ('e', '\u{300}', 'è'),
        ('i', '\u{300}', 'ì'),
        ('ı', '\u{300}', 'ì'),
        ('o', '\u{300}', 'ò'),
        ('u', '\u{300}', 'ù'),
        ('A', '\u{300}', 'À'),
        ('E', '\u{300}', 'È'),
        ('I', '\u{300}', 'Ì'),
        ('O', '\u{300}', 'Ò'),
        ('U', '\u{300}', 'Ù'),
        ('a', '\u{302}', 'â'),
        ('e', '\u{302}', 'ê'),
        ('i', '\u{302}', 'î'),
        ('ı', '\u{302}', 'î'),
        ('o', '\u{302}', 'ô'),
        ('u', '\u{302}', 'û'),
        ('A', '\u{302}', 'Â'),
        ('E', '\u{302}', 'Ê'),
        ('I', '\u{302}', 'Î'),
        ('O', '\u{302}', 'Ô'),
        ('U', '\u{302}', 'Û'),
        ('a', '\u{308}', 'ä'),
        ('e', '\u{308}', 'ë'),
        ('i', '\u{308}', 'ï'),
        ('ı', '\u{308}', 'ï'),
        ('o', '\u{308}', 'ö'),
        ('u', '\u{308}', 'ü'),
        ('y', '\u{308}', 'ÿ'),
        ('A', '\u{308}', 'Ä'),
        ('E', '\u{308}', 'Ë'),
        ('I', '\u{308}', 'Ï'),
        ('O', '\u{308}', 'Ö'),
        ('U', '\u{308}', 'Ü'),
        ('a', '\u{303}', 'ã'),
        ('n', '\u{303}', 'ñ'),
        ('o', '\u{303}', 'õ'),
        ('A', '\u{303}', 'Ã'),
        ('N', '\u{303}', 'Ñ'),
        ('O', '\u{303}', 'Õ'),
        ('c', '\u{327}', 'ç'),
        ('C', '\u{327}', 'Ç'),
        ('s', '\u{327}', 'ş'),
        ('S', '\u{327}', 'Ş'),
        ('a', '\u{30A}', 'å'),
        ('A', '\u{30A}', 'Å'),
        ('u', '\u{30A}', 'ů'),
        ('c', '\u{30C}', 'č'),
        ('d', '\u{30C}', 'ď'),
        ('e', '\u{30C}', 'ě'),
        ('n', '\u{30C}', 'ň'),
        ('r', '\u{30C}', 'ř'),
        ('s', '\u{30C}', 'š'),
        ('t', '\u{30C}', 'ť'),
        ('z', '\u{30C}', 'ž'),
        ('C', '\u{30C}', 'Č'),
        ('D', '\u{30C}', 'Ď'),
        ('E', '\u{30C}', 'Ě'),
        ('N', '\u{30C}', 'Ň'),
        ('R', '\u{30C}', 'Ř'),
        ('S', '\u{30C}', 'Š'),
        ('T', '\u{30C}', 'Ť'),
        ('Z', '\u{30C}', 'Ž'),
        ('g', '\u{306}', 'ğ'),
        ('G', '\u{306}', 'Ğ'),
        ('o', '\u{30B}', 'ő'),
        ('u', '\u{30B}', 'ű'),
        ('O', '\u{30B}', 'Ő'),
        ('U', '\u{30B}', 'Ű'),
        ('a', '\u{328}', 'ą'),
        ('e', '\u{328}', 'ę'),
        ('A', '\u{328}', 'Ą'),
        ('E', '\u{328}', 'Ę'),
        ('z', '\u{307}', 'ż'),
        ('Z', '\u{307}', 'Ż'),
        ('a', '\u{304}', 'ā'),
        ('e', '\u{304}', 'ē'),
        ('i', '\u{304}', 'ī'),
        ('o', '\u{304}', 'ō'),
        ('u', '\u{304}', 'ū'),
    ];
    match TABLE.iter().find(|(b, m, _)| *b == base && *m == mark) {
        Some((_, _, c)) => c.to_string(),
        None => format!("{base}{mark}"),
    }
}
