//! The renderer: a parsed [`Style`] drawn against a document's [`Source`]s, as plain-text
//! citations and bibliography entries.
//!
//! The input side maps a `Source` onto CSL item variables: `author`/`editor` parsed from
//! the "Last, First; …" field format into name parts, `journal→container-title`, `kind→type`
//! (the import map in `crates/formats` reversed, with the journal-article type spelled as
//! the spec has it, "article-journal"), everything else direct. The output side walks the
//! style's layouts:
//!
//! - affixes render only when the element beneath them renders something (the spec's rule,
//!   applied per element — a missing variable renders nothing, not bare brackets); layout
//!   affixes are the exception, they always render;
//! - layout bodies concatenate their children with no separator: a layout's `delimiter`
//!   only ever separated the cites of a cluster, and each call renders one cite or entry;
//! - a group joins its non-empty children with its delimiter and disappears entirely —
//!   affixes included — when they are all empty;
//! - explicit date parts render in the order the style wrote them (styles carry the
//!   spacing in part affixes); `form="numeric"`/`form="text"` render the en-US localized
//!   shapes m/d/y and "month day, year".
//!
//! Macro references the parser cut render as their empty value, affixes dropped with it.
//! Everything is plain text; formatting attributes were already reported by the parser.
//! Missing variables render empty, and hostile values (gigantic strings, odd separators,
//! numbers that overflow every form) render something, never panic.

use std::cmp::Ordering;

use wordcraft_doc::Source;

use crate::model::{
    AndStyle, Branch, Choose, Condition, Date, DatePart, DatePartName, DateParts, Element, Form, Group, Label, Layout, Match, NameAsSortOrder,
    NameOptions, Names, Number, Plural, Position, SortDirection, SortField, SortKey, Style, Text, TextSource,
};
use crate::terms::TermSet;

/// The most names rendered from one variable (a hostile `author` field can name thousands).
pub(crate) const MAX_NAMES: usize = 256;
/// Rendering recursion backstop. Macro expansion was bounded at parse time (32 levels);
/// this guards a hand-built model that wasn't.
pub(crate) const MAX_RENDER_DEPTH: u32 = 64;
/// How many digit runs are read while parsing an `accessed` date (bounding hostile input).
pub(crate) const MAX_DATE_RUNS: usize = 8;
/// How many characters of an `accessed` value are scanned for its date (a valid
/// `yyyy-mm-dd` prefix fits in far less).
pub(crate) const MAX_DATE_SCAN: usize = 64;

// ---------------------------------------------------------------------------
// The item side: Source → CSL variables

/// One source under citation: its data plus the per-citation state a cite carries.
struct Item<'a> {
    source: &'a Source,
    /// The citation number (`citation-number`): the 1-based position in bibliography order
    /// for `entries`, the caller's number for `cite`.
    number: usize,
    /// The cited pages (`locator`): CSL's locator, typed "page" (all a Word citation has).
    locator: &'a str,
}

/// The rendering context threaded through the tree.
struct Ctx<'a> {
    style: &'a Style,
    item: Item<'a>,
    position: Position,
    /// Bibliography rendering: position conditions are false there (they test cites).
    in_bibliography: bool,
    /// The inheritable name options in scope (the section's, already merged over the
    /// root's at parse); every `<names>` renders under them where it sets nothing itself.
    name_defaults: NameOptions,
    depth: u32,
}

/// The variables that hold names.
const NAME_VARIABLES: &[&str] = &[
    "author",
    "editor",
    "translator",
    "container-author",
    "collection-editor",
    "director",
    "interviewer",
    "illustrator",
    "composer",
    "original-author",
    "reviewed-author",
];

/// A variable name matched the way styles write it (case-insensitively; `URL`/`DOI` are
/// uppercase in the spec, the rest lowercase).
fn var_is(name: &str, canonical: &str) -> bool {
    name.eq_ignore_ascii_case(canonical)
}

/// One person of a name field.
struct Person {
    family: String,
    given: String,
    /// A segment without a comma: an organization, or a name typed without its comma —
    /// rendered verbatim, never inverted or initialized.
    literal: bool,
}

