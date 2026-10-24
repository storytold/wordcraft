//! Built-in sample documents and templates (original text, generated in code).

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, CharProps, NumRef, Rgb, TabAlign, TabLeader, TabStop, TextColor};
use wordcraft_doc::{Block, Document, ListKind, Paragraph, PartKind, Table, para_block};

fn para(text: &str) -> Paragraph {
    Paragraph::with_text(text, CharProps::default())
}

fn styled(text: &str, style: &str) -> Paragraph {
    para(text).styled(style)
}

/// Append text with formatting to a paragraph.
fn add(p: &mut Paragraph, text: &str, props: CharProps) {
    let at = p.len();
    let _ = p.insert_text(at, text, &props);
}

fn bold() -> CharProps {
    CharProps { bold: Some(true), ..Default::default() }
}
fn italic() -> CharProps {
    CharProps { italic: Some(true), ..Default::default() }
}

/// The showcase document: a short studio handbook with headings, lists, a table, a quote,
/// a footer with page numbers.
pub fn sample_document() -> Document {
    let mut d = Document::new();
    d.core.title = "The Open Studio Handbook".into();
    d.core.creator = "WordCraft".into();
    let mut b: Vec<Block> = Vec::new();
    b.push(Block::Para(styled("The Open Studio Handbook", "Title")));
    b.push(Block::Para(styled("How a small community of makers shares space, tools and time", "Subtitle")));
    let mut intro = para("Every Thursday evening the old print shop on Alder Street fills with people who make things. ");
    add(&mut intro, "Painters", bold());
    add(&mut intro, " set up by the north windows, ", CharProps::default());
    add(&mut intro, "letterpress", italic());
    add(
        &mut intro,
        " regulars claim the back room, and somebody always brings bread. This handbook collects what we have learned about running a shared studio: the routines that keep it pleasant, the tools we pool, and the small agreements that let thirty strangers become a crew.",
        CharProps::default(),
    );
    b.push(Block::Para(intro));
    b.push(Block::Para(styled("Why share a studio?", "Heading1")));
    b.push(Block::Para(para("Rent is the obvious reason, but it is not the best one. A shared room puts your work next to work you would never have seen otherwise. A ceramicist explains glaze chemistry to a comic artist; a photographer borrows a weaver's loom for a still life. The cross-talk is the point.")));
    let mut quote = styled("A studio is a place where unfinished things are welcome.", "Quote");
    quote.props.space_before = Some(12.0);
    b.push(Block::Para(quote));
    b.push(Block::Para(styled("What we agree on", "Heading2")));
    let mut num = d.numbering.clone();
    let bullets = num.add_list(ListKind::Bullet);
    let steps = num.add_list(ListKind::Numbered);
    d.numbering = num;
    for (lead, rest) in [
        ("Clean as you go. ", "Leave a table the way you would like to find it."),
        ("Label what is yours. ", "Unlabelled supplies on the commons shelf are fair game."),
        ("Ask before you borrow. ", "Even from the shelf, a quick note helps everyone keep track."),
        ("Quiet hours ", "run from 9 to 11 on weekday mornings."),
    ] {
        let mut p = Paragraph::new().styled("ListParagraph");
        add(&mut p, lead, bold());
        add(&mut p, rest, CharProps::default());
        p.props.numbering = Some(NumRef { num: bullets, level: 0 });
        b.push(Block::Para(p));
    }
    b.push(Block::Para(styled("Shared equipment", "Heading2")));
    b.push(Block::Para(para("The table below lists what the membership owns together and who keeps each item in working order.")));
    let rows = [
        ["Equipment", "Location", "Steward", "Booking"],
        ["Etching press", "Back room", "Mara O.", "Sign-up sheet"],
        ["Kiln (cone 10)", "Yard shed", "Theo P.", "48 h notice"],
        ["Large-format printer", "Office", "Ines R.", "Calendar"],
        ["Light table", "North windows", "Anyone", "First come"],
    ];
    let mut t = Table::new(rows.len(), 4, 468.0);
    t.props.style = Some("GridTable4AccentBlue".into());
    for (r, row) in rows.iter().enumerate() {
        for (c, txt) in row.iter().enumerate() {
            if let Some(cell) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                cell.blocks = vec![para_block(para(txt))];
            }
        }
    }
    if let Some(r) = t.rows.first_mut() {
        r.props.header = true;
    }
    b.push(Block::Table(t));
    b.push(Block::Para(styled("Opening the studio", "Heading2")));
    for s in [
        "Switch on the main breaker by the door.",
        "Open both skylights unless it is raining.",
        "Start the kettle. This step is not optional.",
        "Check the booking sheet for the kiln and press.",
    ] {
        let mut p = para(s).styled("ListParagraph");
        p.props.numbering = Some(NumRef { num: steps, level: 0 });
        b.push(Block::Para(p));
    }
    b.push(Block::Para(styled("Membership", "Heading1")));
    let mut m = para("Membership is ");
    add(&mut m, "pay what you can", CharProps { bold: Some(true), color: Some(TextColor::Rgb(Rgb(0x15, 0x60, 0x82))), ..Default::default() });
    add(
        &mut m,
        ", reviewed every season. New members shadow a regular for their first two visits, and everyone takes one cleaning shift a month. If you have never made anything before, you are exactly who we hoped would show up.",
        CharProps::default(),
    );
    b.push(Block::Para(m));
    let mut h = para("Questions go to the front desk, or to the notice board by the sink. ");
    add(
        &mut h,
        "Visit the community board",
        CharProps { style: Some("Hyperlink".into()), link: Some("https://getartcraft.com/".into()), ..Default::default() },
    );
    add(&mut h, " for events.", CharProps::default());
    b.push(Block::Para(h));
    d.body = b.into_iter().map(std::sync::Arc::new).collect();
    // Footer: centred page number.
    let mut f = Paragraph::new().styled("Footer");
    f.props.align = Some(Align::Center);
    let _ = f.insert_object(
        0,
        InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    );
    let fid = d.add_part(PartKind::Footer, vec![para_block(f)]);
    d.last_section.footers.default = Some(fid);
    let mut hd = styled("The Open Studio Handbook\t\tSpring edition", "Header");
    hd.props.borders = Some(Borders {
        bottom: Some(Border { style: BorderStyle::Single, width: 0.5, color: Some(Rgb(0xBF, 0xBF, 0xBF)), space: 4.0 }),
        ..Default::default()
    });
    let hid = d.add_part(PartKind::Header, vec![para_block(hd)]);
    d.last_section.headers.default = Some(hid);
    d
}

