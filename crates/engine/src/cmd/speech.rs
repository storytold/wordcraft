//! Read Aloud (Review › Speech) and moving the caret past citations.
//!
//! The player lives in [`crate::speech`]; these commands drive it, so the player window, the
//! ribbon, the CLI and agents all control reading the same way.

use serde_json::Value;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{InlineObject, Pos};

use super::sel_result;
use crate::speech::{self, State};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("review.readAloud", "Read Aloud", "Review › Speech", read_aloud)
            .key("Mod+Alt+Space")
            .params(r#"{"skipCitations"?: bool}"#)
            .pure(),
        CommandSpec::new("readAloud.playPause", "Play/Pause", "Review › Speech", play_pause).pure(),
        CommandSpec::new("readAloud.next", "Next Sentence", "Review › Speech", next).pure(),
        CommandSpec::new("readAloud.previous", "Previous Sentence", "Review › Speech", previous).pure(),
        CommandSpec::new("readAloud.stop", "Stop Reading", "Review › Speech", stop).pure(),
        CommandSpec::new("readAloud.speed", "Reading Speed", "Review › Speech", speed).params(r#"{"value": number (0.5…3, 1 = normal)}"#).pure(),
        CommandSpec::new("readAloud.skipCitations", "Skip Citations and Bibliography", "Review › Speech", skip_citations)
            .params(r#"{"value"?: bool}"#)
            .pure(),
        CommandSpec::new("readAloud.status", "Read Aloud Status", "Review › Speech", status).pure(),
        CommandSpec::new("caret.pastCitation", "Move Past Citation", "Zotero › Citations", past_citation).pure(),
    ]
}

/// Start reading from the caret (or the selection); while reading, pause or carry on.
fn read_aloud(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(b) = p::bool(v, "skipCitations") {
        s.read_aloud.skip_citations = b;
    }
    if s.read_aloud.state() != State::Stopped {
        s.read_aloud.play_pause();
        return status(s, v);
    }
    let skip = s.read_aloud.skip_citations;
    let mut queue = if s.sel.is_collapsed() {
        speech::sentences(&s.doc, &s.sel.focus, None, skip)
    } else {
        let (a, b) = s.sel.ordered();
        speech::sentences(&s.doc, &a, Some(&b), skip)
    };
    // At the end of the document: read it from the top.
    if queue.is_empty() && s.sel.is_collapsed() {
        let top = s.doc.start_of(s.sel.focus.story);
        queue = speech::sentences(&s.doc, &top, None, skip);
    }
    if queue.is_empty() {
        return Err(CmdError::Failed("there's nothing to read".into()));
    }
    s.read_aloud.start(queue);
    status(s, v)
}

fn play_pause(s: &mut Session, v: &Value) -> CmdResult {
    if s.read_aloud.progress().1 == 0 {
        return read_aloud(s, v);
    }
    s.read_aloud.play_pause();
    status(s, v)
}

fn next(s: &mut Session, v: &Value) -> CmdResult {
    s.read_aloud.skip(1);
    status(s, v)
}

fn previous(s: &mut Session, v: &Value) -> CmdResult {
    s.read_aloud.skip(-1);
    status(s, v)
}

fn stop(s: &mut Session, v: &Value) -> CmdResult {
    s.read_aloud.stop();
    status(s, v)
}

fn speed(s: &mut Session, v: &Value) -> CmdResult {
    let r = p::f32(v, "value").or_else(|| p::f32(v, "rate")).ok_or_else(|| CmdError::Params("`value` (number, 0.5…3) is required".into()))?;
    s.read_aloud.set_rate(r);
    status(s, v)
}

/// Turn skipping citations on or off; a reading in progress carries on from the same sentence.
fn skip_citations(s: &mut Session, v: &Value) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or(!s.read_aloud.skip_citations);
    if on != s.read_aloud.skip_citations {
        s.read_aloud.skip_citations = on;
        if let Some(cur) = s.read_aloud.current() {
            let paused = s.read_aloud.state() == State::Paused;
            let queue = speech::sentences(&s.doc, &cur.start, None, on);
            if !queue.is_empty() {
                s.read_aloud.start(queue);
                if paused {
                    s.read_aloud.play_pause();
                }
            }
        }
    }
    status(s, v)
}

fn status(s: &mut Session, _: &Value) -> CmdResult {
    Ok(s.read_aloud.status())
}

