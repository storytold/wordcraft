/// How a style orders its bibliography (and, for numbered styles, numbers its citations).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Alphabetical by the first author's surname (the title when there is no author).
    Author,
    /// Alphabetical by title.
    Title,
    /// In order of first citation in the document.
    Cited,
}

/// How one author's name is written in a bibliography entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Name {
    /// Rivera, Alex
    LastFirst,
    /// Alex Rivera
    FirstLast,
    /// Rivera, A.
    LastInitials,
    /// A. Rivera
    InitialsLast,
    /// Rivera A.
    LastSpaceInitials,
    /// RIVERA, Alex
    UpperLastFirst,
    /// RIVERA A
    UpperLastBareInitials,
}

/// The author list of a bibliography entry.
#[derive(Clone, Copy, Debug)]
pub struct Names {
    /// The first author's form, and every later author's.
    pub first: Name,
    pub rest: Name,
    /// Between names; between exactly two; before the last of three or more.
    pub sep: &'static str,
    pub two: &'static str,
    pub last: &'static str,
    /// List at most this many authors, then `et_al` (0: list them all).
    pub max: usize,
    pub et_al: &'static str,
}

/// The in-text citation.
#[derive(Clone, Copy, Debug)]
pub enum Cite {
    /// `(Who<year_sep>Year<pages_sep>Pages)`.
    AuthorDate { year_sep: &'static str, pages_sep: &'static str },
    /// `(Who Pages)`.
    AuthorPage,
    /// `<open>n<close>`; pages follow `pages_sep` inside the brackets (left out when `None`).
    Number { open: &'static str, close: &'static str, pages_sep: Option<&'static str> },
}

/// A citation and bibliography style, as data. Entry templates fill `{n}`, `{names}`, `{year}`,
/// `{title}`, `{city}`, `{publisher}`, `{pub}` (city: publisher), `{journal}`, `{volume}`,
/// `{pages}`, `{url}` and `{type}` (GB/T 7714 document type code); text between `⟦` and `⟧` is
/// left out when a field in it is empty, and a full stop never doubles up.
#[derive(Debug)]
pub struct CiteStyle {
    /// The name shown in the style picker and stored in the session.
    pub name: &'static str,
    /// Lowercase letter/digit prefixes of Word's style names and style-sheet file names that mean
    /// this style (`gostname` matches "GOST - Name Sort"); the longest match wins.
    pub aliases: &'static [&'static str],
    pub cite: Cite,
    /// Joins two authors in a citation ("&" or "and").
    pub and: &'static str,
    /// Cite this many authors or more as "First et al.".
    pub et_al_from: usize,
    pub order: Order,
    pub names: Names,
    /// Entry template for a book (or anything without a journal), and for a journal article.
    pub book: &'static str,
    pub article: &'static str,
    /// Default title of the bibliography.
    pub bib_title: &'static str,
}

impl CiteStyle {
    /// Numbered styles cite `[1]`/`(1)` and list entries without a hanging indent.
    pub fn numbered(&self) -> bool {
        matches!(self.cite, Cite::Number { .. })
    }
}

const fn names(first: Name, rest: Name, sep: &'static str, two: &'static str, last: &'static str) -> Names {
    Names { first, rest, sep, two, last, max: 0, et_al: "" }
}

const AUTHOR_DATE: Cite = Cite::AuthorDate { year_sep: " ", pages_sep: ", p. " };