/// A block-style letter.
pub fn letter() -> Document {
    let mut d = Document::new();
    let lines: Vec<Paragraph> = vec![
        styled("Your Name", "Title"),
        para("123 Your Street · City, ST 00000 · you@example.com"),
        para(""),
        para(&crate::cmd::insert::format_date("MMMM d, yyyy")),
        para(""),
        para("Recipient Name"),
        para("Title, Company"),
        para("Street Address"),
        para(""),
        para("Dear Recipient,"),
        para(
            "Start with a sentence that says why you are writing. Keep paragraphs short and specific, and close with what you would like to happen next.",
        ),
        para("Thank you for your time."),
        para("Sincerely,"),
        para(""),
        para("Your Name"),
    ];
    d.body = lines.into_iter().map(para_block).collect();
    d
}

/// A one-page résumé.
pub fn resume() -> Document {
    let mut d = Document::new();
    let mut b: Vec<Block> =
        vec![Block::Para(styled("Alex Rivera", "Title")), Block::Para(para("Illustrator · alex@example.com · portfolio.example.com"))];
    for (head, items) in [
        ("Experience", vec![("Lead Illustrator, Harbor Books", "2021 – present"), ("Freelance Illustrator", "2016 – 2021")]),
        ("Education", vec![("BFA Illustration, Coastal College of Art", "2016")]),
        ("Skills", vec![("Ink, gouache, digital painting, book layout, lettering", "")]),
    ] {
        b.push(Block::Para(styled(head, "Heading1")));
        for (what, when) in items {
            let mut p = para(what);
            p.props.tabs = Some(vec![TabStop { pos: 468.0, align: TabAlign::Right, leader: TabLeader::None }]);
            if !when.is_empty() {
                add(&mut p, &format!("\t{when}"), italic());
            }
            b.push(Block::Para(p));
        }
    }
    d.body = b.into_iter().map(std::sync::Arc::new).collect();
    d
}

