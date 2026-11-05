//! The CSL walk: XML tree → [`Style`], leniently.
//!
//! Structure follows the spec's style file layout: `<info>` metadata, `<locale>` term
//! overrides (en-US only), `<macro>` bodies, and the `<citation>`/`<bibliography>` rendering
//! sections. Unknown elements and attributes are ignored (the format is forward-compatible
//! by design); constructs that would silently change rendered output warn into
//! [`Style::warned`] instead. Macro references are resolved here: dangling references are
//! dropped with a warning, and any macro whose expansion would exceed
//! [`MAX_MACRO_RECURSION`] levels (a cycle, or a long enough chain) has its references cut,
//! so a renderer can never walk into an unbounded loop.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    AndStyle, Bibliography, Branch, Choose, Condition, Date, DatePart, DatePartName, DateParts, Element, Form, Group, Label, Layout, Match,
    NameAsSortOrder, NameOptions, Names, Number, Plural, Position, SortDirection, SortField, SortKey, Style, Text, TextSource,
};
use crate::terms::TermSet;
use crate::xml::{self, El};
use crate::{for_each_element, referenced_macros, walk_elements};

/// A style file may not be larger than this.
pub(crate) const MAX_STYLE_BYTES: usize = 2 << 20; // 2 MiB
/// Macro expansion is cut at this many levels; references beyond are dropped with a warning.
pub(crate) const MAX_MACRO_RECURSION: u32 = 32;
/// More macros than this and the rest are ignored (with a warning).
pub(crate) const MAX_MACROS: usize = 512;
/// Strings carried into the model are clamped to this many characters.
pub(crate) const MAX_TEXT_CHARS: usize = 4_000;
/// `Style.warnings` is deduplicated and capped.
pub(crate) const MAX_WARNINGS: usize = 256;
/// Entries kept from a `variable`/`type`/`position` condition list (APA-scale type lists
/// run to 18; hostile files are cut here, with a warning).
pub(crate) const MAX_CONDITION_VALUES: usize = 128;
/// Name variables kept from one `<names>` (the spec's name variables are 14; hostile
/// files are cut here, with a warning).
pub(crate) const MAX_NAME_VARIABLES: usize = 64;

/// Parse a CSL 1.0.2 style file.
///
/// `Err` carries a human-readable message (not a CSL style, XML errors, an over-size file, a
/// missing citation layout). Warnings list what loaded but isn't rendered ([`Style::warned`]).
pub fn parse(bytes: &[u8]) -> Result<Style, String> {
    if bytes.len() > MAX_STYLE_BYTES {
        return Err(format!("CSL style is larger than {} MiB", MAX_STYLE_BYTES >> 20));
    }
    let root = xml::parse(bytes).map_err(|e| format!("not a CSL style: {e}"))?;
    if root.local() != "style" {
        return Err(format!("not a CSL style: root element is '{}'", root.local()));
    }
    let mut w = Warnings::default();
    warn_style_attrs(&root, &mut w);
    let mut style_name_options = NameOptions::default();
    inheritable_name_attrs(&root, &mut style_name_options);
    warn_unsupported_name_attrs(&root, &mut w);

    let mut title = String::new();
    let mut id = String::new();
    let mut terms = TermSet::en_us();
    let mut macros: BTreeMap<String, Vec<Element>> = BTreeMap::new();
    let mut citation: Option<Layout> = None;
    let mut bibliography: Option<Option<Bibliography>> = None;

    for el in root.els() {
        match el.local() {
            "info" => {
                for c in el.els() {
                    match c.local() {
                        "title" if title.is_empty() => title = clamp(c.text().trim()),
                        "id" if id.is_empty() => id = clamp(c.text().trim()),
                        _ => {} // author, updated, link… metadata, ignored
                    }
                }
            }
            "locale" => parse_locale(el, &mut terms, &mut w),
            "macro" => {
                if macros.len() >= MAX_MACROS {
                    w.push(format!("more than {MAX_MACROS} macros; the rest are ignored"));
                } else {
                    let name = clamp(el.attr("name").unwrap_or("").trim());
                    if name.is_empty() {
                        w.push("a <macro> without a name is ignored".into());
                    } else if macros.insert(name.clone(), elements(el, &mut w)).is_some() {
                        w.push(format!("duplicate macro '{name}'; the last one wins"));
                    }
                }
            }
            "citation" => {
                if citation.is_some() {
                    w.push("more than one <citation>; the first is used".into());
                } else {
                    citation = parse_citation(el, &style_name_options, &mut w);
                }
            }
            "bibliography" => {
                if bibliography.is_some() {
                    w.push("more than one <bibliography>; the first is used".into());
                } else {
                    bibliography = Some(parse_bibliography(el, &style_name_options, &mut w));
                }
            }
            _ => {} // unknown top-level element, ignored
        }
    }

    let mut citation = citation.ok_or("not a CSL style: no <citation> with a <layout>")?;
    let mut bibliography = match bibliography {
        Some(b) => Some(b.ok_or("the <bibliography> has no <layout>")?),
        None => None,
    };
    resolve_macros(&mut macros, &mut citation, &mut bibliography, &mut w);
    check_term_references(&citation, &bibliography, &macros, &terms, &mut w);

    Ok(Style { id, title, locale_terms: terms, name_options: style_name_options, macros, citation, bibliography, warnings: w.into_vec() })
}