/// Every style, in picker order (alphabetical). Formats follow the published style guides'
/// rules for the source types WordCraft records, written in our own words.
pub static STYLES: [CiteStyle; 12] = [
    CiteStyle {
        name: "APA",
        aliases: &["apa"],
        cite: Cite::AuthorDate { year_sep: ", ", pages_sep: ", p. " },
        and: "&",
        et_al_from: 3,
        order: Order::Author,
        names: names(Name::LastInitials, Name::LastInitials, ", ", ", & ", ", & "),
        book: "⟦{names} ⟧({year}). {title}.⟦ {publisher}.⟧⟦ {url}⟧",
        article: "⟦{names} ⟧({year}). {title}. {journal}⟦, {volume}⟧⟦, {pages}⟧.⟦ {url}⟧",
        bib_title: "References",
    },
    CiteStyle {
        name: "Chicago",
        aliases: &["chicago"],
        cite: Cite::AuthorDate { year_sep: " ", pages_sep: ", " },
        and: "and",
        et_al_from: 3,
        order: Order::Author,
        names: names(Name::LastFirst, Name::FirstLast, ", ", ", and ", ", and "),
        book: "⟦{names}. ⟧{year}. {title}.⟦ {pub}.⟧⟦ {url}⟧",
        article: "⟦{names}. ⟧{year}. {title}. {journal}⟦, {volume}⟧⟦, {pages}⟧.⟦ {url}⟧",
        bib_title: "References",
    },
    // GB/T 7714 sequence-coding: numbered by first citation, surnames in capitals with bare
    // initials, at most three authors, a document type code after the title.
    CiteStyle {
        name: "GB/T 7714",
        aliases: &["gb"],
        cite: Cite::Number { open: "[", close: "]", pages_sep: Some(", ") },
        and: "and",
        et_al_from: 3,
        order: Order::Cited,
        names: Names { max: 3, et_al: ", et al.", ..names(Name::UpperLastBareInitials, Name::UpperLastBareInitials, ", ", ", ", ", ") },
        book: "[{n}] ⟦{names}. ⟧{title}[{type}].⟦ {pub},⟧ {year}.⟦ {url}.⟧",
        article: "[{n}] ⟦{names}. ⟧{title}[{type}]. {journal}, {year}⟦, {volume}⟧⟦: {pages}⟧.⟦ {url}.⟧",
        bib_title: "References",
    },
    // GOST 7.1: a numbered list in alphabetical order (by author or by title); areas separated
    // by " – ", the container after "//".
    CiteStyle {
        name: "GOST (name sort)",
        aliases: &["gost", "gostname"],
        cite: Cite::Number { open: "[", close: "]", pages_sep: Some(", p. ") },
        and: "and",
        et_al_from: 3,
        order: Order::Author,
        names: names(Name::LastSpaceInitials, Name::LastSpaceInitials, ", ", ", ", ", "),
        book: GOST_BOOK,
        article: GOST_ARTICLE,
        bib_title: "References",
    },
    CiteStyle {
        name: "GOST (title sort)",
        aliases: &["gosttitle"],
        cite: Cite::Number { open: "[", close: "]", pages_sep: Some(", p. ") },
        and: "and",
        et_al_from: 3,
        order: Order::Title,
        names: names(Name::LastSpaceInitials, Name::LastSpaceInitials, ", ", ", ", ", "),
        book: GOST_BOOK,
        article: GOST_ARTICLE,
        bib_title: "References",
    },
    // Harvard as taught by Anglia Ruskin: "Surname, I., Year. Title. Place: Publisher."
    CiteStyle {
        name: "Harvard (Anglia)",
        aliases: &["harvard"],
        cite: Cite::AuthorDate { year_sep: ", ", pages_sep: ", p. " },
        and: "and",
        et_al_from: 4,
        order: Order::Author,
        names: names(Name::LastInitials, Name::LastInitials, ", ", " and ", " and "),
        book: "⟦{names}, ⟧{year}. {title}.⟦ {pub}.⟧⟦ Available at: <{url}>.⟧",
        article: "⟦{names}, ⟧{year}. {title}. {journal}⟦, {volume}⟧⟦, pp. {pages}⟧.⟦ Available at: <{url}>.⟧",
        bib_title: "References",
    },
    CiteStyle {
        name: "IEEE",
        aliases: &["ieee"],
        cite: Cite::Number { open: "[", close: "]", pages_sep: None },
        and: "and",
        et_al_from: 3,
        order: Order::Cited,
        names: names(Name::InitialsLast, Name::InitialsLast, ", ", ", ", ", "),
        book: "[{n}] ⟦{names}, ⟧“{title},”⟦ {pub},⟧ {year}.⟦ {url}⟧",
        article: "[{n}] ⟦{names}, ⟧“{title},” {journal}⟦, {volume}⟧⟦, {pages}⟧, {year}.⟦ {url}⟧",
        bib_title: "References",
    },
    // ISO 690 first element and date: surnames in capitals, the year after the names.
    CiteStyle {
        name: "ISO 690 (author-date)",
        aliases: &["iso690"],
        cite: AUTHOR_DATE,
        and: "and",
        et_al_from: 4,
        order: Order::Author,
        names: names(Name::UpperLastFirst, Name::UpperLastFirst, ", ", " and ", " and "),
        book: "⟦{names}, ⟧{year}. {title}.⟦ {pub}.⟧⟦ Available from: {url}⟧",
        article: "⟦{names}, ⟧{year}. {title}. {journal}⟦, {volume}⟧⟦, {pages}⟧.⟦ Available from: {url}⟧",
        bib_title: "References",
    },
    // ISO 690 numeric reference: numbered by first citation, the year with the imprint.
    CiteStyle {
        name: "ISO 690 (numerical)",
        aliases: &["iso690num", "iso690nmerical"],
        cite: Cite::Number { open: "(", close: ")", pages_sep: Some(", p. ") },
        and: "and",
        et_al_from: 4,
        order: Order::Cited,
        names: names(Name::UpperLastFirst, Name::UpperLastFirst, ", ", " and ", " and "),
        book: "{n}. ⟦{names}. ⟧{title}.⟦ {pub},⟧ {year}.⟦ Available from: {url}⟧",
        article: "{n}. ⟦{names}. ⟧{title}. {journal}. {year}⟦, {volume}⟧⟦, {pages}⟧.⟦ Available from: {url}⟧",
        bib_title: "References",
    },
    CiteStyle {
        name: "MLA",
        aliases: &["mla"],
        cite: Cite::AuthorPage,
        and: "and",
        et_al_from: 3,
        order: Order::Author,
        names: Names { max: 1, et_al: ", et al.", ..names(Name::LastFirst, Name::LastFirst, ", ", ", ", ", ") },
        book: "⟦{names}. ⟧“{title}.”⟦ {publisher},⟧ {year}.⟦ {url}⟧",
        article: "⟦{names}. ⟧“{title}.” {journal}⟦, {volume}⟧⟦, {pages}⟧, {year}.⟦ {url}⟧",
        bib_title: "Works Cited",
    },
    // SIST 02: numbered by first citation, full names separated by semicolons.
    CiteStyle {
        name: "SIST02",
        aliases: &["sist"],
        cite: Cite::Number { open: "(", close: ")", pages_sep: Some(", p. ") },
        and: "and",
        et_al_from: 3,
        order: Order::Cited,
        names: names(Name::LastFirst, Name::LastFirst, "; ", "; ", "; "),
        book: "{n}) ⟦{names}. ⟧{title}.⟦ {city},⟧⟦ {publisher},⟧ {year}.⟦ {url}⟧",
        article: "{n}) ⟦{names}. ⟧{title}. {journal}. {year}⟦, vol. {volume}⟧⟦, p. {pages}⟧.⟦ {url}⟧",
        bib_title: "References",
    },
    // Turabian: author-date citations, a bibliography with the year in the imprint.
    CiteStyle {
        name: "Turabian",
        aliases: &["turabian"],
        cite: Cite::AuthorDate { year_sep: " ", pages_sep: ", " },
        and: "and",
        et_al_from: 4,
        order: Order::Author,
        names: names(Name::LastFirst, Name::FirstLast, ", ", ", and ", ", and "),
        book: "⟦{names}. ⟧{title}.⟦ {pub},⟧ {year}.⟦ {url}.⟧",
        article: "⟦{names}. ⟧“{title}.” {journal}⟦ {volume}⟧ ({year})⟦: {pages}⟧.⟦ {url}.⟧",
        bib_title: "Bibliography",
    },
];