/// A report with a cover heading and sections.
pub fn report() -> Document {
    let mut d = Document::new();
    let b: Vec<Block> = vec![
        Block::Para(styled("Report Title", "Title")),
        Block::Para(styled("Subtitle or date", "Subtitle")),
        Block::Para(styled("Summary", "Heading1")),
        Block::Para(para("Summarise the findings in two or three sentences.")),
        Block::Para(styled("Background", "Heading1")),
        Block::Para(para("Describe the context and the question the report answers.")),
        Block::Para(styled("Findings", "Heading1")),
        Block::Para(styled("First finding", "Heading2")),
        Block::Para(para("Explain the evidence.")),
        Block::Para(styled("Recommendations", "Heading1")),
        Block::Para(para("List what should happen next.")),
    ];
    d.body = b.into_iter().map(std::sync::Arc::new).collect();
    d
}

/// Words for [`large_document`]'s generated prose (our own list).
const WORDS: &[&str] = &[
    "the", "studio", "of", "a", "and", "light", "paper", "press", "river", "measure", "quiet", "build", "harbor", "to", "in", "with", "garden",
    "window", "ink", "season", "careful", "small", "table", "every", "morning", "letter", "colour", "wood", "stone", "is", "maker", "shared",
    "evening", "thread", "for", "carries", "north", "slowly", "against", "simple", "between", "patient", "kiln", "glaze", "drawing", "notebook",
    "bright", "under", "map", "lantern", "bridge", "orchard", "copper", "weaving", "from", "over",
];

/// A deterministic run of `n` words (`seed` picks which) forming sentences.
fn prose(seed: &mut u64, n: usize) -> String {
    let mut s = String::new();
    let mut in_sentence = 0;
    for i in 0..n {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let w = WORDS.get(((*seed >> 33) as usize) % WORDS.len()).copied().unwrap_or("word");
        if in_sentence == 0 {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                s.extend(f.to_uppercase());
                s.push_str(c.as_str());
            }
        } else {
            s.push_str(w);
        }
        in_sentence += 1;
        if in_sentence > 8 + (*seed >> 60) as usize || i + 1 == n {
            s.push('.');
            in_sentence = 0;
        }
        if i + 1 < n {
            s.push(' ');
        }
    }
    s
}

/// A `w` × `h` PNG with a gradient of its own (`k` varies the colours), so each picture in
/// [`large_document`] is distinct media that has to be decoded on its own.
fn picture_png(k: u32, w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([((x * 255) / w.max(1)) as u8, ((y * 255) / h.max(1)) as u8, (k.wrapping_mul(37) % 256) as u8])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    match img.write_to(&mut out, image::ImageFormat::Png) {
        Ok(()) => out.into_inner(),
        Err(_) => Vec::new(),
    }
}

