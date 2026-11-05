//! Word bibliography sources: a `b:Sources` document — the custom XML part of a document, or
//! Word's own `Sources.xml` master list, imported through Manage Sources (#2640).

use std::collections::HashSet;

use wordcraft_doc::Source;

use crate::DocxError;
use crate::xml::El;

/// Bibliography sources kept from one file (Word's own source manager holds far fewer).
pub const MAX_SOURCES: usize = 10_000;
/// Characters kept of a bibliography's citation style name.
pub const MAX_STYLE_NAME: usize = 64;

/// A `b:Sources` document read into the document model: its sources in file order, and the
/// citation style it names (`b:Sources/@StyleName`), when it has one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    pub sources: Vec<Source>,
    pub style: Option<String>,
}

/// The bibliography namespace Word's own `Sources.xml` files may carry instead of OOXML's (the
/// VBA `Source.XML` format, `schemas.microsoft.com/office/word/2004/10/bibliography`).
const WORD_2004_BIBLIOGRAPHY: &str = "http://schemas.microsoft.com/office/word/2004/10/bibliography";

/// Read a Word bibliography sources file — Word's `Sources.xml`, or any `b:Sources` document in
/// one of the bibliography namespaces (ECMA-376's, or Word's own 2004/10) — into [`Sources`].
pub fn read_sources(bytes: &[u8]) -> Result<Sources, DocxError> {
    // `parse_ns` maps Word's 2004/10 namespace onto the canonical `b` prefix as well, so both
    // formats read; the root check then refuses an un-namespaced or unrelated `<Sources>`.
    let root = crate::xml::parse_ns(bytes, &[("b", WORD_2004_BIBLIOGRAPHY)])?;
    if root.name != "b:Sources" {
        return Err(DocxError::NotWord(format!("root element is {}", root.name)));
    }
    Ok(parse(&root, &mut Seen::default(), MAX_SOURCES))
}

/// Bibliography sources already read: tags, and (title, author) of untagged ones.
#[derive(Default)]
pub(crate) struct Seen {
    tags: HashSet<String>,
    untagged: HashSet<(String, String)>,
}