// ---------------------------------------------------------------------------
// Warnings and clamps

/// Warning collector: deduplicated and capped, so a hostile file can't grow it without bound.
#[derive(Default)]
struct Warnings {
    items: Vec<String>,
}

impl Warnings {
    fn push(&mut self, msg: String) {
        if self.items.len() < MAX_WARNINGS && !self.items.contains(&msg) {
            self.items.push(msg);
        }
    }
    fn into_vec(self) -> Vec<String> {
        self.items
    }
}

/// A carried string, clamped to [`MAX_TEXT_CHARS`] characters.
fn clamp(s: &str) -> String {
    s.chars().take(MAX_TEXT_CHARS).collect()
}

/// An attribute as a clamped string (empty when absent).
fn attr(el: &El, name: &str) -> String {
    clamp(el.attr(name).unwrap_or(""))
}

/// A whitespace-separated attribute value as a clamped, count-bounded list; entries past
/// `max` are dropped with a warning naming what was cut.
fn list(el: &El, name: &str, max: usize, what: &str, w: &mut Warnings) -> Vec<String> {
    let mut all: Vec<String> = el.attr(name).unwrap_or("").split_whitespace().take(max + 1).map(clamp).collect();
    if all.len() > max {
        all.truncate(max);
        w.push(format!("a {what} lists more than {max} values; the rest are ignored"));
    }
    all
}

// ---------------------------------------------------------------------------
// Style-level attributes

/// Attributes of the root that load but are not rendered.
fn warn_style_attrs(root: &El, w: &mut Warnings) {
    if root.attr("class") == Some("note") {
        w.push("note-class styles (footnote citations) render as in-text citations".into());
    }
    if let Some(v) = root.attr("version")
        && !v.starts_with("1.")
    {
        w.push(format!("CSL version '{v}' is not 1.x; parsed as 1.0.2 anyway"));
    }
    if let Some(l) = root.attr("default-locale")
        && !is_en_us(l)
    {
        w.push(format!("only the en-US locale is supported; default-locale '{l}' is ignored"));
    }
    if root.attr("page-range-format").is_some() {
        w.push("page-range-format is not supported; page ranges render as written".into());
    }
    if root.attr("demote-non-dropping-particle").is_some() {
        w.push("demote-non-dropping-particle is not supported".into());
    }
    if root.attr("initialize-with-hyphen").is_some() {
        w.push("initialize-with-hyphen is not supported".into());
    }
    if root.attr("names-delimiter").is_some() {
        w.push("style-level names-delimiter (between name variables) is not supported".into());
    }
}

