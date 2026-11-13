//! Compare builds its result on the original document, so the original's text must all reach the
//! result: unchanged, or marked deleted (#5924). These probes open real `.docx` files through
//! the reader, as the corpus does, and count the original's words in the result two ways: in the
//! session (every run that isn't an insertion) and in the saved file (`w:t` and `w:delText`
//! outside `w:ins`).

use std::collections::HashMap;
use std::io::Read;

use crate::Session;
use serde_json::{Value, json};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Block, Document, StoryRef};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

/// A package holding just `body` (paragraphs and tables).
fn docx(body: &str) -> Vec<u8> {
    let document =
        format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#);
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in [("_rels/.rels", root), ("word/document.xml", document.as_str())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zw, text.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(package: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(package)).unwrap();
    let mut f = z.by_name("word/document.xml").unwrap();
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    s
}

fn para(inner: &str) -> String {
    format!("<w:p>{inner}</w:p>")
}

fn run(t: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{t}</w:t></w:r>"#)
}

/// `text` in runs of `every` characters, cycling plain, bold, italic and coloured: runs split
/// mid-word and mid-sentence, as Word's editing history leaves them.
fn runs(text: &str, every: usize) -> String {
    let rprs = ["", "<w:rPr><w:b/></w:rPr>", "<w:rPr><w:i/></w:rPr>", r#"<w:rPr><w:color w:val="C00000"/></w:rPr>"#];
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(every.max(1))
        .enumerate()
        .map(|(k, c)| format!(r#"<w:r>{}<w:t xml:space="preserve">{}</w:t></w:r>"#, rprs[k % rprs.len()], c.iter().collect::<String>()))
        .collect()
}

/// Distinct words `w{from}`..`w{to}` with the spaces between them.
fn words(prefix: &str, from: usize, to: usize) -> String {
    (from..to).map(|i| format!("{prefix}{i}")).collect::<Vec<_>>().join(" ")
}

/// Prose from a small vocabulary, so words repeat as they do in real text.
fn prose(from: usize, to: usize) -> String {
    const VOCAB: [&str; 12] = ["the", "contract", "shall", "be", "of", "and", "party", "agreement", "notice", "to", "in", "terms"];
    (from..to).map(|i| VOCAB[(i * 7 + i / 5) % VOCAB.len()]).collect::<Vec<_>>().join(" ")
}

fn field(instr: &str, result_runs: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instr} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result_runs}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

fn text_box(text: &str) -> String {
    format!(
        r##"<w:r><w:pict><v:shape type="#_x0000_t202" style="position:absolute;margin-left:300pt;width:150pt;height:30pt"><v:textbox><w:txbxContent><w:p>{}</w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"##,
        run(text)
    )
}

/// What the session shows of the original: every run that isn't an insertion, objects as their
/// text, one line per paragraph (tables' cells in order).
fn session_view(doc: &Document) -> String {
    fn block(b: &Block, out: &mut String, depth: usize) {
        match b {
            Block::Para(p) => {
                let mut k = 0;
                for (r, c) in p.run_ranges() {
                    for ch in p.text.get(r).unwrap_or("").chars() {
                        if ch == OBJ {
                            if c.ins.is_none()
                                && let Some(o) = p.objects.get(k)
                            {
                                out.push_str(o.plain_text());
                            }
                            k += 1;
                        } else if c.ins.is_none() {
                            out.push(ch);
                        }
                    }
                }
                out.push('\n');
            }
            Block::Table(t) if depth < 16 => {
                for b in t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()) {
                    block(b, out, depth + 1);
                    out.push('\t');
                }
            }
            Block::Table(_) => {}
        }
    }
    let mut out = String::new();
    for b in &doc.body {
        block(b, &mut out, 0);
    }
    out
}

/// The text boxes anchored in the body whose anchors aren't insertions: their stories' text.
fn session_boxes(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (off, o) in p.objects_with_offsets() {
            for id in o.text_boxes() {
                if p.props_of_char(off).ins.is_none() {
                    out.push(doc.plain_text(StoryRef::Part(id)));
                }
            }
        }
    }
    out
}

fn unescape(t: &str) -> String {
    t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// What a saved `document.xml` shows of the original: `w:t` and `w:delText` outside `w:ins`, one
/// line per paragraph. Text boxes (written twice, as DrawingML and VML) and field codes are left
/// out; symbols (`w:sym`) aren't text.
fn file_view(xml: &str) -> String {
    let mut out = String::new();
    let (mut ins, mut boxes, mut in_text) = (0usize, 0usize, false);
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        if in_text && ins == 0 && boxes == 0 {
            out.push_str(&unescape(&rest[..lt]));
        }
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        let closing = tag.starts_with('/');
        let empty = tag.ends_with('/');
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match name {
            "w:ins" if !empty => ins = if closing { ins.saturating_sub(1) } else { ins + 1 },
            "w:txbxContent" if !empty => boxes = if closing { boxes.saturating_sub(1) } else { boxes + 1 },
            "w:t" | "w:delText" => in_text = !closing && !empty,
            "w:p" if closing => out.push('\n'),
            "w:tab" | "w:br" | "w:cr" => out.push(' '),
            _ => {}
        }
    }
    out
}

/// The words of `expected` that `got` lacks (as many copies as are missing).
fn missing(expected: &str, got: &str) -> Vec<String> {
    let mut have: HashMap<&str, usize> = HashMap::new();
    for w in got.split_whitespace() {
        *have.entry(w).or_default() += 1;
    }
    let mut out = Vec::new();
    for w in expected.split_whitespace() {
        match have.get_mut(w) {
            Some(n) if *n > 0 => *n -= 1,
            _ => out.push(w.to_string()),
        }
    }
    out
}

/// Compare `original` with `revised` (bodies of two packages, both opened from disk) with the
/// extra `params`, and check that none of the original's words went missing. Returns the saved
/// result's `document.xml`.
fn assert_complete(name: &str, original: &str, revised: &str, params: Value) -> String {
    let (ob, rb) = (docx(original), docx(revised));
    // Tests run in parallel threads of one process, and two may pass the same `name`:
    // the counter keeps each call's pair of files its own.
    static CALL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = CALL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let o = dir.join(format!("wc-compare-probe-{name}-{}-{n}-o.docx", std::process::id()));
    let r = dir.join(format!("wc-compare-probe-{name}-{}-{n}-r.docx", std::process::id()));
    std::fs::write(&o, &ob).unwrap();
    std::fs::write(&r, &rb).unwrap();
    let mut v = json!({"path": r.to_string_lossy(), "original": o.to_string_lossy()});
    if let (Some(v), Some(extra)) = (v.as_object_mut(), params.as_object()) {
        v.extend(extra.clone());
    }
    let mut s = Session::new(Document::new());
    let result = s.run("review.compare", &v);
    let _ = std::fs::remove_file(&o);
    let _ = std::fs::remove_file(&r);
    result.unwrap();
    let before = crate::io::open_bytes("original.docx", &ob).unwrap();
    let in_session = missing(&session_view(&before), &session_view(&s.doc));
    let saved = crate::io::save_bytes("result.docx", &s.doc).unwrap();
    let in_file = missing(&file_view(&document_xml(&ob)), &file_view(&document_xml(&saved)));
    assert!(
        in_session.is_empty() && in_file.is_empty(),
        "{name}: {} original words missing from the result, {} from its saved file; session {:?}, file {:?}",
        in_session.len(),
        in_file.len(),
        in_session.iter().take(20).collect::<Vec<_>>(),
        in_file.iter().take(20).collect::<Vec<_>>()
    );
    let boxes = session_boxes(&s.doc);
    for text in session_boxes(&before) {
        assert!(boxes.contains(&text), "{name}: the original's text box \"{text}\" is missing from the result: {boxes:?}");
    }
    document_xml(&saved)
}

/// For a corpus pair (`WC_COMPARE_ORIGINAL`, `WC_COMPARE_REVISED`: paths to `.docx` files): which
/// of the original's words the result lacks, in the session and in the saved file, so a loss can
/// be placed in the comparison or in the writer. Run with `--ignored --nocapture`.
#[test]
#[ignore = "needs a corpus pair named by WC_COMPARE_ORIGINAL and WC_COMPARE_REVISED"]
fn compare_reports_the_original_words_a_corpus_pair_loses() {
    let (Ok(o), Ok(r)) = (std::env::var("WC_COMPARE_ORIGINAL"), std::env::var("WC_COMPARE_REVISED")) else { return };
    let ob = std::fs::read(&o).unwrap();
    let mut s = Session::new(Document::new());
    s.run("review.compare", &json!({"path": r, "original": o})).unwrap();
    let before = crate::io::open_bytes("original.docx", &ob).unwrap();
    let in_session = missing(&session_view(&before), &session_view(&s.doc));
    let saved = crate::io::save_bytes("result.docx", &s.doc).unwrap();
    // The reader against the file: words of the original's XML the opened document lacks.
    let read = missing(&file_view(&document_xml(&ob)), &session_view(&before));
    let in_file = missing(&file_view(&document_xml(&ob)), &file_view(&document_xml(&saved)));
    println!("reader lost {}: {:?}", read.len(), read.iter().take(40).collect::<Vec<_>>());
    println!("session lost {}: {:?}", in_session.len(), in_session.iter().take(40).collect::<Vec<_>>());
    println!("saved file lost {}: {:?}", in_file.len(), in_file.iter().take(40).collect::<Vec<_>>());
}

fn keep(n: usize) -> String {
    para(&run(&format!("Keeper paragraph number {n} stays as it is.")))
}

#[test]
fn compare_keeps_the_words_of_a_diffed_paragraph_that_opens_with_a_symbol() {
    // A manual bullet: the paragraph's first run is a `w:sym`. That run's formatting must not be
    // given to the words of the diffed paragraph, or the saved file writes every one of them as
    // a symbol in the bullet's font and the text leaves the file.
    let bullet =
        r#"<w:r><w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol"/></w:rPr><w:sym w:font="Symbol" w:char="F0B7"/></w:r><w:r><w:tab/></w:r>"#;
    let original = format!("{}{}{}", keep(1), para(&format!("{bullet}{}", runs(&words("s", 0, 120), 9))), keep(2));
    let revised =
        format!("{}{}{}", keep(1), para(&format!("{bullet}{}", runs(&format!("{} {}", words("s", 0, 40), words("s", 80, 120)), 13))), keep(2));
    assert_complete("symbol-first", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_every_word_of_a_long_multi_run_paragraph_losing_its_middle() {
    for (name, text, cut) in [("distinct", words("w", 0, 640), (200, 440)), ("prose", prose(0, 640), (200, 440))] {
        let all: Vec<&str> = text.split(' ').collect();
        let mut kept: Vec<String> = all.iter().take(cut.0).chain(all.iter().skip(cut.1)).map(|w| w.to_string()).collect();
        // A few words changed before the cut too, so the pair is more than one deletion.
        for w in kept.iter_mut().skip(100).take(10) {
            w.push('x');
        }
        let original = format!("{}{}{}", keep(1), para(&runs(&text, 7)), keep(2));
        let revised = format!("{}{}{}", keep(1), para(&runs(&kept.join(" "), 11)), keep(2));
        assert_complete(&format!("multi-run-{name}"), &original, &revised, json!({}));
        assert_complete(&format!("multi-run-{name}-chars"), &original, &revised, json!({"level": "character"}));
    }
}

#[test]
fn compare_keeps_a_long_paragraph_the_revised_file_deletes_beside_an_edited_one() {
    // Word deletes the long paragraph whole; positionally it pairs with the edited one after it.
    let long = runs(&prose(0, 644), 17);
    let original = format!("{}{}{}{}", keep(1), para(&long), para(&run("A short clause about delivery dates.")), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&run("A short clause about payment dates.")), keep(2));
    assert_complete("deleted-beside-edited", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_the_words_around_inline_objects_of_a_diffed_paragraph() {
    let objects = |middle: &str| {
        format!(
            r#"<w:bookmarkStart w:id="1" w:name="_GoBack"/><w:bookmarkEnd w:id="1"/>{}<w:hyperlink w:anchor="target">{}</w:hyperlink>{}<w:fldSimple w:instr=" PAGE "><w:r><w:t>7</w:t></w:r></w:fldSimple>{}{}{}{}{}"#,
            runs(&words("a", 0, 30), 5),
            run(" linked words here "),
            runs(middle, 6),
            run(" see "),
            field(r"REF target \h", &format!("{}{}", run("stored "), run("reference text"))),
            run(" and "),
            field(r#"HYPERLINK "https://example.com/x""#, &format!("{}{}", run("an outside "), run("link"))),
            runs(&words("z", 0, 30), 8),
        )
    };
    let original = format!("{}{}{}", keep(1), para(&objects(&words("m", 0, 200))), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&objects(&format!("{} {}", words("m", 0, 50), words("m", 150, 200)))), keep(2));
    assert_complete("inline-objects", &original, &revised, json!({}));
    // The objects themselves gone from the revised paragraph, with its middle.
    let revised = format!("{}{}{}", keep(1), para(&runs(&format!("{} {}", words("a", 0, 30), words("m", 150, 200)), 9)), keep(2));
    assert_complete("inline-objects-removed", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_field_results_inside_a_changed_region() {
    let original = format!(
        "{}{}{}",
        keep(1),
        para(&format!(
            "{}{}{}{}{}",
            runs(&words("b", 0, 40), 7),
            field(r#"QUOTE "a long quoted result""#, &runs(&words("q", 0, 30), 4)),
            runs(&words("c", 0, 40), 7),
            field("ADDIN ZOTERO_ITEM CSL_CITATION", &runs(&words("cite", 0, 25), 5)),
            runs(&words("d", 0, 40), 7)
        )),
        keep(2)
    );
    let revised = format!("{}{}{}", keep(1), para(&runs(&format!("{} {}", words("b", 0, 20), words("d", 20, 40)), 7)), keep(2));
    assert_complete("field-results", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_text_boxes_equations_and_controls_of_a_diffed_paragraph() {
    let rich = |middle: &str, box_text: &str| {
        format!(
            r#"{}{}{}<m:oMath><m:r><m:t>x+1</m:t></m:r></m:oMath>{}<w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt>{}"#,
            runs(&words("e", 0, 30), 6),
            text_box(box_text),
            runs(middle, 9),
            run(" after the equation "),
            runs(&words("ctl", 0, 20), 4),
            runs(&words("f", 0, 30), 6),
        )
    };
    let original = format!("{}{}{}", keep(1), para(&rich(&words("m", 0, 150), "Original box words.")), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&rich(&words("m", 60, 150), "Revised box words.")), keep(2));
    let xml = assert_complete("boxes-equations-controls", &original, &revised, json!({}));
    // The equation stays an equation: its linear text never stands in for it as body text.
    assert_eq!(xml.matches("<m:oMath>").count(), 1, "the equation must be saved once, as OMML");
    assert!(!xml.contains("m149x+1"), "the word before the equation must keep its own text");
    // And it keeps its content: an empty `m:oMath` shell would pass the count above.
    let omath = xml.split("<m:oMath>").nth(1).and_then(|t| t.split("</m:oMath>").next()).unwrap_or("");
    let mt: String = omath.split("<m:t>").skip(1).map(|t| t.split("</m:t>").next().unwrap_or("")).collect();
    assert_eq!(mt, "x+1", "the saved equation lost its content");
}

#[test]
fn compare_matches_an_equation_whose_runs_are_segmented_differently() {
    // The same equation, its OMML runs split differently in the two files: the structure
    // (and the linear text) is identical, only the retained source XML differs.
    let single = r#"<m:oMath><m:r><m:t>x+1</m:t></m:r></m:oMath>"#;
    let segmented = r#"<m:oMath><m:r><m:t>x</m:t></m:r><m:r><m:t>+1</m:t></m:r></m:oMath>"#;
    // The words before the equation change; the words after it are the same in both
    // files, so the equation sits between matching text and must itself match.
    let with_eq = |eq: &str, before: &str| format!("{}{eq}{}", run(&format!("{before} ")), run(&format!(" {}", words("m", 75, 150))));
    let original = format!("{}{}{}", keep(1), para(&with_eq(single, &words("m", 0, 75))), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&with_eq(segmented, &format!("{} {}", words("x", 0, 60), words("m", 60, 75)))), keep(2));
    let xml = assert_complete("equation-segmentation", &original, &revised, json!({}));
    assert_eq!(xml.matches("<m:oMath>").count(), 1, "an unchanged equation must be saved once, not as a deleted and an inserted copy");
    let omath = xml.split("<m:oMath>").nth(1).and_then(|t| t.split("</m:oMath>").next()).unwrap_or("");
    let mt: String = omath.split("<m:t>").skip(1).map(|t| t.split("</m:t>").next().unwrap_or("")).collect();
    assert_eq!(mt, "x+1", "the saved equation lost its content");
}

#[test]
fn compare_keeps_an_equations_own_colour_in_a_diffed_paragraph() {
    // The equation's runs share a blue colour, lifted onto its marker run at read; the
    // paragraph's first run is red. The copied equation must keep blue, not take red.
    let blue = r#"<m:oMath><m:r><w:rPr><w:color w:val="0000FF"/></w:rPr><m:t>x+1</m:t></m:r></m:oMath>"#;
    let red_run = |t: &str| format!(r#"<w:r><w:rPr><w:color w:val="C00000"/></w:rPr><w:t xml:space="preserve">{t}</w:t></w:r>"#);
    let with_eq = |before: &str| format!("{}{blue}{}", red_run(&format!("{before} ")), run(&format!(" {}", words("m", 75, 150))));
    let original = format!("{}{}{}", keep(1), para(&with_eq(&words("m", 0, 75))), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&with_eq(&format!("{} {}", words("x", 0, 60), words("m", 60, 75)))), keep(2));
    let xml = assert_complete("equation-colour", &original, &revised, json!({}));
    assert_eq!(xml.matches("<m:oMath>").count(), 1, "the equation must be saved once");
    let omath = xml.split("<m:oMath>").nth(1).and_then(|t| t.split("</m:oMath>").next()).unwrap_or("");
    assert!(omath.contains("0000FF"), "the saved equation must keep its own colour: {omath}");
    assert!(!omath.contains("C00000"), "the saved equation took the paragraph's first run colour: {omath}");
}

#[test]
fn compare_keeps_a_diffed_paragraph_that_opens_with_hidden_text() {
    let hidden = r#"<w:r><w:rPr><w:vanish/></w:rPr><w:t xml:space="preserve">hidden lead </w:t></w:r>"#;
    let original = format!("{}{}{}", keep(1), para(&format!("{hidden}{}", runs(&words("h", 0, 150), 8))), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&format!("{hidden}{}", runs(&words("h", 50, 150), 8))), keep(2));
    assert_complete("hidden-first", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_paragraphs_near_the_character_and_table_limits() {
    // 1,991 and 2,015 characters (five-character words and their spaces): either side of the
    // length past which a character-level compare goes word by word.
    for (name, n) in [("under", 332), ("over", 336)] {
        let text = words("k", 1000, 1000 + n);
        let cut = format!("{} {}", words("k", 1000, 1000 + n / 3), words("k", 1000 + n / 2, 1000 + n));
        let original = format!("{}{}{}", keep(1), para(&runs(&text, 10)), keep(2));
        let revised = format!("{}{}{}", keep(1), para(&runs(&cut, 10)), keep(2));
        assert_complete(&format!("char-limit-{name}"), &original, &revised, json!({"level": "character"}));
    }
    // Over the longest sequence one comparison table is built for: changes near both ends keep
    // the middle long, so it is truncated.
    let text = words("t", 0, 4100);
    let mut cut: Vec<String> = text.split(' ').map(str::to_string).collect();
    cut.drain(1500..1700);
    cut[3].push('x');
    let last = cut.len() - 3;
    cut[last].push('x');
    let original = format!("{}{}{}", keep(1), para(&runs(&text, 50)), keep(2));
    let revised = format!("{}{}{}", keep(1), para(&runs(&cut.join(" "), 50)), keep(2));
    assert_complete("lcs-limit", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_every_block_of_a_region_with_no_matching_block() {
    // Every clause edited, three dropped and two added: no paragraph matches exactly, so the
    // whole body is one run of changes paired by position.
    let clause = |i: usize, days: &str| {
        para(&runs(
            &format!(
                "Clause {i}. The supplier shall deliver the goods described in schedule {i} within {days} days of the order date and shall bear the cost of carriage."
            ),
            23,
        ))
    };
    let original: String = (0..10).map(|i| clause(i, "thirty")).collect();
    let revised: String = (0..10)
        .filter(|i| ![3, 4, 7].contains(i))
        .map(|i| clause(i, "forty"))
        .chain([para(&run("A new clause.")), para(&run("Another new clause."))])
        .collect();
    assert_complete("no-anchor-clauses", &original, &revised, json!({}));
    // Repeated boilerplate: eight copies become five, with edits between them.
    let sign = || para(&runs("Signed for and on behalf of the company by its duly authorised officer.", 19));
    let original: String = (0..8).map(|i| format!("{}{}", sign(), para(&run(&format!("Witness {i} name and address."))))).collect();
    let revised: String = (0..5).map(|i| format!("{}{}", sign(), para(&run(&format!("Witness {i} full name."))))).collect();
    assert_complete("repeated-boilerplate", &original, &revised, json!({}));
}

#[test]
fn compare_keeps_a_table_paired_with_a_paragraph() {
    let table = format!(
        "<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid><w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl>",
        para(&run("cell one words")),
        para(&run("cell two words"))
    );
    let original = format!("{}{}{}{}", keep(1), table, para(&runs(&words("g", 0, 200), 12)), keep(2));
    let revised = format!("{}{}{}{}", keep(1), para(&run("A paragraph where the table was.")), para(&runs(&words("g", 40, 200), 12)), keep(2));
    assert_complete("table-and-paragraph", &original, &revised, json!({}));
}