/// Move the caret to just after the citation it's in or before (a run of adjacent citations
/// counts as one); with no such citation in the paragraph, after the next one in the document.
fn past_citation(s: &mut Session, _: &Value) -> CmdResult {
    let caret = s.sel.ordered().1;
    let story = caret.story;
    let obj = OBJ.len_utf8();
    // Every citation as (first position, position just after it).
    let mut spans: Vec<(Pos, Pos)> = s
        .doc
        .field_ranges(story)
        .into_iter()
        .filter(|r| speech::is_citation_code(&r.instr))
        .map(|r| (r.start, Pos::new(story, r.end.path.clone(), r.end.off + obj)))
        .collect();
    for path in s.doc.para_paths(story) {
        let Some(para) = s.doc.para(story, &path) else { continue };
        for off in para.object_offsets() {
            if let Some(InlineObject::Field { instr, .. }) = para.object_at(off)
                && speech::is_citation_code(instr)
            {
                spans.push((Pos::new(story, path.clone(), off), Pos::new(story, path.clone(), off + obj)));
            }
        }
    }
    spans.sort();
    // The citation the caret is in or right before, else the next one.
    let Some(mut i) = spans.iter().position(|(_, end)| *end > caret) else {
        return Err(CmdError::Failed("there's no citation after the caret".into()));
    };
    let mut after = spans.get(i).map(|x| x.1.clone()).unwrap_or(caret);
    // Swallow citations that follow directly ("(A)(B)" or "(A) (B)").
    while let Some((start, end)) = spans.get(i + 1) {
        let gap = if start.path == after.path { s.doc.para(story, &after.path).and_then(|p| p.text.get(after.off..start.off)) } else { None };
        if !gap.is_some_and(|g| g.chars().all(|c| c.is_whitespace() || c == ';' || c == ',')) {
            break;
        }
        after = end.clone();
        i += 1;
    }
    s.sel = crate::Selection::caret(s.doc.clamp(&after));
    s.goal_x = None;
    sel_result(s)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Document, InlineObject, Path, Pos, StoryRef};

    use crate::Session;
    use crate::speech::State;

    fn cite(d: &mut Document, block: usize, off: usize, text: &str) {
        let c = CharProps::default();
        let p = d.para_mut(StoryRef::Body, &Path::top(block)).unwrap();
        p.insert_object(off, InlineObject::FieldStart { instr: "ADDIN ZOTERO_ITEM CSL_CITATION {}".into(), locked: false }, &c).unwrap();
        let n = p.insert_text(off + 3, text, &c).unwrap();
        p.insert_object(off + 3 + n, InlineObject::FieldEnd, &c).unwrap();
    }

    /// "Cats purr (Smith 2020). Dogs bark (Jones 2021; Lee 2019).\nReferences\nSmith J. Cats.\nJones K. Dogs."
    /// with Zotero citations and a Zotero bibliography over the last two paragraphs.
    fn doc() -> Document {
        let mut d = Document::from_text("Cats purr . Dogs bark .\nReferences\nSmith J. Cats.\nJones K. Dogs.");
        cite(&mut d, 0, 22, "(Jones 2021; Lee 2019)");
        cite(&mut d, 0, 10, "(Smith 2020)");
        let c = CharProps::default();
        d.para_mut(StoryRef::Body, &Path::top(2))
            .unwrap()
            .insert_object(0, InlineObject::FieldStart { instr: "ADDIN ZOTERO_BIBL {} CSL_BIBLIOGRAPHY".into(), locked: false }, &c)
            .unwrap();
        let p3 = d.para_mut(StoryRef::Body, &Path::top(3)).unwrap();
        let end = p3.len();
        p3.insert_object(end, InlineObject::FieldEnd, &c).unwrap();
        d
    }

    fn texts(s: &Session) -> Vec<String> {
        crate::speech::sentences(&s.doc, &Pos::body(0, 0), None, s.read_aloud.skip_citations).into_iter().map(|u| u.text).collect()
    }

    #[test]
    fn citations_and_bibliography_are_skipped() {
        let mut s = Session::new(doc());
        assert_eq!(s.doc.plain_text(StoryRef::Body).lines().next(), Some("Cats purr (Smith 2020). Dogs bark (Jones 2021; Lee 2019)."));
        assert_eq!(texts(&s), ["Cats purr.", "Dogs bark.", "References"]);
        s.run("readAloud.skipCitations", &json!({"value": false})).unwrap();
        assert_eq!(texts(&s), ["Cats purr (Smith 2020).", "Dogs bark (Jones 2021; Lee 2019).", "References", "Smith J. Cats.", "Jones K. Dogs."]);
    }

    #[test]
    fn read_aloud_commands_drive_the_player() {
        let mut s = Session::new(doc());
        s.sel = crate::Selection::caret(Pos::body(0, 0));
        let r = s.run("review.readAloud", &json!({})).unwrap();
        assert_eq!(r["state"], "playing");
        assert_eq!(r["count"], 3);
        assert_eq!(r["current"]["text"], "Cats purr.");
        assert_eq!(s.run("readAloud.next", &json!({})).unwrap()["current"]["text"], "Dogs bark.");
        assert_eq!(s.run("readAloud.previous", &json!({})).unwrap()["index"], 0);
        assert_eq!(s.run("readAloud.speed", &json!({"value": 1.5})).unwrap()["rate"], 1.5);
        assert!(s.run("readAloud.speed", &json!({})).is_err());
        // Pressing Read Aloud again pauses, then carries on.
        assert_eq!(s.run("review.readAloud", &json!({})).unwrap()["state"], "paused");
        assert_eq!(s.run("readAloud.playPause", &json!({})).unwrap()["state"], "playing");
        // Turning skipping off mid-reading keeps the place.
        s.run("readAloud.next", &json!({})).unwrap();
        let r = s.run("readAloud.skipCitations", &json!({"value": false})).unwrap();
        assert_eq!(r["current"]["text"], "Dogs bark (Jones 2021; Lee 2019).");
        assert_eq!(r["count"], 4);
        let r = s.run("readAloud.stop", &json!({})).unwrap();
        assert_eq!(r["state"], "stopped");
        assert_eq!(r["open"], false);
        assert_eq!(s.read_aloud.state(), State::Stopped);
    }

    #[test]
    fn read_aloud_reads_the_selection_only() {
        let mut s = Session::new(Document::from_text("One. Two. Three."));
        s.sel = crate::Selection { anchor: Pos::body(0, 5), focus: Pos::body(0, 9) };
        let r = s.run("review.readAloud", &json!({})).unwrap();
        assert_eq!(r["count"], 1);
        assert_eq!(r["current"]["text"], "Two.");
    }

    #[test]
    fn read_aloud_at_the_end_starts_from_the_top() {
        let mut s = Session::new(Document::from_text("One. Two."));
        s.sel = crate::Selection::caret(Pos::body(0, 9));
        assert_eq!(s.run("review.readAloud", &json!({})).unwrap()["current"]["text"], "One.");
        let mut s = Session::new(Document::from_text(""));
        assert!(s.run("review.readAloud", &json!({})).is_err());
    }

    #[test]
    fn move_past_citation() {
        let mut s = Session::new(doc());
        // Before the first citation → just after it.
        s.sel = crate::Selection::caret(Pos::body(0, 5));
        s.run("caret.pastCitation", &json!({})).unwrap();
        let first_end = s.sel.focus.clone();
        let p = s.doc.para_at(&first_end).unwrap();
        assert_eq!(p.text.get(first_end.off..), Some(". Dogs bark \u{FFFC}(Jones 2021; Lee 2019)\u{FFFC}."));
        // Inside the second → after it.
        let inside = p.text.find("Lee").unwrap();
        s.sel = crate::Selection::caret(Pos::body(0, inside));
        s.run("caret.pastCitation", &json!({})).unwrap();
        let p = s.doc.para_at(&s.sel.focus).unwrap();
        assert_eq!(p.text.get(s.sel.focus.off..), Some("."));
        // After the last citation in the body text: on to the end of the bibliography.
        s.run("caret.pastCitation", &json!({})).unwrap();
        assert_eq!(s.sel.focus.path, Path::top(3));
        assert!(s.run("caret.pastCitation", &json!({})).is_err());
    }

    #[test]
    fn adjacent_citations_are_skipped_together() {
        let mut d = Document::from_text("A  b.");
        cite(&mut d, 0, 2, "(Lee)");
        cite(&mut d, 0, 1, "(Kim)");
        let mut s = Session::new(d);
        s.sel = crate::Selection::caret(Pos::body(0, 1));
        s.run("caret.pastCitation", &json!({})).unwrap();
        let p = s.doc.para_at(&s.sel.focus).unwrap();
        assert_eq!(p.text.get(s.sel.focus.off..), Some(" b."));
    }
}