/// A locale is supported when it is (or falls back to) en-US.
fn is_en_us(lang: &str) -> bool {
    lang.is_empty() || lang.eq_ignore_ascii_case("en") || lang.eq_ignore_ascii_case("en-us")
}

// ---------------------------------------------------------------------------
// Locale sections

/// One `<locale>`: en-US overrides are honored, anything else warns and is skipped.
fn parse_locale(el: &El, terms: &mut TermSet, w: &mut Warnings) {
    let lang = el.attr("lang").unwrap_or("");
    if !is_en_us(lang) {
        w.push(format!("only the en-US locale is supported; locale '{lang}' is ignored"));
        return;
    }
    for c in el.els() {
        match c.local() {
            "terms" => {
                for t in c.els() {
                    if t.local() == "term" {
                        parse_term(t, terms, w);
                    }
                }
            }
            "date" => w.push("locale date formats are not supported; dates render from their parts".into()),
            "style-options" => w.push("locale style-options are not supported".into()),
            _ => {}
        }
    }
}

/// One `<term>`: direct text, or `<single>`/`<multiple>` children.
fn parse_term(t: &El, terms: &mut TermSet, w: &mut Warnings) {
    let name = clamp(t.attr("name").unwrap_or("").trim());
    if name.is_empty() {
        return;
    }
    let form = t.attr("form").and_then(Form::parse).unwrap_or(Form::Long);
    if t.attr("match").is_some() || t.attr("gender").is_some() || t.attr("gender-form").is_some() {
        w.push("term grammar transforms (match/gender) are not supported".into());
    }
    let mut single = String::new();
    let mut multiple = String::new();
    for c in t.els() {
        match c.local() {
            "single" => single = clamp(c.text().trim()),
            "multiple" => multiple = clamp(c.text().trim()),
            _ => {}
        }
    }
    if single.is_empty() && multiple.is_empty() {
        single = clamp(t.text().trim());
    }
    if single.is_empty() {
        single = multiple.clone();
    }
    if multiple.is_empty() {
        multiple = single.clone();
    }
    terms.set(&name, form, crate::terms::Term { single, multiple });
}

// ---------------------------------------------------------------------------
// Citation and bibliography

/// The inheritable name options `<style>`, `<citation>`, `<bibliography>`, `<names>` and
/// `<name>` share (the spec's Inheritable Name Options), read into `options`.
fn inheritable_name_attrs(el: &El, options: &mut NameOptions) {
    if let Some(v) = el.attr("name-as-sort-order") {
        options.name_as_sort_order = match v {
            "all" => Some(NameAsSortOrder::All),
            "first" => Some(NameAsSortOrder::First),
            _ => None,
        };
    }
    if let Some(v) = el.attr("initialize-with") {
        options.initialize_with = Some(clamp(v));
    }
    if let Some(v) = el.attr("et-al-min") {
        options.et_al_min = v.parse().ok();
    }
    if let Some(v) = el.attr("et-al-use-first") {
        options.et_al_use_first = v.parse().ok();
    }
    if let Some(v) = el.attr("and") {
        options.and = match v {
            "text" => Some(AndStyle::Text),
            "symbol" => Some(AndStyle::Symbol),
            _ => None,
        };
    }
    if let Some(v) = el.attr("name-delimiter") {
        options.delimiter = Some(clamp(v));
    }
}

