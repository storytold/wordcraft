//! The CSL style model: what a renderer consumes.
//!
//! This is the subset of CSL 1.0.2 (docs.citationstyles.org, a public spec) that WordCraft
//! renders, written from that spec. Each shape maps to one CSL element; the parser
//! ([`crate::parse`]) fills it leniently and reports anything it had to drop in
//! [`Style::warned`] — nothing silently changes meaning.
//!
//! Conventions: absent attributes become `None`/`false`/empty strings (the spec defaults the
//! renderer applies); strings are already clamped to a sane length by the parser; macro
//! references ([`TextSource::Macro`]) are resolved at parse time, so every surviving
//! reference points at a macro whose expansion is bounded (see `parse`).

use std::collections::BTreeMap;

use crate::terms::TermSet;

/// A parsed CSL style file.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// `<info><id>` — the style's identifying URI (empty when the file has none).
    pub id: String,
    /// `<info><title>`.
    pub title: String,
    /// The en-US terms the renderer may draw on: the CSL 1.0.2 spec appendix defaults merged
    /// with the style's own `xml:lang="en-US"` overrides.
    pub locale_terms: TermSet,
    /// The inheritable name options set on the root `<style>`: the defaults every
    /// `<names>` in the style renders under (section and element options win).
    pub name_options: NameOptions,
    /// `<macro name="…">` bodies, by name.
    pub macros: BTreeMap<String, Vec<Element>>,
    /// The `<citation><layout>`; a style without one is refused by the parser.
    pub citation: Layout,
    /// The `<bibliography>`, when the style has one.
    pub bibliography: Option<Bibliography>,
    /// Human-readable notes about constructs outside the subset (deduplicated, capped).
    pub warnings: Vec<String>,
}

impl Style {
    /// Warnings about unsupported constructs the parser loaded anyway.
    pub fn warned(&self) -> &[String] {
        &self.warnings
    }
}

/// The bibliography section: a required layout plus its sort keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Bibliography {
    pub layout: Layout,
    /// `<sort>` keys in order (empty when the section has no `<sort>`).
    pub sort: Vec<SortKey>,
}

/// One `<sort><key/>`: a variable or macro to sort by, and a direction.
#[derive(Clone, Debug, PartialEq)]
pub struct SortKey {
    pub field: SortField,
    pub direction: SortDirection,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SortField {
    Variable(String),
    Macro(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// A rendering sequence with affixes: `<layout>` bodies (citation, bibliography, choose
/// branches) and `<else>` bodies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub elements: Vec<Element>,
    pub prefix: String,
    pub suffix: String,
    pub delimiter: String,
    /// The inheritable name options set on the `<citation>`/`<bibliography>` section,
    /// merged over the root's; branch and else layouts leave this default (the options
    /// in scope reach them through the section they render in).
    pub name_options: NameOptions,
}

/// Every rendering element of the subset.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Text(Text),
    Number(Number),
    Date(Date),
    Names(Names),
    Label(Label),
    Group(Group),
    Choose(Choose),
}

/// One value of CSL's many `form` attributes, as written in the file. Each element accepts
/// the subset that makes sense for it (`Label` takes the term forms, `Number` the number
/// forms, date parts the date-part forms); the parser keeps what it can map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Form {
    Long,
    Short,
    Verb,
    VerbShort,
    Symbol,
    Numeric,
    NumericLeadingZeros,
    Roman,
    Ordinal,
    Text,
}

impl Form {
    /// The attribute value as written, or `None` when it isn't a CSL form.
    pub fn parse(v: &str) -> Option<Form> {
        Some(match v {
            "long" => Form::Long,
            "short" => Form::Short,
            "verb" => Form::Verb,
            "verb-short" => Form::VerbShort,
            "symbol" => Form::Symbol,
            "numeric" => Form::Numeric,
            "numeric-leading-zeros" => Form::NumericLeadingZeros,
            "roman" => Form::Roman,
            "ordinal" => Form::Ordinal,
            "text" => Form::Text,
            _ => return None,
        })
    }
}

/// Where a `<text>` reads from: exactly one of `variable`/`macro`/`term`/`value`.
#[derive(Clone, Debug, PartialEq)]
pub enum TextSource {
    Variable(String),
    /// A macro reference, resolved at parse time (existing, bounded expansion).
    Macro(String),
    Term(String),
    Value(String),
}

/// `<text>`: a variable, macro, term or literal value.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub source: TextSource,
    pub form: Option<Form>,
    /// `plural` on a term: `Some(true)`/`Some(false)` force the multiple/single shape
    /// (the spec's values for `text term`; the default is the singular).
    pub plural: Option<bool>,
    pub prefix: String,
    pub suffix: String,
    /// `strip-periods="true"`: periods removed from the rendered text.
    pub strip_periods: bool,
}

/// `<number>`: a number variable (page, volume, issue…).
#[derive(Clone, Debug, PartialEq)]
pub struct Number {
    pub variable: String,
    pub form: Option<Form>,
    pub prefix: String,
    pub suffix: String,
}