/// The fresh sources and style of a parsed `b:Sources` root element: `seen` carries the tags and
/// untagged (title, author) keys already taken, so repeated parts collapse into one source, and
/// at most `room` fresh sources are built (later ones are left unread, bounding hostile files).
pub(crate) fn parse(root: &El, seen: &mut Seen, room: usize) -> Sources {
    let style = root
        .attrs
        .iter()
        .find(|(k, _)| k.rsplit(':').next() == Some("StyleName"))
        .map(|(_, v)| v.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(MAX_STYLE_NAME).collect());
    let child_text = |el: &El, local: &str| el.els().find(|e| e.local() == local).map(|e| e.text()).unwrap_or_default();
    let parse_persons = |parent: &El| -> String {
        let mut names = Vec::new();
        let person_els: Vec<&El> = {
            let in_lists: Vec<&El> =
                parent.els().filter(|e| e.local() == "NameList").flat_map(|n| n.els().filter(|e| e.local() == "Person")).collect();
            if in_lists.is_empty() { parent.els().filter(|e| e.local() == "Person").collect() } else { in_lists }
        };
        for name_el in person_els {
            let last = child_text(name_el, "Last");
            let first = child_text(name_el, "First");
            let middle = child_text(name_el, "Middle");
            let name = if !first.is_empty() && !middle.is_empty() {
                format!("{last}, {first} {middle}")
            } else if !first.is_empty() {
                format!("{last}, {first}")
            } else {
                last
            };
            if !name.is_empty() {
                names.push(name);
            }
        }
        names.join("; ")
    };

    let mut out = Sources { sources: Vec::new(), style };
    for src_el in root.els().filter(|e| e.local() == "Source") {
        if out.sources.len() >= room {
            log::warn!("docx: more than {MAX_SOURCES} bibliography sources; the rest are dropped");
            break;
        }
        let tag = child_text(src_el, "Tag");
        let raw_kind = child_text(src_el, "SourceType");
        let kind = crate::write::from_source_type(&raw_kind);
        let (author, corporate, editor) = match src_el.els().find(|e| e.local() == "Author") {
            Some(a) => {
                let role = a.els().find(|e| e.local() == "Author");
                let auth = role.map(parse_persons).filter(|s| !s.is_empty()).unwrap_or_else(|| parse_persons(a));
                // Word's Corporate Author: one name, kept whole (`b:Corporate`, #4848).
                let corporate = role.map(|r| child_text(r, "Corporate").trim().to_string()).unwrap_or_default();
                let ed = a.els().find(|e| e.local() == "Editor").map(parse_persons).unwrap_or_default();
                (auth, corporate, ed)
            }
            None => (String::new(), String::new(), String::new()),
        };
        let editor = if editor.is_empty() { src_el.els().find(|e| e.local() == "Editor").map(parse_persons).unwrap_or_default() } else { editor };
        let title = child_text(src_el, "Title");
        let year = child_text(src_el, "Year");
        let month = child_text(src_el, "Month");
        let day = child_text(src_el, "Day");
        let city = child_text(src_el, "City");
        let publisher = child_text(src_el, "Publisher");
        let journal = child_text(src_el, "JournalName");
        let edition = child_text(src_el, "Edition");
        let volume = child_text(src_el, "Volume");
        let issue = child_text(src_el, "Issue");
        let pages = child_text(src_el, "Pages");
        let url = child_text(src_el, "URL");
        let accessed = match child_text(src_el, "Accessed") {
            a if !a.is_empty() => a,
            _ => {
                let y = child_text(src_el, "YearAccessed");
                let m = child_text(src_el, "MonthAccessed");
                let d = child_text(src_el, "DayAccessed");
                if !y.is_empty() && !m.is_empty() && !d.is_empty() {
                    format!("{y}-{m}-{d}")
                } else if !y.is_empty() && !m.is_empty() {
                    format!("{y}-{m}")
                } else {
                    y
                }
            }
        };
        let doi = match child_text(src_el, "DOI") {
            d if !d.is_empty() => d,
            _ => child_text(src_el, "StandardNumber"),
        };

        // Keep sources with some content, once each (by tag, or title and author untagged).
        if tag.is_empty() && title.is_empty() && author.is_empty() && corporate.is_empty() && editor.is_empty() {
            continue;
        }
        let fresh = if tag.is_empty() {
            let who = [&author, &corporate, &editor].into_iter().find(|w| !w.is_empty()).cloned().unwrap_or_default();
            seen.untagged.insert((title.clone(), who))
        } else {
            seen.tags.insert(tag.clone())
        };
        if !fresh {
            continue;
        }
        out.sources.push(Source {
            tag,
            kind,
            author,
            corporate,
            title,
            year,
            city,
            publisher,
            journal,
            volume,
            pages,
            url,
            editor,
            edition,
            issue,
            month,
            day,
            accessed,
            doi,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORD_SOURCES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"
           xmlns="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"
           SelectedStyle="\MLA.XSL" StyleName="MLA">
  <b:Source>
    <b:Tag>Riv21</b:Tag>
    <b:SourceType>Book</b:SourceType>
    <b:Author>
      <b:Author>
        <b:NameList>
          <b:Person><b:Last>Rivera</b:Last><b:First>Alex</b:First></b:Person>
          <b:Person><b:Last>Chen</b:Last><b:First>Mei</b:First><b:Middle>L.</b:Middle></b:Person>
        </b:NameList>
      </b:Author>
    </b:Author>
    <b:Title>Shared Spaces</b:Title>
    <b:Year>2021</b:Year>
    <b:City>Portland</b:City>
    <b:Publisher>Harbor Books</b:Publisher>
  </b:Source>
  <b:Source>
    <b:Tag>Web19</b:Tag>
    <b:SourceType>InternetSite</b:SourceType>
    <b:Author>
      <b:Author>
        <b:NameList>
          <b:Person><b:Last>Okafor</b:Last><b:First>Ada</b:First></b:Person>
        </b:NameList>
      </b:Author>
    </b:Author>
    <b:Title>Dockyards of the north</b:Title>
    <b:Year>2019</b:Year>
    <b:URL>https://example.org/docks</b:URL>
    <b:YearAccessed>2024</b:YearAccessed>
    <b:MonthAccessed>May</b:MonthAccessed>
    <b:DayAccessed>17</b:DayAccessed>
    <b:StandardNumber>10.1234/x</b:StandardNumber>
  </b:Source>
  <b:Source>
    <b:Tag>NoDate</b:Tag>
    <b:SourceType>JournalArticle</b:SourceType>
    <b:Title>Untitled notes</b:Title>
    <b:JournalName>Field Notes</b:JournalName>
    <b:Volume>4</b:Volume>
  </b:Source>
  <b:Source>
    <b:Tag>Riv21</b:Tag>
    <b:SourceType>Book</b:SourceType>
    <b:Title>A duplicate of Riv21</b:Title>
    <b:Year>2021</b:Year>
  </b:Source>
  <b:Source>
    <b:SourceType>Book</b:SourceType>
  </b:Source>
</b:Sources>"#;

    /// A Word `Sources.xml` (`b:Sources`): its style, the `b:Source` elements with their names,
    /// kinds and fallback fields; a repeated tag and an entry with no content at all are dropped.
    #[test]
    fn a_word_sources_file_reads_into_sources() {
        let f = read_sources(WORD_SOURCES.as_bytes()).unwrap();
        assert_eq!(f.style.as_deref(), Some("MLA"), "the citation style comes from StyleName");
        assert_eq!(
            f.sources.len(),
            3,
            "the repeated tag and the empty b:Source are dropped: {:?}",
            f.sources.iter().map(|s| &s.tag).collect::<Vec<_>>()
        );
        let book = &f.sources[0];
        assert_eq!((book.tag.as_str(), book.kind.as_str()), ("Riv21", "book"));
        assert_eq!(book.author, "Rivera, Alex; Chen, Mei L.");
        assert_eq!(book.title, "Shared Spaces", "the first source of a repeated tag wins");
        assert_eq!(
            (book.title.as_str(), book.year.as_str(), book.city.as_str(), book.publisher.as_str()),
            ("Shared Spaces", "2021", "Portland", "Harbor Books")
        );
        let web = &f.sources[1];
        assert_eq!(web.kind, "website");
        assert_eq!(web.author, "Okafor, Ada");
        assert_eq!(web.accessed, "2024-May-17", "assemble the accessed date from its parts");
        assert_eq!(web.doi, "10.1234/x", "StandardNumber stands in for a missing DOI");
        let notes = &f.sources[2];
        assert_eq!((notes.kind.as_str(), notes.journal.as_str(), notes.volume.as_str()), ("article", "Field Notes", "4"));
    }

    /// Anything whose root isn't `b:Sources` is refused, not read as an empty bibliography.
    #[test]
    fn a_file_that_is_not_b_sources_is_an_error() {
        assert!(read_sources(b"<html><body>hi</body></html>").is_err());
        assert!(matches!(read_sources(b"not xml at all"), Err(DocxError::Xml(_))));
        assert!(read_sources(b"<Sources><Source><Tag>A</Tag></Source></Sources>").is_err(), "no namespace");
        assert!(read_sources(br#"<o:Sources xmlns:o="urn:example:other"><o:Source/></o:Sources>"#).is_err(), "an unrelated namespace");
    }

    /// #4848: a corporate author (Word's Corporate Author box, `b:Corporate`) is read whole, and
    /// an untagged source with only a corporate author is still kept, once.
    #[test]
    fn a_corporate_author_is_read_whole() {
        let xml = br#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source><b:Tag>Wor20</b:Tag><b:SourceType>Report</b:SourceType>
    <b:Author><b:Author><b:Corporate> World Health Organization </b:Corporate></b:Author></b:Author>
    <b:Title>World report on hearing</b:Title><b:Year>2020</b:Year></b:Source>
  <b:Source><b:SourceType>Book</b:SourceType>
    <b:Author><b:Author><b:Corporate>Harbor Guild, Inc.</b:Corporate></b:Author></b:Author></b:Source>
  <b:Source><b:SourceType>Book</b:SourceType>
    <b:Author><b:Author><b:Corporate>Harbor Guild, Inc.</b:Corporate></b:Author></b:Author></b:Source>
</b:Sources>"#;
        let f = read_sources(xml).unwrap();
        assert_eq!(f.sources.len(), 2, "{:?}", f.sources);
        assert_eq!((f.sources[0].corporate.as_str(), f.sources[0].author.as_str()), ("World Health Organization", ""));
        assert_eq!(f.sources[1].corporate, "Harbor Guild, Inc.", "a comma doesn't split a corporate name");
    }

    /// Word's own 2004/10 bibliography namespace (`Sources.xml` written by scripts and older
    /// builds, the VBA `Source.XML` format) reads as well as OOXML's.
    #[test]
    fn word_2004_bibliography_namespace_is_accepted() {
        let xml = br#"<b:Sources xmlns:b="http://schemas.microsoft.com/office/word/2004/10/bibliography" StyleName="MLA">
  <b:Source><b:Tag>Riv21</b:Tag><b:SourceType>Book</b:SourceType><b:Title>Shared Spaces</b:Title><b:Year>2021</b:Year></b:Source>
</b:Sources>"#;
        let f = read_sources(xml).unwrap();
        assert_eq!(f.style.as_deref(), Some("MLA"));
        assert_eq!(f.sources.len(), 1);
        assert_eq!(f.sources[0].tag, "Riv21");
    }
}