/// Name-option semantics outside the subset, wherever they appear (root, section,
/// `<names>`, `<name>`): warned, never silently changed.
fn warn_unsupported_name_attrs(el: &El, w: &mut Warnings) {
    if let Some(v) = el.attr("form") {
        w.push(format!("name form '{v}' is not supported; names render in full"));
    }
    if el.attr("name-form").is_some() {
        w.push("name-form is not supported; names render in full".into());
    }
    if el.attr("et-al-use-last").is_some() {
        w.push("et-al-use-last is not supported".into());
    }
    if let Some(v) = el.attr("sort-separator")
        && v != ", "
    {
        w.push("sort-separator other than ', ' is not supported".into());
    }
    if el.attr("delimiter-precedes-last").is_some() || el.attr("delimiter-precedes-et-al").is_some() {
        w.push("delimiter-precedes-last/delimiter-precedes-et-al rules are not supported".into());
    }
}

/// `<citation>`: its layout (with the section's name options merged over the root's), or
/// `None` when it has none (the caller refuses the style).
fn parse_citation(el: &El, root: &NameOptions, w: &mut Warnings) -> Option<Layout> {
    if el.attr("collapse").is_some() {
        w.push("citation collapsing (collapse) is not supported; citations render expanded".into());
    }
    if el.attrs.iter().any(|(k, _)| k.starts_with("disambiguate") || k == "givenname-disambiguation-rule") {
        w.push("citation disambiguation (disambiguate-…) is not supported".into());
    }
    if el.attr("near-note-distance").is_some() {
        w.push("near-note is not supported".into());
    }
    let mut options = NameOptions::default();
    inheritable_name_attrs(el, &mut options);
    warn_unsupported_name_attrs(el, w);
    let mut layout = section_layout(el, "citation", w)?;
    layout.name_options = options.with_defaults(root);
    Some(layout)
}

/// `<bibliography>`: its layout (with the section's name options merged over the root's)
/// and sort keys, or `None` when it has no layout.
fn parse_bibliography(el: &El, root: &NameOptions, w: &mut Warnings) -> Option<Bibliography> {
    if el.attr("second-field-align").is_some() {
        w.push("bibliography second-field-align is not supported".into());
    }
    if ["hanging-indent", "entry-spacing", "line-spacing", "subsequent-author-substitute"].iter().any(|a| el.attr(a).is_some()) {
        w.push("bibliography formatting (hanging-indent, entry-spacing, line-spacing, subsequent-author-substitute) is not supported".into());
    }
    let mut options = NameOptions::default();
    inheritable_name_attrs(el, &mut options);
    warn_unsupported_name_attrs(el, w);
    let mut sort = Vec::new();
    let mut layout = None;
    for c in el.els() {
        match c.local() {
            "sort" => sort = sort_keys(c, w),
            "layout" if layout.is_none() => layout = Some(parse_layout(c, w)),
            _ => {}
        }
    }
    layout.map(|mut layout| {
        layout.name_options = options.with_defaults(root);
        Bibliography { layout, sort }
    })
}

/// `<sort>`: keys in order.
fn sort_keys(el: &El, w: &mut Warnings) -> Vec<SortKey> {
    let mut keys = Vec::new();
    for c in el.els() {
        if c.local() != "key" {
            continue;
        }
        let field = if let Some(v) = c.attr("variable") {
            SortField::Variable(clamp(v))
        } else if let Some(m) = c.attr("macro") {
            SortField::Macro(clamp(m))
        } else {
            continue;
        };
        let direction = match c.attr("sort") {
            Some("descending") => SortDirection::Descending,
            _ => SortDirection::Ascending,
        };
        if ["names-min", "names-use-first", "names-use-last"].iter().any(|a| c.attr(a).is_some()) {
            w.push("sort key name options (names-min/…) are not supported".into());
        }
        keys.push(SortKey { field, direction });
    }
    keys
}

/// The (first) `<layout>` child of a section, warning about sorting where the model can't
/// carry it and about extra layouts.
fn section_layout(el: &El, section: &str, w: &mut Warnings) -> Option<Layout> {
    let mut layout: Option<Layout> = None;
    for c in el.els() {
        match c.local() {
            "sort" => w.push(format!("{section} sorting is not supported")),
            "layout" => {
                if layout.is_none() {
                    layout = Some(parse_layout(c, w));
                } else {
                    w.push(format!("more than one <layout> in <{section}>; the first is used"));
                }
            }
            _ => {}
        }
    }
    layout
}