/// The names of one name variable, parsed from the "Last, First; Last, First" field
/// format (only `author` and `editor` carry data; the rest are always empty).
fn names_of(source: &Source, variable: &str) -> Vec<Person> {
    let field = if var_is(variable, "author") {
        source.author.as_str()
    } else if var_is(variable, "editor") {
        source.editor.as_str()
    } else {
        ""
    };
    field
        .split(';')
        .filter(|segment| !segment.trim().is_empty())
        .take(MAX_NAMES)
        .map(|segment| match segment.split_once(',') {
            Some((family, given)) => Person { family: family.trim().to_string(), given: given.trim().to_string(), literal: false },
            None => Person { family: segment.trim().to_string(), given: String::new(), literal: true },
        })
        .filter(|p| !p.family.is_empty() || !p.given.is_empty())
        .collect()
}

/// A plain-field variable's value, trimmed (in the spelling styles use).
fn plain_variable(source: &Source, name: &str) -> String {
    let s = source;
    if var_is(name, "title") {
        s.title.trim().to_string()
    } else if var_is(name, "container-title") || var_is(name, "journal") {
        s.journal.trim().to_string()
    } else if var_is(name, "publisher") {
        s.publisher.trim().to_string()
    } else if var_is(name, "publisher-place") {
        s.city.trim().to_string()
    } else if var_is(name, "volume") {
        s.volume.trim().to_string()
    } else if var_is(name, "issue") {
        s.issue.trim().to_string()
    } else if var_is(name, "page") {
        s.pages.trim().to_string()
    } else if var_is(name, "edition") {
        s.edition.trim().to_string()
    } else if var_is(name, "URL") {
        s.url.trim().to_string()
    } else if var_is(name, "DOI") {
        s.doi.trim().to_string()
    } else {
        String::new()
    }
}

/// A month 1–12 (a value that isn't one has no month).
fn month_of(v: &str) -> Option<u32> {
    v.trim().parse().ok().filter(|m| (1..=12).contains(m))
}

/// A day 1–31.
fn day_of(v: &str) -> Option<u32> {
    v.trim().parse().ok().filter(|d| (1..=31).contains(d))
}

/// A date variable's parts: year, month, day, with the absent parts empty/`None`.
fn date_parts(source: &Source, name: &str) -> (String, Option<u32>, Option<u32>) {
    if var_is(name, "issued") {
        (source.year.trim().to_string(), month_of(&source.month), day_of(&source.day))
    } else if var_is(name, "accessed") {
        accessed_parts(&source.accessed)
    } else {
        (String::new(), None, None)
    }
}