/// A large synthetic book of about `pages` pages (#554): chapters with headings, long
/// paragraphs, a picture every few pages (some floating with square wrapping), tables,
/// footnotes, fields (a TOC, PAGE/NUMPAGES in the footer, cross-page REF-like fields) and
/// numbered lists. Deterministic; for benchmarks and performance tests. Sizes are clamped
/// (1..=5000 pages).
pub fn large_document(pages: usize) -> Document {
    use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap};
    let pages = pages.clamp(1, 5000);
    let mut d = Document::new();
    d.core.title = "A Long Synthetic Book".into();
    d.core.creator = "WordCraft".into();
    let mut seed = 0x5eed_u64;
    let mut b: Vec<Block> = Vec::new();
    b.push(Block::Para(styled("A Long Synthetic Book", "Title")));
    let mut toc = Paragraph::new();
    let _ = toc.insert_object(
        0,
        InlineObject::Field {
            instr: "TOC \\o \"1-2\" \\h".into(),
            result: String::new(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &CharProps::default(),
    );
    b.push(Block::Para(toc));
    let mut num = d.numbering.clone();
    let steps = num.add_list(ListKind::Numbered);
    d.numbering = num;
    // About 3.5 paragraphs of ~110 words fill a page; pictures every 2.5 pages, tables every 5.
    let paras = pages.saturating_mul(3);
    let (pictures, tables) = (pages * 2 / 5, pages / 5);
    let (mut pic, mut tab) = (0usize, 0usize);
    let chapter_len = 60;
    for i in 0..paras {
        if i % chapter_len == 0 {
            let ch = i / chapter_len + 1;
            b.push(Block::Para(styled(&format!("Chapter {ch}: {}", prose(&mut seed, 4).trim_end_matches('.')), "Heading1")));
        } else if i % 15 == 0 {
            b.push(Block::Para(styled(prose(&mut seed, 5).trim_end_matches('.'), "Heading2")));
        }
        let mut p = para(&prose(&mut seed, 90 + (i * 7) % 50));
        if i % 6 == 3 {
            // A footnote.
            let id = d.add_part(PartKind::Footnote, Vec::new());
            let mut n = Paragraph::new().styled("FootnoteText");
            let refp = CharProps { style: Some("FootnoteReference".into()), ..Default::default() };
            let _ = n.insert_object(0, InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() }, &refp);
            add(&mut n, &format!(" {}", prose(&mut seed, 25)), CharProps::default());
            let _ = d.set_story(wordcraft_doc::StoryRef::Part(id), vec![para_block(n)]);
            let at = p.len();
            let _ = p.insert_object(at, InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() }, &refp);
        }
        if i % 10 == 7 {
            add(&mut p, " Today is ", CharProps::default());
            let at = p.len();
            let _ = p.insert_object(
                at,
                InlineObject::Field {
                    instr: "DATE \\@ \"MMMM d, yyyy\"".into(),
                    result: "May 1, 2026".into(),
                    locked: false,
                    dirty: false,
                    code: false,
                    form: String::new(),
                },
                &CharProps::default(),
            );
            add(&mut p, ".", CharProps::default());
        }
        if i % 4 == 1 {
            add(&mut p, &format!(" {}", prose(&mut seed, 12)), bold());
        }
        b.push(Block::Para(p));
        if i % 25 == 12 {
            for _ in 0..4 {
                let mut l = para(&prose(&mut seed, 14)).styled("ListParagraph");
                l.props.numbering = Some(NumRef { num: steps, level: 0 });
                b.push(Block::Para(l));
            }
        }
        if pic < pictures && i * pictures / paras.max(1) >= pic {
            let media = d.add_media(picture_png(pic as u32, 320 + (pic as u32 % 5) * 40, 240), "png");
            let floating = pic % 4 == 1;
            let float = if floating {
                Float {
                    wrap: Wrap::Square,
                    h_rel: Anchor::Column,
                    v_rel: Anchor::Paragraph,
                    h_align: Some(FloatAlign::End),
                    dist: 9.0,
                    ..Default::default()
                }
            } else {
                Float::default()
            };
            let (w, h) = if floating { (180.0, 135.0) } else { (360.0, 240.0) };
            let mut fig = if floating { para(&prose(&mut seed, 60)) } else { Paragraph::new() };
            let _ = fig.insert_object(
                0,
                InlineObject::Image { media, w, h, alt: format!("Figure {}", pic + 1), float, crop: [0.0; 4], ole: None },
                &CharProps::default(),
            );
            b.push(Block::Para(fig));
            b.push(Block::Para(styled(&format!("Figure {}: {}", pic + 1, prose(&mut seed, 8)), "Caption")));
            pic += 1;
        }
        if tab < tables && i * tables / paras.max(1) >= tab && i % 3 == 2 {
            let (rows, cols) = (6 + tab % 6, 4);
            let mut t = Table::new(rows, cols, 468.0);
            for (r, row) in t.rows.iter_mut().enumerate() {
                for cell in row.cells.iter_mut() {
                    cell.blocks = vec![para_block(para(&prose(&mut seed, if r == 0 { 2 } else { 3 + r % 5 })))];
                }
            }
            if let Some(r) = t.rows.first_mut() {
                r.props.header = true;
            }
            b.push(Block::Table(t));
            tab += 1;
        }
    }
    d.body = b.into_iter().map(std::sync::Arc::new).collect();
    let mut f = Paragraph::new().styled("Footer");
    f.props.align = Some(Align::Center);
    add(&mut f, "Page ", CharProps::default());
    let at = f.len();
    let _ = f.insert_object(
        at,
        InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    );
    add(&mut f, " of ", CharProps::default());
    let at = f.len();
    let _ = f.insert_object(
        at,
        InlineObject::Field { instr: "NUMPAGES".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    );
    let fid = d.add_part(PartKind::Footer, vec![para_block(f)]);
    d.last_section.footers.default = Some(fid);
    let hid = d.add_part(PartKind::Header, vec![para_block(styled("A Long Synthetic Book", "Header"))]);
    d.last_section.headers.default = Some(hid);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_valid() {
        assert_eq!(sample_document().word_count(), 361);
        for d in [sample_document(), letter(), resume(), report()] {
            assert!(d.word_count() > 5);
            let l = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
            assert!(!l.pages.is_empty());
        }
    }
}