// ---------------------------------------------------------------------------
// Rendering elements

/// The rendering elements among an element's children (unknown children are ignored).
fn elements(parent: &El, w: &mut Warnings) -> Vec<Element> {
    parent.els().filter_map(|c| element(c, w)).collect()
}

/// One rendering element, or `None` for unknown/unusable children.
fn element(el: &El, w: &mut Warnings) -> Option<Element> {
    warn_formatting_attrs(el, w);
    match el.local() {
        "text" => text_el(el, w).map(Element::Text),
        "number" => number_el(el, w).map(Element::Number),
        "date" => date_el(el, w).map(Element::Date),
        "names" => names_el(el, w).map(Element::Names),
        "label" => label_el(el, w).map(Element::Label),
        "group" => Some(Element::Group(Group {
            elements: elements(el, w),
            delimiter: attr(el, "delimiter"),
            prefix: attr(el, "prefix"),
            suffix: attr(el, "suffix"),
        })),
        "choose" => Some(Element::Choose(choose(el, w))),
        _ => None,
    }
}

/// Formatting attributes every rendering element may carry, none of which the model keeps.
const FORMATTING_ATTRS: &[&str] =
    &["font-style", "font-variant", "font-weight", "text-decoration", "vertical-align", "text-case", "display", "quotes"];

/// Warn once about formatting attributes the renderer ignores.
fn warn_formatting_attrs(el: &El, w: &mut Warnings) {
    if el.attrs.iter().any(|(k, _)| FORMATTING_ATTRS.contains(&k.as_str())) {
        w.push("formatting attributes (font-style, font-weight, text-case, quotes, …) are ignored".into());
    }
}

/// `<text>`: exactly one of variable/macro/term/value (in that precedence when several).
fn text_el(el: &El, w: &mut Warnings) -> Option<Text> {
    let present: Vec<&str> = ["variable", "macro", "term", "value"].iter().filter(|a| el.attr(a).is_some()).copied().collect();
    let Some(first) = present.first() else {
        w.push("a <text> without variable/macro/term/value is ignored".into());
        return None;
    };
    if present.len() > 1 {
        w.push(format!("a <text> with several of variable/macro/term/value uses '{first}' and ignores the rest"));
    }
    let value = clamp(el.attr(first).unwrap_or(""));
    let source = match *first {
        "variable" => TextSource::Variable(value),
        "macro" => TextSource::Macro(value),
        "term" => TextSource::Term(value),
        _ => TextSource::Value(value),
    };
    Some(Text {
        source,
        form: el.attr("form").and_then(Form::parse),
        plural: match el.attr("plural") {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        },
        prefix: attr(el, "prefix"),
        suffix: attr(el, "suffix"),
        strip_periods: el.attr("strip-periods") == Some("true"),
    })
}

/// `<number>`.
fn number_el(el: &El, w: &mut Warnings) -> Option<Number> {
    let variable = attr(el, "variable");
    if variable.is_empty() {
        w.push("a <number> without a variable is ignored".into());
        return None;
    }
    if el.attr("strip-periods").is_some() {
        w.push("strip-periods on <number> is not supported".into());
    }
    Some(Number { variable, form: el.attr("form").and_then(Form::parse), prefix: attr(el, "prefix"), suffix: attr(el, "suffix") })
}

