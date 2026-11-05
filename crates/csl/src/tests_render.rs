//! Renderer tests: hand-written fixture styles (author-date, numeric, names/et-al,
//! choose/type branches, date forms, sort keys) rendered against hand-written sources,
//! with expected strings derived by hand from the style logic per the CSL 1.0.2 subset
//! this crate renders (layout affixes, group delimiter/empty rules, choose evaluation,
//! name options, date forms, label plurals, number forms, terms).

use crate::model::{Element, Position, Style};
use crate::parse;
use wordcraft_doc::Source;

fn style(xml: &str) -> Style {
    parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

/// A source with tag and kind set; tests fill the fields they render.
fn src(tag: &str, kind: &str) -> Source {
    Source { tag: tag.into(), kind: kind.into(), ..Source::default() }
}

// ---------------------------------------------------------------------------
// Fixtures

/// An author-date style: names (sort order, initials) + year in the citation, a locator
/// with its dynamic label, and a bibliography sorted by the author macro then issued.
const AUTHOR_DATE: &str = r#"<style version="1.0">
  <macro name="author">
    <names variable="author">
      <name name-as-sort-order="all" initialize-with=". "/>
    </names>
  </macro>
  <macro name="issued">
    <date variable="issued"><date-part name="year"/></date>
  </macro>
  <citation>
    <layout prefix="(" suffix=")" delimiter="; ">
      <group delimiter=", ">
        <text macro="author"/>
        <text macro="issued"/>
        <group delimiter=" ">
          <label variable="locator" form="short"/>
          <text variable="locator"/>
        </group>
      </group>
    </layout>
  </citation>
  <bibliography>
    <sort>
      <key macro="author"/>
      <key variable="issued"/>
    </sort>
    <layout suffix=".">
      <group delimiter=" ">
        <text macro="author"/>
        <text macro="issued" prefix="(" suffix=")"/>
      </group>
    </layout>
  </bibliography>
</style>"#;

/// A numeric style: numbered cites, entries ordered by author, with type branches.
const NUMERIC: &str = r#"<style version="1.0">
  <citation>
    <layout prefix="[" suffix="]"><number variable="citation-number"/></layout>
  </citation>
  <bibliography>
    <sort><key variable="author"/></sort>
    <layout suffix=".">
      <number variable="citation-number" prefix="[" suffix="] "/>
      <group delimiter=", ">
        <text variable="title"/>
        <choose>
          <if type="book"><text variable="publisher" prefix="(" suffix=")"/></if>
          <else-if type="article-journal">
            <group delimiter=" ">
              <text variable="container-title"/>
              <date variable="issued" form="numeric" prefix="(" suffix=")"/>
            </group>
          </else-if>
          <else><text variable="URL" prefix="[" suffix="]"/></else>
        </choose>
      </group>
    </layout>
  </bibliography>
</style>"#;

/// Names options: first-name sort order, initials, and-form, delimiter, et-al truncation.
const NAMES_ET_AL: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <names variable="author">
        <name name-as-sort-order="first" initialize-with=". " and="text" delimiter="; " et-al-min="3" et-al-use-first="2"/>
      </names>
    </layout>
  </citation>
</style>"#;

/// Editors in full sort order with a custom delimiter and the symbol and-form.
const EDITORS_SORTED: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <names variable="editor">
        <name name-as-sort-order="all" delimiter=" / " and="symbol"/>
      </names>
    </layout>
  </citation>
</style>"#;

/// Choose branches: none-match variables, types, any-match variables, positions.
const CHOOSE: &str = r#"<style version="1.0">
  <citation>
    <layout prefix="{" suffix="}">
      <choose>
        <if variable="author" match="none"><text value="NOAUTHOR"/></if>
        <else-if type="book"><text value="BOOK"/></else-if>
        <else-if variable="DOI URL" match="any"><text value="ONLINE"/></else-if>
        <else-if position="first"><text value="FIRST"/></else-if>
        <else-if position="subsequent ibid" match="any"><text value="LATER"/></else-if>
        <else><text value="OTHER"/></else>
      </choose>
    </layout>
  </citation>
  <bibliography>
    <layout><choose>
      <if position="first subsequent ibid" match="any"><text value="POSITION"/></if>
      <else><text value="NOPOSITION"/></else>
    </choose></layout>
  </bibliography>
</style>"#;

/// Date forms: explicit parts in style order (ordinal day, long/short month, short year)
/// and the localized numeric/text forms; `accessed` read from its yyyy-mm-dd shape.
const DATES: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <group delimiter=" | ">
        <date variable="issued">
          <date-part name="day" form="ordinal" suffix=" "/>
          <date-part name="month" form="long" suffix=" "/>
          <date-part name="year"/>
        </date>
        <date variable="issued" form="numeric"/>
        <date variable="issued" form="text"/>
        <date variable="issued">
          <date-part name="month" form="short" suffix=" "/>
          <date-part name="year" form="short"/>
        </date>
        <date variable="accessed" form="numeric"/>
      </group>
    </layout>
  </citation>
</style>"#;

/// Labels (locator-like and names labels) with singular/plural forms, and terms.
const LABELS: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <group delimiter="; ">
        <group>
          <label variable="page" form="short" suffix=" "/>
          <text variable="page"/>
        </group>
        <group>
          <label variable="volume" suffix=" "/>
          <number variable="volume" form="ordinal"/>
        </group>
        <group>
          <label variable="issue" form="short" plural="always" suffix=" "/>
          <number variable="issue"/>
        </group>
        <names variable="editor">
          <name and="symbol"/>
          <label form="short" prefix=" (" suffix=")"/>
        </names>
        <group delimiter=" ">
          <text term="editor" form="verb"/>
          <text term="and" form="symbol"/>
          <text term="no date" form="short"/>
        </group>
        <group>
          <label variable="edition" form="short" suffix=" "/>
          <number variable="edition"/>
        </group>
      </group>
    </layout>
  </citation>
</style>"#;

/// Bibliography sorting: descending year, then title; ties keep the caller's order.
const SORTED: &str = r#"<style version="1.0">
  <citation><layout><text variable="title"/></layout></citation>
  <bibliography>
    <sort>
      <key variable="issued" sort="descending"/>
      <key variable="title"/>
    </sort>
    <layout><number variable="citation-number" suffix=": "/><text variable="title"/></layout>
  </bibliography>
</style>"#;

/// No sort keys: the caller's order, numbered from it.
const UNSORTED: &str = r#"<style version="1.0">
  <citation><layout><text variable="title"/></layout></citation>
  <bibliography>
    <layout suffix="!"><text variable="title"/></layout>
  </bibliography>
</style>"#;

/// Affix rules: empty groups render nothing (affixes included), a cyclic macro is cut by
/// the parser and its affixes render nothing, layout affixes always render.
const AFFIX: &str = r#"<style version="1.0">
  <macro name="dead"><text macro="dead" prefix="[" suffix="]"/></macro>
  <citation>
    <layout prefix="(" suffix=")">
      <group prefix="[" suffix="]" delimiter=", ">
        <text variable="publisher"/>
        <number variable="volume"/>
      </group>
      <text macro="dead" prefix="(" suffix=")"/>
    </layout>
  </citation>
</style>"#;

/// Number forms and short years, fed hostile values.
const NUMBERS: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <group delimiter=", ">
        <number variable="volume" form="roman"/>
        <number variable="issue" form="numeric-leading-zeros"/>
        <number variable="page" form="ordinal"/>
        <date variable="issued">
          <date-part name="year" form="short"/>
        </date>
      </group>
    </layout>
  </citation>
</style>"#;

/// The spec's group-suppression rule: a group whose called variables are all empty is
/// suppressed with its descriptive text and labels (the "retrieved from URL" example
/// shape, and IEEE's volume/issue term groups through nested groups and macros).
const GROUP_SUPPRESS: &str = r#"<style version="1.0">
  <macro name="volume">
    <group delimiter=", ">
      <group delimiter=" ">
        <number variable="volume" form="numeric"/>
        <text term="volume" form="short"/>
      </group>
      <group delimiter=" ">
        <number variable="issue" form="numeric"/>
        <text term="issue" form="short"/>
      </group>
    </group>
  </macro>
  <citation>
    <layout>
      <group delimiter="; ">
        <group delimiter=" ">
          <text term="retrieved"/>
          <text term="from"/>
          <text variable="URL"/>
        </group>
        <group delimiter=", ">
          <text variable="title"/>
          <text macro="volume"/>
        </group>
      </group>
    </layout>
  </citation>
</style>"#;

/// A non-empty nested group counts as a non-empty variable: the outer group renders.
const NESTED_GROUP: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <group delimiter=" ">
        <group><text term="and" form="symbol"/></group>
        <text variable="note"/>
      </group>
    </layout>
  </citation>
</style>"#;

/// `date-parts` limits the localized date forms to fewer parts (APA 7's year-only dates);
/// explicit parts render as written.
const DATE_PARTS: &str = r#"<style version="1.0">
  <citation>
    <layout>
      <group delimiter=" | ">
        <date variable="issued" form="numeric"/>
        <date variable="issued" form="numeric" date-parts="year"/>
        <date variable="issued" form="numeric" date-parts="year-month"/>
        <date variable="issued" form="text" date-parts="year"/>
        <date variable="issued" date-parts="year">
          <date-part name="month" form="short" suffix=" "/>
          <date-part name="year"/>
        </date>
      </group>
    </layout>
  </citation>
</style>"#;

/// The APA shape: inheritable name options (root `initialize-with`, section et-al), a
/// `date-parts="year"` localized date, and an 18-entry type list.
const APA_SHAPE: &str = r#"<style version="1.0" class="in-text" initialize-with=". ">
  <macro name="author">
    <names variable="author">
      <name name-as-sort-order="all"/>
    </names>
  </macro>
  <citation et-al-min="3" et-al-use-first="1">
    <layout prefix="(" suffix=")">
      <group delimiter=", ">
        <text macro="author"/>
        <choose>
          <if type="article-journal article-magazine article-newspaper broadcast chapter dataset entry entry-dictionary entry-encyclopedia figure graphic interview legal_case legislation manuscript map musical_score motion-picture" match="any">
            <date variable="issued" form="numeric" date-parts="year"/>
          </if>
          <else>
            <text term="no date" form="short"/>
          </else>
        </choose>
      </group>
    </layout>
  </citation>
</style>"#;

// ---------------------------------------------------------------------------
// Tests

#[test]
fn group_suppression_hides_descriptive_text_of_empty_variables() {
    let s = style(GROUP_SUPPRESS);
    assert!(s.warned().is_empty(), "{:?}", s.warned());

    let mut online = src("online", "book");
    online.url = "https://example.com/x".into();
    online.title = "Shared Spaces".into();
    // URL set: the descriptive terms render with it; volume and issue empty: their term
    // groups vanish (they only accompanied the empty numbers), nested and via the macro.
    assert_eq!(s.cite(&online, Position::First, 1, ""), "retrieved from https://example.com/x; Shared Spaces");

    let mut print = src("print", "book");
    print.title = "Shared Spaces".into();
    print.volume = "5".into();
    print.issue = "2".into();
    // No URL: "retrieved from" disappears with it; the numbers carry their terms.
    assert_eq!(s.cite(&print, Position::First, 1, ""), "Shared Spaces, 5 vol., 2 no.");

    let bare = src("bare", "book");
    // Everything empty: every group suppresses, nothing renders.
    assert_eq!(s.cite(&bare, Position::First, 1, ""), "");
}

#[test]
fn a_nonempty_nested_group_counts_as_a_nonempty_variable() {
    let s = style(NESTED_GROUP);
    // The inner group rendered (its term has no variable to gate it): the outer group is
    // not suppressed even though `note` is empty.
    assert_eq!(s.cite(&src("n", "book"), Position::First, 1, ""), "&");
}

#[test]
fn date_parts_limits_the_localized_forms() {
    let s = style(DATE_PARTS);
    let mut d = src("d", "book");
    d.year = "2021".into();
    d.month = "3".into();
    d.day = "15".into();
    // Defaults render everything; date-parts narrows the localized forms; the explicit
    // parts render as written regardless (month-03 short is the locale's "Mar.").
    assert_eq!(s.cite(&d, Position::First, 1, ""), "3/15/2021 | 2021 | 3/2021 | 2021 | Mar. 2021");

    // A year-month date with a day set: the day drops out of the localized forms (the
    // text form's month-year join is the subset's ", ").
    let s2 = style(
        r#"<style version="1.0"><citation><layout>
        <date variable="issued" form="numeric" date-parts="year-month"/>
        <text value=" | "/>
        <date variable="issued" form="text" date-parts="year-month"/>
      </layout></citation></style>"#,
    );
    let mut ym = src("ym", "book");
    ym.year = "2021".into();
    ym.month = "3".into();
    ym.day = "15".into();
    assert_eq!(s2.cite(&ym, Position::First, 1, ""), "3/2021 | March, 2021");
}

#[test]
fn long_type_lists_dispatch_beyond_sixteen_entries() {
    // An APA-shaped 18-entry type list: the 18th entry (motion-picture) must still
    // dispatch — the kind map reverses into these spec type names.
    let types = "article-journal article-magazine article-newspaper broadcast chapter dataset entry entry-dictionary entry-encyclopedia figure graphic interview legal_case legislation manuscript map musical_score motion-picture";
    let s = parse(
        format!(
            r#"<style version="1.0"><citation><layout prefix="["><choose>
            <if type="{types}" match="any"><text value="T"/></if>
            <else><text value="OTHER"/></else>
          </choose></layout></citation></style>"#
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(s.cite(&src("film", "film"), Position::First, 1, ""), "[T");
    assert_eq!(s.cite(&src("other", "other"), Position::First, 1, ""), "[OTHER");
}

#[test]
fn inherited_name_options_render_the_apa_shape() {
    let s = style(APA_SHAPE);
    assert!(s.warned().is_empty(), "{:?}", s.warned());

    // Root initialize-with + element sort order + section et-al, all in one name render.
    let mut film = src("film", "film");
    film.author = "Kilns, Dana; Mora, Pia".into();
    film.year = "2021".into();
    film.month = "3".into();
    film.day = "15".into();
    // motion-picture is the type list's 18th entry; the date narrows to its year.
    assert_eq!(s.cite(&film, Position::First, 1, ""), "(Kilns, D., Mora, P., 2021)");

    // Three names hit the section's et-al-min, keeping one.
    let mut trio = src("trio", "film");
    trio.author = "Kilns, Dana; Mora, Pia; Tan, Wu".into();
    trio.year = "2021".into();
    assert_eq!(s.cite(&trio, Position::First, 1, ""), "(Kilns, D., et al., 2021)");

    // A type outside the list: the no-date term, with the element-won sort order.
    let mut misc = src("misc", "other");
    misc.author = "Tan, Wu".into();
    assert_eq!(s.cite(&misc, Position::First, 1, ""), "(Tan, W., n.d.)");

    // Element-level options win over the inherited ones.
    let s = parse(
        br#"<style version="1.0" initialize-with=". "><citation><layout>
        <names variable="author"><name name-as-sort-order="all" initialize-with=""/></names>
      </layout></citation></style>"#,
    )
    .unwrap();
    let mut who = src("who", "book");
    who.author = "Kilns, Dana".into();
    assert_eq!(s.cite(&who, Position::First, 1, ""), "Kilns, D");
}

#[test]
fn text_terms_honor_plural() {
    let s = parse(
        br#"<style version="1.0"><citation><layout>
        <text term="page" form="short"/>
        <text value=" | "/>
        <text term="page" form="short" plural="true"/>
        <text value=" | "/>
        <text term="page" form="short" plural="false"/>
        <text value=" | "/>
        <text term="editor"/>
        <text value=" | "/>
        <text term="editor" plural="true"/>
      </layout></citation></style>"#,
    )
    .unwrap();
    assert!(s.warned().is_empty(), "{:?}", s.warned());
    // plural="true" picks the term's multiple shape, "false" and the default the single.
    assert_eq!(s.cite(&src("x", "book"), Position::First, 1, ""), "p. | pp. | p. | editor | editors");
}

#[test]
fn a_names_label_renders_where_the_style_wrote_it() {
    // Label first: "Edited by Ann Lee & Kim Fox" (a name child follows the label).
    let s = parse(
        br#"<style version="1.0"><citation><layout>
        <names variable="editor">
          <label form="verb" suffix=" "/>
          <name and="symbol"/>
        </names>
      </layout></citation></style>"#,
    )
    .unwrap();
    let mut e = src("e", "book");
    e.editor = "Lee, Ann; Fox, Kim".into();
    assert_eq!(s.cite(&e, Position::First, 1, ""), "edited by Ann Lee & Kim Fox");
    // The position is carried into the model (the LABELS fixture pins label-after).
    let Element::Names(n) = &s.citation.elements[0] else { panic!("not names") };
    assert!(n.label_precedes_names);
}

#[test]
fn author_date_cites_and_entries() {
    let s = style(AUTHOR_DATE);
    assert!(s.warned().is_empty(), "{:?}", s.warned());

    let mut doe = src("doe2020", "book");
    doe.author = "Doe, Jane".into();
    doe.year = "2020".into();
    doe.title = "Frogs of the North".into();
    doe.publisher = "Pond Press".into();

    let mut roe = src("roe1999", "article");
    roe.author = "Roe, John; Roe, Ann".into();
    roe.year = "1999".into();
    roe.title = "A Study".into();
    roe.journal = "Nature".into();
    roe.volume = "5".into();
    roe.issue = "2".into();
    roe.pages = "10-20".into();

    let mut anon = src("anon2020", "website");
    anon.year = "2020".into();
    anon.title = "Anon Page".into();
    anon.url = "https://example.com".into();

    let mut alba = src("alba2005", "book");
    alba.author = "Alba, Maria".into();
    alba.year = "2005".into();
    alba.title = "Shorelines".into();

    // Cites: names in sort order with initials, the year, the cited pages with their
    // dynamic label (p. for one page, pp. for a range).
    assert_eq!(s.cite(&doe, Position::First, 1, "12"), "(Doe, J., 2020, p. 12)");
    assert_eq!(s.cite(&roe, Position::Subsequent, 2, "10-13"), "(Roe, J., Roe, A., 1999, pp. 10-13)");
    // No author, no locator: the empty children drop out of the group.
    assert_eq!(s.cite(&anon, Position::First, 3, ""), "(2020)");
    assert_eq!(s.cite(&alba, Position::Ibid, 4, ""), "(Alba, M., 2005)");

    // Entries: sorted by the author macro key (empty sorts first), then issued.
    let entries = s.entries(&[doe, roe, anon, alba]);
    let texts: Vec<&str> = entries.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(texts, ["(2020).", "Alba, M. (2005).", "Doe, J. (2020).", "Roe, J., Roe, A. (1999)."]);
    assert_eq!(entries[0].1, ["", "2020"]);
    assert_eq!(entries[1].1, ["alba, m.", "2005"]);
    assert_eq!(entries[2].1, ["doe, j.", "2020"]);
    assert_eq!(entries[3].1, ["roe, j., roe, a.", "1999"]);
}

#[test]
fn numeric_cites_numbering_and_type_branches() {
    let s = style(NUMERIC);

    let mut doe = src("doe2020", "book");
    doe.author = "Doe, Jane".into();
    doe.title = "Frogs of the North".into();
    doe.publisher = "Pond Press".into();

    let mut roe = src("roe1999", "article");
    roe.author = "Roe, John; Roe, Ann".into();
    roe.title = "A Study".into();
    roe.journal = "Nature".into();
    roe.year = "1999".into();
    roe.month = "5".into();
    roe.day = "3".into();

    let mut walt = src("walt2021", "website");
    walt.author = "Web, Walt".into();
    walt.title = "A Page".into();
    walt.url = "https://x.example".into();

    // The cite number comes from the caller (the bibliography order position).
    assert_eq!(s.cite(&doe, Position::First, 1, ""), "[1]");
    assert_eq!(s.cite(&roe, Position::Subsequent, 7, "4"), "[7]");

    // Entries reorder by the sort key (author) and number from that order.
    let entries = s.entries(&[roe, doe, walt]);
    let texts: Vec<&str> = entries.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(texts, ["[1] Frogs of the North, (Pond Press).", "[2] A Study, Nature (5/3/1999).", "[3] A Page, [https://x.example]."]);
    assert_eq!(entries[0].1, ["doe, jane"]);
    assert_eq!(entries[1].1, ["roe, john; roe, ann"]);
    assert_eq!(entries[2].1, ["web, walt"]);
}

#[test]
fn names_options_render() {
    let s = style(NAMES_ET_AL);
    let c = |author: &str| s.cite(&Source { author: author.into(), ..src("x", "book") }, Position::First, 1, "");
    // First name sorted with initials, the rest as given-family (initialize-with applies
    // to every given name, sorted or not); and-form joins the last two; et-al truncates
    // from three names, keeping two.
    assert_eq!(c("Doe, Jane"), "Doe, J.");
    assert_eq!(c("Doe, Jane; Roe, John"), "Doe, J. and J. Roe");
    assert_eq!(c("Doe, Jane; Roe, John; Smith, Sam"), "Doe, J.; J. Roe; et al.");
    assert_eq!(c("Doe, Jane; Roe, John; Smith, Sam; Lee, Ann"), "Doe, J.; J. Roe; et al.");
    // A hyphenated given takes one initial (initialize-with-hyphen is out of the subset).
    assert_eq!(c("Berg, Anna-Marie"), "Berg, A.");
    // Odd name fields: empty segments are nothing, a comma-less segment is literal.
    assert_eq!(c(""), "");
    assert_eq!(c("; ; , ;"), "");
    assert_eq!(c("World Health Organization"), "World Health Organization");
    // A hostile pile of names still truncates (the name list is capped at 256).
    let pile = "Smith, John; ".repeat(300);
    assert_eq!(c(&pile), "Smith, J.; J. Smith; et al.");
    // A gigantic given name renders one initial, not the string.
    let giant = format!("Zed, {}", "a".repeat(100_000));
    assert_eq!(c(&giant), "Zed, a.");

    let s = style(EDITORS_SORTED);
    let mut e = src("e", "book");
    e.editor = "Lee, Ann; Fox, Kim; Ray, Sid".into();
    assert_eq!(s.cite(&e, Position::First, 1, ""), "Lee, Ann / Fox, Kim & Ray, Sid");
}

#[test]
fn choose_type_and_position_branches() {
    let s = style(CHOOSE);
    let mut book = src("b", "book");
    book.author = "Doe, Jane".into();
    let mut art = src("a", "article");
    art.author = "Tan, Wu".into();
    let mut web = src("w", "website");
    web.url = "https://example.com".into();
    let mut doi_art = src("d", "article");
    doi_art.author = "Ick, Io".into();
    doi_art.doi = "10.1/x".into();

    assert_eq!(s.cite(&book, Position::First, 1, ""), "{BOOK}");
    assert_eq!(s.cite(&doi_art, Position::First, 1, ""), "{ONLINE}");
    // The authorless website fails the `match="none"` test first.
    assert_eq!(s.cite(&web, Position::Subsequent, 1, ""), "{NOAUTHOR}");
    assert_eq!(s.cite(&art, Position::First, 1, ""), "{FIRST}");
    assert_eq!(s.cite(&art, Position::Subsequent, 1, ""), "{LATER}");
    assert_eq!(s.cite(&art, Position::Ibid, 1, ""), "{LATER}");
    // Bibliography rendering: position conditions are false there.
    let entries = s.entries(&[book, art]);
    assert_eq!(entries[0].0, "NOPOSITION");
    assert_eq!(entries[1].0, "NOPOSITION");
}

#[test]
fn date_forms_render() {
    let s = style(DATES);
    let mut d = src("d", "book");
    d.year = "2020".into();
    d.month = "9".into();
    d.day = "1".into();
    d.accessed = "2021-06-30".into();
    assert_eq!(s.cite(&d, Position::First, 1, ""), "1st September 2020 | 9/1/2020 | September 1, 2020 | Sept. 20 | 6/30/2021");

    // A year alone: missing parts (and their affixes) drop away.
    let mut y = src("y", "book");
    y.year = "2020".into();
    assert_eq!(s.cite(&y, Position::First, 1, ""), "2020 | 2020 | 2020 | 20");

    let mut md = src("md", "book");
    md.year = "1999".into();
    md.month = "12".into();
    md.day = "21".into();
    assert_eq!(s.cite(&md, Position::First, 1, ""), "21st December 1999 | 12/21/1999 | December 21, 1999 | Dec. 99");

    // Month 13 is no month; an accessed that isn't yyyy-mm-dd has no parts.
    let mut odd = src("odd", "book");
    odd.year = "2001".into();
    odd.month = "13".into();
    odd.accessed = "last summer".into();
    assert_eq!(s.cite(&odd, Position::First, 1, ""), "2001 | 2001 | 2001 | 01");
}

#[test]
fn labels_terms_and_plurals() {
    let s = style(LABELS);
    assert!(s.warned().is_empty(), "{:?}", s.warned());
    let mut f = src("f", "book");
    f.pages = "10-20".into();
    f.volume = "2".into();
    f.issue = "3".into();
    f.edition = "2".into();
    f.editor = "Lee, Ann; Fox, Kim".into();
    // pp. for a range, volume singular with the ordinal number form, nos. forced plural,
    // the names' own label plural by the editor count, term forms with their spellings.
    assert_eq!(s.cite(&f, Position::First, 1, ""), "pp. 10-20; volume 2nd; nos. 3; Ann Lee & Kim Fox (eds.); edited by & n.d.; ed. 2");

    let mut g = src("g", "book");
    g.pages = "12".into();
    g.edition = "2".into();
    g.editor = "Solo, Max".into();
    // p. for one page; the empty volume/issue groups render nothing; one editor, ed.
    assert_eq!(s.cite(&g, Position::First, 1, ""), "p. 12; Max Solo (ed.); edited by & n.d.; ed. 2");

    let mut none = src("n", "book");
    none.pages = "12".into();
    // No editors at all: the names element — its label included — renders nothing.
    assert_eq!(s.cite(&none, Position::First, 1, ""), "p. 12; edited by & n.d.");
}

#[test]
fn bibliography_sort_order_and_keys() {
    let s = style(SORTED);
    let t = |title: &str, year: &str| Source { title: title.into(), year: year.into(), ..src("t", "book") };
    let items = [t("Beta", "2020"), t("Alpha", "1999"), t("alpha 2", "2020"), t("Gamma", "2020"), t("alpha 2", "2020")];
    let entries = s.entries(&items);
    let texts: Vec<&str> = entries.iter().map(|(x, _)| x.as_str()).collect();
    // Descending year, ascending (case-insensitive) title, ties in the caller's order.
    assert_eq!(texts, ["1: alpha 2", "2: alpha 2", "3: Beta", "4: Gamma", "5: Alpha"]);
    assert_eq!(entries[0].1, ["2020", "alpha 2"]);
    assert_eq!(entries[2].1, ["2020", "beta"]);
    assert_eq!(entries[4].1, ["1999", "alpha"]);

    // No sort keys: the caller's order, numbered from it, and an empty key vector.
    let s = style(UNSORTED);
    let items = [t("Second", "2020"), t("First", "1999")];
    let entries = s.entries(&items);
    assert_eq!(entries[0], ("Second!".to_string(), Vec::<String>::new()));
    assert_eq!(entries[1], ("First!".to_string(), Vec::<String>::new()));
}

#[test]
fn empty_renders_and_cut_macro_affixes() {
    let s = style(AFFIX);
    assert!(s.warned().iter().any(|w| w.contains("deeper than 32")));
    // All children empty: the group (and the cut macro's affixes) render nothing; the
    // layout's own affixes always render — they are the citation's brackets.
    let empty = src("e", "book");
    assert_eq!(s.cite(&empty, Position::First, 1, ""), "()");
    let mut p = src("p", "book");
    p.publisher = "Pub".into();
    assert_eq!(s.cite(&p, Position::First, 1, ""), "([Pub])");
}

#[test]
fn hostile_sources_render_without_panicking() {
    let s = style(NUMBERS);
    let mut h = src("h", "book");
    h.volume = "2024".into();
    h.issue = "7".into();
    h.pages = "3".into();
    h.year = "2020".into();
    assert_eq!(s.cite(&h, Position::First, usize::MAX, ""), "mmxxiv, 07, 3rd, 20");

    // Numbers that overflow every form render as written; so do short years.
    let mut bad = src("bad", "book");
    bad.volume = "99999999999999999999999".into();
    bad.issue = "12".into();
    bad.pages = "abc".into();
    bad.year = "999999999999999999999".into();
    assert_eq!(s.cite(&bad, Position::First, 1, ""), "99999999999999999999999, 12, abc, 99");

    // Empty items and styles without a bibliography render no entries.
    let s2 = style(NAMES_ET_AL);
    assert_eq!(s2.entries(&[]), Vec::<(String, Vec<String>)>::new());
    // A source with nothing set renders the empty string, not a panic.
    let s3 = style(DATES);
    assert_eq!(s3.cite(&Source::default(), Position::First, 1, ""), "");
}

#[test]
fn bib_title_is_none_and_dynamic_labels_do_not_warn() {
    // CSL styles don't name their bibliography; the caller supplies the heading.
    let s = style(AUTHOR_DATE);
    assert_eq!(s.bib_title(), None);
    // `locator` resolves dynamically (the page term over the cited pages), and the number
    // variables without locale terms are the renderer's business: neither warns.
    let s = parse(
        br#"<style version="1.0"><citation><layout>
        <label variable="locator" form="short"/>
        <label variable="citation-number"/>
      </layout></citation></style>"#,
    )
    .unwrap();
    assert!(s.warned().is_empty(), "{:?}", s.warned());
}

#[test]
fn macro_fan_out_is_bounded_by_the_render_budget() {
    // Each macro references the one below twice: depth 31 parses clean, but a render
    // without a total-work bound expands ~2^31 bodies. The budget stops it; the citation
    // comes back (short, empty tail) instead of hanging.
    let mut defs = String::new();
    for i in 0..31 {
        defs.push_str(&format!("<macro name=\"m{i}\"><text macro=\"m{}\"/><text macro=\"m{}\"/></macro>", i + 1, i + 1));
    }
    defs.push_str("<macro name=\"m31\"><text value=\"x\"/></macro>");
    let xml = format!("<style version=\"1.0\"><citation><layout><text macro=\"m0\"/></layout></citation>{defs}</style>");
    let s = parse(xml.as_bytes()).unwrap();
    assert!(s.warned().is_empty(), "{:?}", s.warned());
    let item = src("a", "book");
    let out = s.cite(&item, crate::Position::First, 1, "");
    // The budget is 10 000 elements: the render stops there and returns (an unbounded
    // render would expand ~2^31 bodies and never come back).
    assert!(out.chars().count() <= 10_000, "the budget stops the fan-out: {} chars", out.chars().count());
}