const GOST_BOOK: &str = "{n}. ⟦{names} ⟧{title}. –⟦ {city} :⟧⟦ {publisher},⟧ {year}.⟦ – URL: {url}.⟧";

const GOST_ARTICLE: &str = "{n}. ⟦{names} ⟧{title} // {journal}. – {year}.⟦ – Vol. {volume}.⟧⟦ – P. {pages}.⟧⟦ – URL: {url}.⟧";

/// The style called `name`: one of ours (any case), or Word's style name or style-sheet file
/// name (`Harvard - Anglia`, `\GostTitle.XSL`).
pub fn find_style(name: &str) -> Option<&'static CiteStyle> {
    if let Some(st) = STYLES.iter().find(|x| x.name.eq_ignore_ascii_case(name.trim())) {
        return Some(st);
    }
    let key: String = name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let key = key.strip_suffix("xsl").unwrap_or(&key);
    STYLES
        .iter()
        .flat_map(|st| st.aliases.iter().map(move |a| (st, *a)))
        .filter(|(_, a)| key.starts_with(a))
        .max_by_key(|(_, a)| a.len())
        .map(|(st, _)| st)
}

/// The session's style; APA when the name is unknown.
pub fn style(name: &str) -> &'static CiteStyle {
    find_style(name).unwrap_or(&STYLES[0])
}