/// An `accessed` value is free text: read a `yyyy-mm-dd`-ish leading date (the shape the
/// importers write, and the only one that can be trusted); anything else has no parts.
fn accessed_parts(v: &str) -> (String, Option<u32>, Option<u32>) {
    let mut runs: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in v.chars().take(MAX_DATE_SCAN) {
        if c.is_ascii_digit() {
            current.push(c);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
            if runs.len() >= MAX_DATE_RUNS {
                break;
            }
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    let mut runs = runs.into_iter();
    let Some(year) = runs.next() else { return (String::new(), None, None) };
    if year.chars().count() != 4 {
        return (String::new(), None, None);
    }
    let month = runs.next().and_then(|m| month_of(&m));
    let day = runs.next().and_then(|d| day_of(&d));
    (year, month, day)
}

/// The CSL type of a source: Task 1's import map (`crates/formats`, `kind_of`) reversed,
/// with the closest spec types for the kinds that map collapses to "other". The journal
/// article type is the spec's "article-journal" — the spelling real styles test (note:
/// slice 1's csljson importer spells it "journal-article"; flagged for follow-up there).
fn item_type(source: &Source) -> &'static str {
    match source.kind.as_str() {
        "article" => "article-journal",
        "book" => "book",
        "bookSection" => "chapter",
        "conference" => "paper-conference",
        "report" => "report",
        "website" => "webpage",
        "film" => "motion-picture",
        "periodical" => "periodical",
        _ => "document",
    }
}

/// The rendered value of a variable for a `<text>`: plain fields as written, names in
/// their long form, dates as `y-m-d`, and the per-cite state (`citation-number`,
/// `locator`) from the cite.
fn text_variable(ctx: &Ctx, name: &str) -> String {
    if var_is(name, "citation-number") {
        return if ctx.item.number > 0 { ctx.item.number.to_string() } else { String::new() };
    }
    if var_is(name, "locator") {
        return ctx.item.locator.trim().to_string();
    }
    if NAME_VARIABLES.iter().any(|v| var_is(name, v)) {
        return render_name_list(&ctx.style.locale_terms, &names_of(ctx.item.source, name), &ctx.name_defaults);
    }
    if var_is(name, "issued") || var_is(name, "accessed") {
        let (year, month, day) = date_parts(ctx.item.source, name);
        return date_as_text(&year, month, day);
    }
    plain_variable(ctx.item.source, name)
}

/// A date as a `<text>` renders it: `y-m-d` with the present parts.
fn date_as_text(year: &str, month: Option<u32>, day: Option<u32>) -> String {
    let mut s = year.to_string();
    if let Some(m) = month {
        s.push_str(&format!("-{m:02}"));
    }
    if let Some(d) = day {
        s.push_str(&format!("-{d:02}"));
    }
    s
}

/// A variable's raw field text (what sort keys and label pluralization read).
fn raw_variable(ctx: &Ctx, name: &str) -> String {
    if var_is(name, "citation-number") {
        return if ctx.item.number > 0 { ctx.item.number.to_string() } else { String::new() };
    }
    if var_is(name, "locator") {
        return ctx.item.locator.trim().to_string();
    }
    if var_is(name, "author") {
        return ctx.item.source.author.trim().to_string();
    }
    if var_is(name, "editor") {
        return ctx.item.source.editor.trim().to_string();
    }
    if var_is(name, "issued") || var_is(name, "accessed") {
        // Composed so the key orders chronologically: yyyymmdd.
        let (year, month, day) = date_parts(ctx.item.source, name);
        return format!(
            "{}{}{}",
            pad_digits(&year, 4),
            month.map_or(String::new(), |m| pad_digits(&m.to_string(), 2)),
            day.map_or(String::new(), |d| pad_digits(&d.to_string(), 2))
        );
    }
    plain_variable(ctx.item.source, name)
}

/// Left-pad an all-digit string to `width` (so "9" orders before "10").
fn pad_digits(s: &str, width: usize) -> String {
    let t = s.trim();
    let n = t.chars().count();
    if !t.is_empty() && n < width && t.bytes().all(|b| b.is_ascii_digit()) { format!("{}{}", "0".repeat(width - n), t) } else { t.to_string() }
}

/// A sort key's variable value: padded when it is a bare number, so numeric variables
/// order numerically.
fn pad_numeric(s: &str) -> String {
    pad_digits(s, 4)
}

/// Whether a variable has a value for this item (choose's `variable` test, and the gate
/// for labels).
fn variable_set(ctx: &Ctx, name: &str) -> bool {
    if var_is(name, "citation-number") {
        return ctx.item.number > 0;
    }
    if var_is(name, "locator") {
        return !ctx.item.locator.trim().is_empty();
    }
    if NAME_VARIABLES.iter().any(|v| var_is(name, v)) {
        return !names_of(ctx.item.source, name).is_empty();
    }
    if var_is(name, "issued") || var_is(name, "accessed") {
        let (year, month, day) = date_parts(ctx.item.source, name);
        return !year.is_empty() || month.is_some() || day.is_some();
    }
    !plain_variable(ctx.item.source, name).is_empty()
}

// ---------------------------------------------------------------------------
// The public API

impl Style {
    /// One in-text citation: the citation layout rendered for `item` at `position` (the
    /// first/subsequent/ibid state choose branches test), with `number` as the item's
    /// `citation-number` and `pages` as its `locator` (the cited pages).
    pub fn cite(&self, item: &Source, position: Position, number: usize, pages: &str) -> String {
        let mut ctx = Ctx {
            style: self,
            item: Item { source: item, number, locator: pages },
            position,
            in_bibliography: false,
            name_defaults: self.citation.name_options.clone(),
            depth: 0,
        };
        render_layout(&mut ctx, &self.citation).text
    }

    /// Bibliography entries in bibliography order: sorted by the bibliography's `<sort>`
    /// keys (no sort → the caller's order, i.e. first-cited), each with the sort key that
    /// placed it (one lowercased, number-padded string per sort key, the exact strings the
    /// order was decided on). Numeric styles get their `citation-number` from this order
    /// (1-based). A style without a `<bibliography>` renders no entries.
    pub fn entries(&self, items: &[Source]) -> Vec<(String, Vec<String>)> {
        let Some(bib) = self.bibliography.as_ref() else { return Vec::new() };
        let name_defaults = bib.layout.name_options.clone();
        let mut order: Vec<(usize, &Source, Vec<String>)> =
            items.iter().enumerate().map(|(i, s)| (i, s, self.sort_key(s, &bib.sort, &name_defaults))).collect();
        order.sort_by(|a, b| compare_keys(&a.2, &b.2, &bib.sort));
        let mut out = Vec::with_capacity(items.len());
        for (rank, (_, source, key)) in order.into_iter().enumerate() {
            let mut ctx = Ctx {
                style: self,
                item: Item { source, number: rank + 1, locator: "" },
                position: Position::First,
                in_bibliography: true,
                name_defaults: name_defaults.clone(),
                depth: 0,
            };
            out.push((render_layout(&mut ctx, &bib.layout).text, key));
        }
        out
    }

    /// The bibliography's title. CSL styles don't name their bibliography, so this is
    /// always `None` and the caller falls back to its own heading (e.g. "Bibliography").
    pub fn bib_title(&self) -> Option<&str> {
        None
    }

    /// The sort key of one source: one string per `<sort>` key, as it ordered.
    fn sort_key(&self, source: &Source, sort: &[SortKey], name_defaults: &NameOptions) -> Vec<String> {
        let mut ctx = Ctx {
            style: self,
            item: Item { source, number: 0, locator: "" },
            position: Position::First,
            in_bibliography: true,
            name_defaults: name_defaults.clone(),
            depth: 0,
        };
        sort.iter()
            .map(|k| match &k.field {
                SortField::Variable(v) => pad_numeric(&raw_variable(&ctx, v)),
                SortField::Macro(m) => render_macro(&mut ctx, m).text,
            })
            .map(|s| s.trim().to_lowercase())
            .collect()
    }
}

/// Compare two sort keys under the sort's directions; ties stay in the caller's order
/// (the sort is stable).
fn compare_keys(a: &[String], b: &[String], sort: &[SortKey]) -> Ordering {
    for (i, key) in sort.iter().enumerate() {
        let ord = a.get(i).map_or("", String::as_str).cmp(b.get(i).map_or("", String::as_str));
        if ord != Ordering::Equal {
            return match key.direction {
                SortDirection::Ascending => ord,
                SortDirection::Descending => ord.reverse(),
            };
        }
    }
    Ordering::Equal
}

// ---------------------------------------------------------------------------
// Element rendering

/// One element's render, carrying what the group-suppression rule needs beside the text:
/// whether any variable was called (directly or through a macro), and whether anything
/// called was non-empty — a variable with content, or (the spec's two follow-up
/// paragraphs) a nested group or macro that rendered.
struct Rendered {
    text: String,
    calls_variable: bool,
    nonempty_variable: bool,
}

impl Rendered {
    /// A term or value: its text, however long, never counts as a variable.
    fn plain(text: String) -> Rendered {
        Rendered { calls_variable: false, nonempty_variable: false, text }
    }

    /// A variable element: non-empty text means the variable had content.
    fn variable(text: String) -> Rendered {
        Rendered { calls_variable: true, nonempty_variable: !text.is_empty(), text }
    }

    /// The same render with the flags of another (a macro's body wrapped in affixes).
    fn with_text(self, text: String) -> Rendered {
        Rendered { text, calls_variable: self.calls_variable, nonempty_variable: self.nonempty_variable }
    }
}

/// Whether an element is a nested group or a macro call — the two the spec treats as a
/// non-empty variable for an enclosing group's suppression test when they render content.
fn is_group_or_macro(el: &Element) -> bool {
    match el {
        Element::Group(_) => true,
        Element::Text(t) => matches!(t.source, TextSource::Macro(_)),
        _ => false,
    }
}

/// How a child's render feeds its parent group's suppression test: its variable calls
/// always count toward the union, and a nested group's or macro's non-empty output counts
/// as a non-empty variable.
fn absorb(calls: &mut bool, nonempty: &mut bool, child: &Rendered, group_or_macro: bool) {
    *calls |= child.calls_variable;
    *nonempty |= child.nonempty_variable;
    if group_or_macro && !child.text.is_empty() {
        *nonempty = true;
    }
}

/// One rendering element.
fn render_element(ctx: &mut Ctx, el: &Element) -> Rendered {
    match el {
        Element::Text(t) => render_text(ctx, t),
        Element::Number(n) => render_number(ctx, n),
        Element::Date(d) => render_date(ctx, d),
        Element::Names(n) => render_names(ctx, n),
        Element::Label(l) => render_label(ctx, l),
        Element::Group(g) => render_group(ctx, g),
        Element::Choose(c) => render_choose(ctx, c),
    }
}

/// The elements of a body (a layout, macro or substitute chain) concatenated, with their
/// variable calls and non-emptiness absorbed for whoever's suppression test needs them.
fn render_body(ctx: &mut Ctx, els: &[Element]) -> Rendered {
    let mut calls = false;
    let mut nonempty = false;
    let mut text = String::new();
    for el in els {
        let r = render_element(ctx, el);
        absorb(&mut calls, &mut nonempty, &r, is_group_or_macro(el));
        text.push_str(&r.text);
    }
    Rendered { text, calls_variable: calls, nonempty_variable: nonempty }
}

/// A layout body: children concatenated with no separator. Layout affixes always render —
/// they are the citation's brackets, not conditional punctuation.
fn render_layout(ctx: &mut Ctx, l: &Layout) -> Rendered {
    let r = render_body(ctx, &l.elements);
    let text = format!("{}{}{}", l.prefix, r.text, l.suffix);
    r.with_text(text)
}

/// A group: non-empty children joined by the delimiter — and suppressed entirely, the
/// spec's conditional, when at least one child called a variable and nothing called had
/// content (a variable, a nested group, or a macro). This is what keeps a volume-less
/// book from wearing a bare "vol.": descriptive terms and labels ride along with their
/// variables. An all-empty group also renders nothing, affixes included.
fn render_group(ctx: &mut Ctx, g: &Group) -> Rendered {
    let mut calls = false;
    let mut nonempty = false;
    let mut parts: Vec<String> = Vec::with_capacity(g.elements.len());
    for el in &g.elements {
        let r = render_element(ctx, el);
        absorb(&mut calls, &mut nonempty, &r, is_group_or_macro(el));
        if !r.text.is_empty() {
            parts.push(r.text);
        }
    }
    if (calls && !nonempty) || parts.is_empty() {
        return Rendered { text: String::new(), calls_variable: calls, nonempty_variable: nonempty };
    }
    Rendered { text: format!("{}{}{}", g.prefix, parts.join(&g.delimiter), g.suffix), calls_variable: calls, nonempty_variable: nonempty }
}

/// A choose: the first branch whose conditions hold, else the else body.
fn render_choose(ctx: &mut Ctx, c: &Choose) -> Rendered {
    for b in &c.branches {
        if branch_matches(ctx, b) {
            return render_layout(ctx, &b.layout);
        }
    }
    c.else_branch.as_ref().map_or(Rendered::plain(String::new()), |e| render_layout(ctx, e))
}

/// All/any/none over the conditions; a branch with no conditions never matches (the
/// parser warns when an `if` only carried conditions outside the subset).
fn branch_matches(ctx: &Ctx, b: &Branch) -> bool {
    if b.conditions.is_empty() {
        return false;
    }
    match b.mode {
        Match::All => b.conditions.iter().all(|c| condition_holds(ctx, c)),
        Match::Any => b.conditions.iter().any(|c| condition_holds(ctx, c)),
        Match::None => b.conditions.iter().all(|c| !condition_holds(ctx, c)),
    }
}

fn condition_holds(ctx: &Ctx, c: &Condition) -> bool {
    match c {
        Condition::Variable { name, set } => variable_set(ctx, name) == *set,
        Condition::Type { names } => names.iter().any(|t| t.eq_ignore_ascii_case(item_type(ctx.item.source))),
        Condition::Position { value } => !ctx.in_bibliography && ctx.position == *value,
    }
}

/// Wrap a rendered body in its affixes when it rendered something. `strip_periods`
/// clears periods from the body (the value, not the style's own punctuation).
fn affixed(body: &str, prefix: &str, suffix: &str, strip_periods: bool) -> String {
    if body.is_empty() {
        return String::new();
    }
    let body = if strip_periods { body.replace('.', "") } else { body.to_string() };
    format!("{prefix}{body}{suffix}")
}

/// A `<text>`: a variable, macro, term or literal value.
fn render_text(ctx: &mut Ctx, t: &Text) -> Rendered {
    match &t.source {
        TextSource::Variable(v) => Rendered::variable(affixed(&text_variable(ctx, v), &t.prefix, &t.suffix, t.strip_periods)),
        TextSource::Macro(m) => {
            let r = render_macro(ctx, m);
            let text = affixed(&r.text, &t.prefix, &t.suffix, t.strip_periods);
            r.with_text(text)
        }
        TextSource::Term(name) => Rendered::plain(affixed(
            ctx.style.locale_terms.text(name, t.form.unwrap_or(Form::Long), t.plural.unwrap_or(false)).unwrap_or(""),
            &t.prefix,
            &t.suffix,
            t.strip_periods,
        )),
        TextSource::Value(v) => Rendered::plain(affixed(v, &t.prefix, &t.suffix, t.strip_periods)),
    }
}

/// A macro body: elements concatenated, bounded by the render depth. Macro references the
/// parser cut arrive here as an empty value with their affixes kept — and render nothing,
/// affixes dropped with the empty body. A non-empty macro counts as a non-empty variable
/// for an enclosing group's suppression test (the caller's absorb applies that).
fn render_macro(ctx: &mut Ctx, name: &str) -> Rendered {
    if ctx.depth >= MAX_RENDER_DEPTH {
        return Rendered::plain(String::new());
    }
    let style = ctx.style;
    let Some(body) = style.macros.get(name) else { return Rendered::plain(String::new()) };
    ctx.depth += 1;
    let out = render_body(ctx, body);
    ctx.depth -= 1;
    out
}

/// A `<number>`: a number variable in a number form. Values that aren't one number
/// (ranges, lists, text) render as written — the subset doesn't reformat page ranges.
fn render_number(ctx: &Ctx, n: &Number) -> Rendered {
    let raw = raw_variable(ctx, &n.variable);
    let body = match n.form {
        Some(Form::NumericLeadingZeros) => pad_digits(&raw, 2),
        Some(Form::Ordinal) => ordinal_string(ctx, &raw),
        Some(Form::Roman) => roman_string(&raw),
        _ => raw, // numeric, and the forms that don't apply to numbers
    };
    Rendered::variable(affixed(&body, &n.prefix, &n.suffix, false))
}

/// The ordinal form of a number value ("3" → "3rd"), suffixes from the locale's ordinal
/// terms so a style's overrides hold; a non-number renders as written.
fn ordinal_string(ctx: &Ctx, raw: &str) -> String {
    let Ok(n) = raw.trim().parse::<u64>() else { return raw.to_string() };
    let (tens, units) = (n % 100, n % 10);
    let term = if (11..=13).contains(&tens) {
        "ordinal-04"
    } else if units == 1 {
        "ordinal-01"
    } else if units == 2 {
        "ordinal-02"
    } else if units == 3 {
        "ordinal-03"
    } else {
        "ordinal"
    };
    let suffix = ctx.style.locale_terms.text(term, Form::Long, false).unwrap_or("th");
    format!("{n}{suffix}")
}

/// Lowercase roman numerals for 1–3999 (the classic range); anything else as written.
fn roman_string(raw: &str) -> String {
    let Ok(mut n) = raw.trim().parse::<u64>() else { return raw.to_string() };
    if n == 0 || n > 3999 {
        return raw.to_string();
    }
    let mut out = String::new();
    for (value, numeral) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

/// A `<date>`: its explicit parts in the order the style wrote them, or the en-US
/// localized forms when the style set a form instead of parts — narrowed to the parts
/// `date-parts` allows ("year", "year-month" or everything). A date with no parts at all
/// renders nothing.
fn render_date(ctx: &Ctx, d: &Date) -> Rendered {
    let (year, month, day) = date_parts(ctx.item.source, &d.variable);
    if year.is_empty() && month.is_none() && day.is_none() {
        return Rendered::variable(String::new());
    }
    let body = if !d.parts.is_empty() {
        let mut out = String::new();
        for p in &d.parts {
            out.push_str(&render_date_part(ctx, p, &year, month, day));
        }
        out
    } else {
        match d.form {
            Some(Form::Numeric) => localized_numeric_date(&year, month, day, d.date_parts),
            Some(Form::Text) => localized_text_date(ctx, &year, month, day, d.date_parts),
            _ => String::new(),
        }
    };
    Rendered::variable(affixed(&body, &d.prefix, &d.suffix, false))
}

/// One `<date-part>`; an absent part renders nothing, affixes included.
fn render_date_part(ctx: &Ctx, p: &DatePart, year: &str, month: Option<u32>, day: Option<u32>) -> String {
    let body = match p.name {
        DatePartName::Year => match p.form {
            Some(Form::Short) => short_year(year),
            _ => year.to_string(),
        },
        DatePartName::Month => match month {
            Some(m) => match p.form {
                Some(Form::Short) => month_term(ctx, m, Form::Short),
                Some(Form::Numeric) => m.to_string(),
                Some(Form::NumericLeadingZeros) => format!("{m:02}"),
                _ => month_term(ctx, m, Form::Long),
            },
            None => String::new(),
        },
        DatePartName::Day => match day {
            Some(d) => match p.form {
                Some(Form::NumericLeadingZeros) => format!("{d:02}"),
                Some(Form::Ordinal) => ordinal_string(ctx, &d.to_string()),
                _ => d.to_string(),
            },
            None => String::new(),
        },
    };
    affixed(&body, &p.prefix, &p.suffix, false)
}

/// The last two digits of a year ("2020" → "20"; "5" → "05").
fn short_year(year: &str) -> String {
    let t = year.trim();
    let n = t.chars().count();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        if n >= 2 { t.chars().skip(n - 2).collect() } else { format!("0{t}") }
    } else {
        t.to_string()
    }
}

/// A month's term ("month-09" long/short → "September"/"Sept.").
fn month_term(ctx: &Ctx, month: u32, form: Form) -> String {
    ctx.style.locale_terms.text(&format!("month-{month:02}"), form, false).unwrap_or("").to_string()
}

/// The parts a localized form may render, narrowed by `date-parts`.
fn allowed_parts(month: Option<u32>, day: Option<u32>, parts: DateParts) -> (Option<u32>, Option<u32>) {
    match parts {
        DateParts::Year => (None, None),
        DateParts::YearMonth => (month, None),
        DateParts::YearMonthDay => (month, day),
    }
}

/// en-US `form="numeric"`: the present parts as m/d/y ("3/15/2020"); missing parts (and
/// parts `date-parts` doesn't allow) drop their separator too, so a year alone is "2020".
fn localized_numeric_date(year: &str, month: Option<u32>, day: Option<u32>, parts: DateParts) -> String {
    let (month, day) = allowed_parts(month, day, parts);
    let mut out: Vec<String> = Vec::new();
    if let Some(m) = month {
        out.push(m.to_string());
    }
    if let Some(d) = day {
        out.push(d.to_string());
    }
    if !year.is_empty() {
        out.push(year.to_string());
    }
    out.join("/")
}

/// en-US `form="text"`: "September 1, 2020", parts dropping away as they are missing (or
/// as `date-parts` doesn't allow them).
fn localized_text_date(ctx: &Ctx, year: &str, month: Option<u32>, day: Option<u32>, parts: DateParts) -> String {
    let (month, day) = allowed_parts(month, day, parts);
    let mut out = String::new();
    if let Some(m) = month {
        out.push_str(&month_term(ctx, m, Form::Long));
    }
    if let Some(d) = day {
        out.push(' ');
        out.push_str(&d.to_string());
    }
    if !year.is_empty() {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(year);
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Names and labels

/// Names: the first variable that has names renders (styles list variables as fallback
/// chains), followed by the names' own label; when no variable has names, the substitute
/// chain is tried and the first element that renders something replaces the whole thing,
/// label included.
fn render_names(ctx: &mut Ctx, n: &Names) -> Rendered {
    let variable = n.variables.iter().find(|v| !names_of(ctx.item.source, v).is_empty());
    let Some(variable) = variable else {
        let Some(substitute) = &n.substitute else { return Rendered::variable(String::new()) };
        for el in substitute {
            let r = render_element(ctx, el);
            if !r.text.is_empty() {
                return r;
            }
        }
        return Rendered::variable(String::new());
    };
    let people = names_of(ctx.item.source, variable);
    let options = n.options.with_defaults(&ctx.name_defaults);
    let names = render_name_list(&ctx.style.locale_terms, &people, &options);
    let label = n.with_label.as_ref().map_or(String::new(), |l| names_label_text(ctx, l, variable, people.len()));
    // Label-first styles ("Edited by Smith") render the label where the style wrote it.
    Rendered::variable(if n.label_precedes_names { format!("{label}{names}") } else { format!("{names}{label}") })
}

/// A `<label>` inside `<names>`: the term of the variable that rendered, plural by the
/// name count (contextually; `plural` can force either).
fn names_label_text(ctx: &Ctx, l: &Label, variable: &str, count: usize) -> String {
    let plural = match l.plural {
        Some(Plural::Always) => true,
        Some(Plural::Never) => false,
        _ => count > 1,
    };
    let Some(text) = ctx.style.locale_terms.text(variable, l.form.unwrap_or(Form::Long), plural) else {
        return String::new();
    };
    format!("{}{}{}", l.prefix, text, l.suffix)
}

/// A list of names under the name options: et-al truncation from `et-al-min` keeping
/// `et-al-use-first` (defaulting to min−1, per the spec's default), the and-form joining
/// the last two names when not truncated, and the sort order per name.
fn render_name_list(terms: &TermSet, people: &[Person], options: &NameOptions) -> String {
    if people.is_empty() {
        return String::new();
    }
    let count = people.len();
    let min = options.et_al_min.unwrap_or(u32::MAX);
    let truncated = count as u64 >= min as u64;
    let keep = if truncated {
        let k = options.et_al_use_first.map_or(min.saturating_sub(1) as usize, |f| f as usize);
        k.min(count)
    } else {
        count
    };
    let delimiter = options.delimiter.as_deref().unwrap_or(", ");
    let rendered: Vec<String> = people
        .iter()
        .take(keep)
        .enumerate()
        .map(|(i, p)| {
            let sorted = match options.name_as_sort_order {
                Some(NameAsSortOrder::All) => true,
                Some(NameAsSortOrder::First) => i == 0,
                None => false,
            };
            render_person(p, options, sorted)
        })
        .collect();
    let mut out = if truncated {
        rendered.join(delimiter)
    } else {
        match (options.and, rendered.split_last()) {
            (Some(style), Some((last, rest))) if !rest.is_empty() => {
                let and = match style {
                    AndStyle::Text => terms.text("and", Form::Long, false).unwrap_or("and"),
                    AndStyle::Symbol => terms.text("and", Form::Symbol, false).unwrap_or("&"),
                };
                format!("{} {and} {last}", rest.join(delimiter))
            }
            _ => rendered.join(delimiter),
        }
    };
    if truncated {
        if !out.is_empty() {
            out.push_str(delimiter);
        }
        out.push_str(terms.text("et-al", Form::Long, false).unwrap_or("et al."));
    }
    out
}

/// One name: "family, given" in sort order, "given family" otherwise; `initialize-with`
/// abbreviates the given name to initials in both orders; a comma-less segment is a
/// literal name, rendered verbatim either way.
fn render_person(p: &Person, options: &NameOptions, sorted: bool) -> String {
    if p.literal || p.given.is_empty() {
        return p.family.clone();
    }
    let given = match &options.initialize_with {
        Some(marker) => initials(&p.given, marker),
        None => p.given.clone(),
    };
    if given.is_empty() {
        return p.family.clone();
    }
    if sorted { format!("{}, {}", p.family, given) } else { format!("{given} {}", p.family) }
}

/// Given-name initials: the first character of each word followed by the marker
/// ("Jane Anne", ". " → "J. A."); a hyphenated word takes one initial
/// (initialize-with-hyphen is outside the subset).
fn initials(given: &str, marker: &str) -> String {
    let mut out = String::new();
    for word in given.split_whitespace() {
        if let Some(first) = word.chars().next() {
            out.push(first);
            out.push_str(marker);
        }
    }
    out.trim_end().to_string()
}

/// A standalone `<label>`: the term of its variable ("page" → "p."/"pp."), plural by its
/// value. `locator` is the dynamic one: CSL resolves it to the locator type's term, and a
/// Word citation only ever carries pages, so it is the page term over the cited pages —
/// empty when there is no locator.
fn render_label(ctx: &Ctx, l: &Label) -> Rendered {
    let (term, value) = label_value(ctx, &l.variable);
    let text = if value.is_empty() {
        String::new()
    } else if let Some(term) = term {
        let plural = match l.plural {
            Some(Plural::Always) => true,
            Some(Plural::Never) => false,
            _ => label_is_plural(ctx, &l.variable, &value),
        };
        ctx.style.locale_terms.text(term, l.form.unwrap_or(Form::Long), plural).map_or(String::new(), |t| format!("{}{}{}", l.prefix, t, l.suffix))
    } else {
        String::new()
    };
    Rendered::variable(text)
}

/// The term name and value a label reads: `locator` → the page term over the cited pages;
/// the number variables with no locale term of their own have no label at all.
fn label_value<'a>(ctx: &'a Ctx, variable: &'a str) -> (Option<&'a str>, String) {
    if var_is(variable, "locator") {
        return (Some("page"), ctx.item.locator.trim().to_string());
    }
    if var_is(variable, "citation-number") || var_is(variable, "chapter-number") || var_is(variable, "first-reference-note-number") {
        return (None, String::new());
    }
    if NAME_VARIABLES.iter().any(|v| var_is(variable, v)) {
        return (Some(variable), raw_variable(ctx, variable));
    }
    (Some(variable), raw_variable(ctx, variable))
}

/// Contextual plurality: a names label is plural when several names rendered, a number
/// label when its value lists several (a range "10-11" or a list "12, 15").
fn label_is_plural(ctx: &Ctx, variable: &str, value: &str) -> bool {
    if NAME_VARIABLES.iter().any(|v| var_is(variable, v)) {
        return names_of(ctx.item.source, variable).len() > 1;
    }
    value.chars().any(|c| c == '-' || c == '–' || c == ',' || c == ';')
}
