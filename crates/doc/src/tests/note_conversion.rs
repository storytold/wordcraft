use super::*;
use crate::para::{InlineObject, NoteKind, OBJ};

fn reference(id: u32, kind: NoteKind) -> InlineObject {
    InlineObject::NoteRef { kind, id, custom: "*".into() }
}

fn styled(len: usize, style: &str) -> Run {
    Run { len, props: CharProps { style: Some(style.into()), italic: Some(true), ..Default::default() } }
}

#[test]
fn conversion_preserves_reference_run_boundaries_and_unmatched_storage() {
    for (foot, end, expected) in [
        (true, false, ["FootnoteReference", "EndnoteReference", "FootnoteReference", "EndnoteReference", "EndnoteReference", "FootnoteReference"]),
        (false, true, ["FootnoteReference", "FootnoteReference", "FootnoteReference", "EndnoteReference", "FootnoteReference", "FootnoteReference"]),
        (true, true, ["FootnoteReference", "EndnoteReference", "FootnoteReference", "EndnoteReference", "FootnoteReference", "FootnoteReference"]),
    ] {
        let mut d = Document::new();
        let note = |style: &str| {
            let mut p = Paragraph::with_text("Original note 漢", CharProps::default());
            p.props.style = Some(style.into());
            p.mark.style = Some(style.into());
            vec![para_block(p)]
        };
        let f = d.add_part(PartKind::Footnote, note("FootnoteText"));
        let e = d.add_part(PartKind::Endnote, note("EndnoteText"));
        let p = Paragraph {
            text: format!("é{OBJ}{OBJ}漢{OBJ}{OBJ}"),
            objects: vec![
                reference(f, NoteKind::Footnote),
                InlineObject::BookmarkStart { name: "kept".into() },
                reference(e, NoteKind::Endnote),
                reference(999, NoteKind::Footnote),
                reference(f, NoteKind::Footnote),
            ],
            runs: vec![
                styled(2, "FootnoteReference"),
                styled(3, "FootnoteReference"),
                styled(3, "FootnoteReference"),
                styled(3, "EndnoteReference"),
                styled(3, "EndnoteReference"),
                styled(3, "FootnoteReference"),
            ],
            ..Paragraph::new()
        };
        let mut table = Table::new(1, 1, 200.0);
        table.rows[0].cells[0].blocks = vec![para_block(p.clone())];
        d.body = vec![para_block(p.clone()), Arc::new(Block::Table(table))];
        let h = d.add_part(PartKind::Header, vec![para_block(p.clone())]);
        let t = d.add_part(PartKind::TextBox, vec![para_block(p.clone())]);
        let before = d.clone();
        assert_eq!(d.convert_notes(foot, end), usize::from(foot) + usize::from(end));
        for (story, path) in [
            (StoryRef::Body, Path::top(0)),
            (StoryRef::Body, Path(vec![1, 0, 0, 0])),
            (StoryRef::Part(h), Path::top(0)),
            (StoryRef::Part(t), Path::top(0)),
        ] {
            let got = d.para(story, &path).unwrap();
            assert_eq!(got.text, p.text);
            assert_eq!(got.runs.iter().map(|r| r.props.style.as_deref().unwrap()).collect::<Vec<_>>(), expected);
            assert!(got.runs.iter().all(|r| r.props.italic == Some(true)));
            assert_eq!(got.objects[1], p.objects[1]);
            assert_eq!(got.objects[3], p.objects[3], "a missing note stays unchanged");
            assert_eq!(got.objects[0], reference(f, if foot { NoteKind::Endnote } else { NoteKind::Footnote }));
            assert_eq!(got.objects[2], reference(e, if end { NoteKind::Footnote } else { NoteKind::Endnote }));
            assert_eq!(got.objects[4], got.objects[0], "surplus object storage retains the existing conversion");
        }
        for (id, flipped, from, to) in [(f, foot, "FootnoteText", "EndnoteText"), (e, end, "EndnoteText", "FootnoteText")] {
            let got = d.para(StoryRef::Part(id), &Path::top(0)).unwrap();
            assert_eq!(got.text, before.plain_text(StoryRef::Part(id)));
            assert_eq!(got.props.style.as_deref(), Some(if flipped { to } else { from }));
            assert_eq!(got.mark.style, got.props.style);
        }
        let mut missing = Paragraph {
            text: format!("{OBJ}{OBJ}"),
            objects: vec![reference(f, if foot { NoteKind::Endnote } else { NoteKind::Footnote })],
            runs: vec![styled(0, "FootnoteReference"), styled(3, "FootnoteReference"), styled(3, "FootnoteReference")],
            ..Paragraph::new()
        };
        missing.runs.push(styled(usize::MAX, "FootnoteReference"));
        d.body = vec![para_block(missing)];
        d.convert_notes(true, true);
        let got = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(
            got.runs.iter().map(|r| r.props.style.as_deref().unwrap()).collect::<Vec<_>>(),
            ["FootnoteReference", "EndnoteReference", "FootnoteReference", "FootnoteReference"]
        );
    }
}

fn dense(n: usize) -> Document {
    let mut d = Document::new();
    let id = d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Shared original note", CharProps::default()))]);
    d.body = vec![para_block(Paragraph {
        text: format!("{}{}", OBJ.to_string().repeat(n), "x".repeat(n)),
        objects: (0..n).map(|_| reference(id, NoteKind::Footnote)).collect(),
        runs: (0..n).map(|_| styled(OBJ.len_utf8(), "FootnoteReference")).chain((0..n).map(|_| styled(1, "FootnoteReference"))).collect(),
        ..Paragraph::new()
    })];
    d
}

// Thread CPU excludes scheduler delays. Missing or nonadvancing counters provide no timing
// evidence; the semantic assertions still run on those platforms.
fn cpu_cost(f: impl FnOnce()) -> Option<f64> {
    let ran = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
    let before = ran();
    f();
    match (before, ran()) {
        (Some(a), Some(b)) if b > a => Some((b - a) as f64 * 1e-9),
        _ => None,
    }
}

#[test]
fn conversion_work_scales_with_reference_and_formatting_runs() {
    let (small, big) = (dense(2_000), dense(16_000));
    let convert = |input: &Document| {
        let mut d = input.clone();
        assert_eq!(d.convert_notes(true, false), 1);
        let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        let n = p.objects.len();
        assert!(p.runs[..n].iter().all(|r| r.props.style.as_deref() == Some("EndnoteReference")));
        assert!(p.runs[n..].iter().all(|r| r.props.style.as_deref() == Some("FootnoteReference")));
        std::hint::black_box(d);
    };
    convert(&small);
    convert(&big);
    let (mut t1, mut t8) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..3 {
        let a = cpu_cost(|| (0..8).for_each(|_| convert(&small)));
        let b = cpu_cost(|| convert(&big));
        if let (Some(a), Some(b)) = (a, b) {
            t1 = t1.min(a / 8.0);
            t8 = t8.min(b);
        }
    }
    if t1.is_finite() && t8.is_finite() {
        let ratio = t8 / t1;
        eprintln!("note conversion CPU: small={t1:.6}s big={t8:.6}s ratio={ratio:.2}");
        assert!(ratio < 24.0, "8x the references and runs cost {ratio:.2}x CPU");
    }
}