/// `<date>` with its `<date-part>` children.
fn date_el(el: &El, w: &mut Warnings) -> Option<Date> {
    let variable = attr(el, "variable");
    if variable.is_empty() {
        w.push("a <date> without a variable is ignored".into());
        return None;
    }
    let mut parts = Vec::new();
    for c in el.els() {
        if c.local() != "date-part" {
            continue;
        }
        let name = match c.attr("name") {
            Some("year") => DatePartName::Year,
            Some("month") => DatePartName::Month,
            Some("day") => DatePartName::Day,
            _ => continue, // a bogus part name, ignored
        };
        if c.attr("range-delimiter").is_some() {
            w.push("date range-delimiter is not supported".into());
        }
        parts.push(DatePart { name, form: c.attr("form").and_then(Form::parse), prefix: attr(c, "prefix"), suffix: attr(c, "suffix") });
    }
    Some(Date {
        variable,
        form: el.attr("form").and_then(Form::parse),
        date_parts: el.attr("date-parts").and_then(DateParts::parse).unwrap_or_default(),
        parts,
        prefix: attr(el, "prefix"),
        suffix: attr(el, "suffix"),
    })
}

/// `<names>`: variables, options (from the element merged with its `<name>` child), label,
/// substitute.
fn names_el(el: &El, w: &mut Warnings) -> Option<Names> {
    let variables = list(el, "variable", MAX_NAME_VARIABLES, "names variable list", w);
    if variables.is_empty() {
        w.push("a <names> without variables is ignored".into());
        return None;
    }
    let mut options = NameOptions::default();
    apply_name_attrs_at(el, &mut options, w, true);
    let mut with_label = None;
    let mut label_precedes_names = false;
    let mut substitute = None;
    let mut seen_name = false;
    for c in el.els() {
        match c.local() {
            "name" => {
                seen_name = true;
                apply_name_attrs(c, &mut options, w);
            }
            "institution" => w.push("<institution> (institutional names) is not supported".into()),
            "label" => {
                if with_label.is_none() {
                    with_label = Some(names_label(c));
                    label_precedes_names = !seen_name;
                }
            }
            "substitute" if substitute.is_none() => substitute = Some(elements(c, w)),
            _ => {}
        }
    }
    Some(Names { variables, options, with_label, label_precedes_names, substitute })
}

/// A `<label>` inside `<names>`: no variable of its own (it labels the names themselves).
fn names_label(el: &El) -> Label {
    let mut l = label_common(el);
    l.variable = String::new();
    l
}

/// `<label>`: the label of a variable.
fn label_el(el: &El, w: &mut Warnings) -> Option<Label> {
    let l = label_common(el);
    if l.variable.is_empty() {
        w.push("a <label> without a variable is ignored".into());
        return None;
    }
    Some(l)
}

/// Label fields shared by both placements.
fn label_common(el: &El) -> Label {
    Label {
        variable: attr(el, "variable"),
        form: el.attr("form").and_then(Form::parse),
        plural: match el.attr("plural") {
            Some("always") => Some(Plural::Always),
            Some("never") => Some(Plural::Never),
            Some("contextual") => Some(Plural::Contextual),
            _ => None, // the spec default is contextual
        },
        prefix: attr(el, "prefix"),
        suffix: attr(el, "suffix"),
    }
}

/// Name options from a `<names>` element or a `<name>` child (the child wins where both
/// set them): the inheritable attributes plus the plain `delimiter` a `<name>` carries (in
/// CSL 1.0.2 a `delimiter` on `<names>` itself separates the name *variables* — editors from
/// translators — which this renderer doesn't do, so it warns instead; between names the
/// delimiter comes from `<name delimiter>` or the inherited `name-delimiter`). Attributes
/// outside the subset warn.
fn apply_name_attrs(el: &El, options: &mut NameOptions, w: &mut Warnings) {
    apply_name_attrs_at(el, options, w, false);
}

/// [`apply_name_attrs`] with `names_element` saying which element this is.
fn apply_name_attrs_at(el: &El, options: &mut NameOptions, w: &mut Warnings, names_element: bool) {
    inheritable_name_attrs(el, options);
    if let Some(v) = el.attr("delimiter") {
        if names_element {
            w.push(
                "a <names> delimiter separates the name variables, which is not supported; use <name delimiter> or name-delimiter between names"
                    .into(),
            );
        } else {
            options.delimiter = Some(clamp(v));
        }
    }
    warn_unsupported_name_attrs(el, w);
}