/// `<date>`: a date variable rendered through its parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Date {
    pub variable: String,
    /// `form="text"`/`form="numeric"` selects a localized date format; explicit `parts`
    /// render directly.
    pub form: Option<Form>,
    /// `date-parts`: which parts the localized forms render (explicit `parts` render as
    /// written). The spec default renders everything.
    pub date_parts: DateParts,
    pub parts: Vec<DatePart>,
    pub prefix: String,
    pub suffix: String,
}

/// The `date-parts` attribute of a localized date: which of the present parts its form
/// renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateParts {
    Year,
    YearMonth,
    #[default]
    YearMonthDay,
}

impl DateParts {
    /// The attribute value as written, or `None` when it isn't a `date-parts` value.
    pub fn parse(v: &str) -> Option<DateParts> {
        Some(match v {
            "year" => DateParts::Year,
            "year-month" => DateParts::YearMonth,
            "year-month-day" => DateParts::YearMonthDay,
            _ => return None,
        })
    }
}

/// `<date-part>`: one component of a [`Date`].
#[derive(Clone, Debug, PartialEq)]
pub struct DatePart {
    pub name: DatePartName,
    pub form: Option<Form>,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePartName {
    Year,
    Month,
    Day,
}

/// `<names>`: one or more name variables, with name rendering options, an optional label
/// and an optional substitute chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Names {
    /// e.g. `["author", "editor"]`.
    pub variables: Vec<String>,
    /// Name options from the `<names>` element merged with its `<name>` child (child wins).
    pub options: NameOptions,
    /// The `<label>` child, rendered with the names (`with-label`) — before or after them
    /// as the style wrote it ([`Names::label_precedes_names`]).
    pub with_label: Option<Label>,
    /// Whether the `<label>` child was written before the `<name>` child (label-first
    /// styles render "Edited by Smith", not "Smith (ed.)").
    pub label_precedes_names: bool,
    /// The `<substitute>` children, tried in order when the variables are empty.
    pub substitute: Option<Vec<Element>>,
}

/// The name rendering options the subset carries. Anything else a style sets on
/// `<name>`/`<names>` (forms, sort separators, delimiter rules) warns instead of silently
/// changing rendered names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameOptions {
    /// `name-as-sort-order`: render names as "family, given" for all or only the first name.
    pub name_as_sort_order: Option<NameAsSortOrder>,
    /// `initialize-with`: the string added after each given-name initial (e.g. `". "`).
    pub initialize_with: Option<String>,
    /// `et-al-min`: from this many names on, truncate with the et-al term.
    pub et_al_min: Option<u32>,
    /// `et-al-use-first`: how many names to keep before truncating.
    pub et_al_use_first: Option<u32>,
    /// `and`: how to join the last two names.
    pub and: Option<AndStyle>,
    /// `delimiter`: the separator between names (default ", ").
    pub delimiter: Option<String>,
}

impl NameOptions {
    /// These options rendering under inherited `defaults`: every option left unset falls
    /// back to the inherited value (the spec's inheritable name options — root, then
    /// section, then `<names>`, then `<name>`, each more specific level winning).
    pub fn with_defaults(&self, defaults: &NameOptions) -> NameOptions {
        NameOptions {
            name_as_sort_order: self.name_as_sort_order.or(defaults.name_as_sort_order),
            initialize_with: self.initialize_with.clone().or_else(|| defaults.initialize_with.clone()),
            et_al_min: self.et_al_min.or(defaults.et_al_min),
            et_al_use_first: self.et_al_use_first.or(defaults.et_al_use_first),
            and: self.and.or(defaults.and),
            delimiter: self.delimiter.clone().or_else(|| defaults.delimiter.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameAsSortOrder {
    All,
    First,
}

/// `and="text"`/`and="symbol"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AndStyle {
    Text,
    Symbol,
}

/// `<label>`: the label of a variable (page → "p."/"pp.").
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub variable: String,
    pub form: Option<Form>,
    /// `plural`; `None` is the spec default, "contextual".
    pub plural: Option<Plural>,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    Always,
    Never,
    Contextual,
}

/// `<group>`: children joined by `delimiter`, dropped together when they all render empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub elements: Vec<Element>,
    pub delimiter: String,
    pub prefix: String,
    pub suffix: String,
}

/// `<choose>`: if/else-if branches plus an optional else.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choose {
    pub branches: Vec<Branch>,
    /// The `<else>` body.
    pub else_branch: Option<Layout>,
}

/// One `<if>`/`<else-if>`: when `mode` holds over `conditions`, render `layout`.
/// An empty condition list never matches (the parser warns when an `if` only carried
/// conditions outside the subset).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Branch {
    /// `match`: how the conditions combine. `Match::None` matches when no condition holds.
    pub mode: Match,
    pub conditions: Vec<Condition>,
    pub layout: Layout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Match {
    #[default]
    All,
    Any,
    None,
}

/// One test inside a branch condition attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// `variable="…"`: the variable is non-empty (`set`) — `set: false` is the negated
    /// test, available to renderers that fold `match="none"` into the variables.
    Variable { name: String, set: bool },
    /// `type="book chapter"`: the item is one of these types.
    Type { names: Vec<String> },
    /// `position="first|subsequent|ibid"`: one value per condition.
    Position { value: Position },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    First,
    Subsequent,
    Ibid,
}