/// `<choose>`: if/else-if branches (with their conditions) and an optional else.
fn choose(el: &El, w: &mut Warnings) -> Choose {
    let mut branches = Vec::new();
    let mut else_branch = None;
    for c in el.els() {
        match c.local() {
            "if" | "else-if" => branches.push(branch(c, w)),
            "else" if else_branch.is_none() => else_branch = Some(parse_layout(c, w)),
            _ => {}
        }
    }
    Choose { branches, else_branch }
}

/// One `<if>`/`<else-if>`: the `match` mode, its conditions, and its rendering body.
fn branch(el: &El, w: &mut Warnings) -> Branch {
    let mode = match el.attr("match") {
        Some("any") => Match::Any,
        Some("none") => Match::None,
        _ => Match::All, // the spec default
    };
    let mut conditions = Vec::new();
    for name in list(el, "variable", MAX_CONDITION_VALUES, "condition", w) {
        conditions.push(Condition::Variable { name, set: true });
    }
    let types = list(el, "type", MAX_CONDITION_VALUES, "condition", w);
    if !types.is_empty() {
        conditions.push(Condition::Type { names: types });
    }
    for value in list(el, "position", MAX_CONDITION_VALUES, "condition", w) {
        match value.as_str() {
            "first" => conditions.push(Condition::Position { value: Position::First }),
            "subsequent" => conditions.push(Condition::Position { value: Position::Subsequent }),
            "ibid" => conditions.push(Condition::Position { value: Position::Ibid }),
            "ibid-with-locator" => {
                w.push("position 'ibid-with-locator' is treated as 'ibid'".into());
                conditions.push(Condition::Position { value: Position::Ibid });
            }
            "near-note" => w.push("near-note is not supported".into()),
            _ => {}
        }
    }
    if ["is-numeric", "is-uncertain-date", "locator"].iter().any(|a| el.attr(a).is_some()) {
        w.push("choose conditions is-numeric, is-uncertain-date and locator are not supported".into());
    }
    Branch { mode, conditions, layout: parse_layout(el, w) }
}

/// A `<layout>` (or `<else>`) body.
fn parse_layout(el: &El, w: &mut Warnings) -> Layout {
    warn_formatting_attrs(el, w);
    Layout {
        elements: elements(el, w),
        prefix: attr(el, "prefix"),
        suffix: attr(el, "suffix"),
        delimiter: attr(el, "delimiter"),
        name_options: NameOptions::default(),
    }
}

// ---------------------------------------------------------------------------
// Macro resolution

/// Resolve macro references at parse time: drop references to unknown macros, and cut
/// references whose expansion would exceed [`MAX_MACRO_RECURSION`] levels (never a hang).
fn resolve_macros(macros: &mut BTreeMap<String, Vec<Element>>, citation: &mut Layout, bibliography: &mut Option<Bibliography>, w: &mut Warnings) {
    // References to macros that don't exist.
    let mut all_refs: Vec<String> = Vec::new();
    collect_tree_refs(citation, &mut all_refs);
    if let Some(b) = bibliography.as_ref() {
        collect_tree_refs(&b.layout, &mut all_refs);
    }
    for body in macros.values() {
        referenced_macros(body, &mut all_refs);
    }
    let unknown: BTreeSet<String> = all_refs.iter().filter(|m| !macros.contains_key(*m)).cloned().collect();
    for name in &unknown {
        w.push(format!("unknown macro '{name}' is ignored"));
    }

    // Expansion depth, by fixpoint: each round grows every macro's depth by the depths it
    // references, so cycles (and only cycles or genuinely deep chains) keep growing past
    // the cut. Iterative, never recursive: hostile macro graphs can't stall this.
    let rounds = macros.len().max(MAX_MACRO_RECURSION as usize + 1);
    let mut depth: BTreeMap<&str, u32> = macros.keys().map(|k| (k.as_str(), 1)).collect();
    for _ in 0..rounds {
        let mut next: BTreeMap<&str, u32> = BTreeMap::new();
        for (name, body) in macros.iter() {
            let mut refs = Vec::new();
            referenced_macros(body, &mut refs);
            let inner = refs.iter().filter_map(|r| depth.get(r.as_str()).copied()).max().unwrap_or(0);
            next.insert(name.as_str(), inner.saturating_add(1));
        }
        depth = next;
    }
    let bad: BTreeSet<String> = depth.into_iter().filter(|(_, d)| *d > MAX_MACRO_RECURSION).map(|(name, _)| name.to_string()).collect();
    for name in &bad {
        w.push(format!("macro '{name}' expands deeper than {MAX_MACRO_RECURSION} levels (a cycle?); references to it are dropped"));
    }

    // Drop the offending references everywhere: they render as nothing.
    let drop_set: BTreeSet<&str> = unknown.iter().map(String::as_str).chain(bad.iter().map(String::as_str)).collect();
    let clear = |els: &mut Vec<Element>| {
        walk_elements(els, &mut |e| {
            if let Element::Text(t) = e
                && let TextSource::Macro(m) = &t.source
                && drop_set.contains(m.as_str())
            {
                t.source = TextSource::Value(String::new());
            }
        });
    };
    clear(&mut citation.elements);
    if let Some(b) = bibliography.as_mut() {
        clear(&mut b.layout.elements);
    }
    for body in macros.values_mut() {
        clear(body);
    }
}

/// Macro references of a layout tree (its body plus branch/else/substitute descendants).
fn collect_tree_refs(layout: &Layout, out: &mut Vec<String>) {
    referenced_macros(&layout.elements, out);
}

/// Term references the merged en-US term set (defaults plus the style's overrides) can't
/// resolve would render empty: surface them instead. `<text term="…">` and
/// `<label variable="…">` both draw on terms; a names' own `<label>` (no variable) labels
/// the names' role and is checked by the renderer's variables, not here.
fn check_term_references(
    citation: &Layout,
    bibliography: &Option<Bibliography>,
    macros: &BTreeMap<String, Vec<Element>>,
    terms: &TermSet,
    w: &mut Warnings,
) {
    let mut term_refs: Vec<(String, Option<Form>)> = Vec::new();
    let mut label_refs: Vec<(String, Option<Form>)> = Vec::new();
    let mut collect = |els: &[Element]| {
        for_each_element(els, &mut |e| match e {
            Element::Text(t) => {
                if let TextSource::Term(name) = &t.source {
                    term_refs.push((name.clone(), t.form));
                }
            }
            Element::Label(l) if !l.variable.is_empty() => {
                label_refs.push((l.variable.clone(), l.form));
            }
            _ => {}
        });
    };
    collect(&citation.elements);
    if let Some(b) = bibliography {
        collect(&b.layout.elements);
    }
    for body in macros.values() {
        collect(body);
    }
    for (name, form) in &term_refs {
        if terms.get(name, form.unwrap_or(Form::Long)).is_none() {
            w.push(format!("term '{name}' is not in the en-US locale (or the style's overrides); it renders empty"));
        }
    }
    for (name, form) in &label_refs {
        // Label variables the renderer resolves itself: `locator` is dynamic (CSL resolves
        // it to the locator type's term, the page term over the cited pages here), and the
        // number variables without a locale term of their own render no label at all.
        if matches!(name.as_str(), "locator" | "citation-number" | "chapter-number" | "first-reference-note-number") {
            continue;
        }
        if terms.get(name, form.unwrap_or(Form::Long)).is_none() {
            w.push(format!("label variable '{name}' is not in the en-US locale (or the style's overrides); it renders empty"));
        }
    }
}
