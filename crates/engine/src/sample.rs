//! Built-in sample documents and templates (original text, generated in code), in English and
//! German. German templates are our own German text in the same layout, on German Word's page
//! defaults (A4, 2.5 cm margins and 2 cm at the bottom) and marked as German (`de-DE`).

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

/// The language a template is written in (`file.new {"language": "de"}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    English,
    German,
}

impl Lang {
    /// A language tag (`de`, `de-AT`, `en-US` …). Languages without templates of their own get
    /// the English ones.
    pub fn from_tag(tag: &str) -> Lang {
        if tag.split(['-', '_']).next().is_some_and(|primary| primary.eq_ignore_ascii_case("de")) { Lang::German } else { Lang::English }
    }

    /// The English or the German text.
    fn pick(self, en: &'static str, de: &'static str) -> &'static str {
        match self {
            Lang::English => en,
            Lang::German => de,
        }
    }
}

/// A template by name (`blank`, `sample`, `letter`, `resume`, `report`; anything else is blank).
pub fn template(name: &str, lang: Lang) -> Document {
    let mut d = match name {
        "sample" => handbook(lang),
        "letter" => letter_in(lang),
        "resume" => resume_in(lang),
        "report" => report_in(lang),
        _ => Document::new(),
    };
    if lang == Lang::German {
        // German Word's defaults: A4; 2.5 cm margins, 2 cm at the bottom; header and footer
        // 1.25 cm from the edge. The language keeps proofing from checking German text against
        // the English word list.
        let cm = 72.0 / 2.54;
        let sect = &mut d.last_section;
        (sect.page_w, sect.page_h) = (595.28, 841.89);
        (sect.margin_top, sect.margin_bottom, sect.margin_left, sect.margin_right) = (2.5 * cm, 2.0 * cm, 2.5 * cm, 2.5 * cm);
        (sect.header, sect.footer) = (1.25 * cm, 1.25 * cm);
        d.styles.default_chr.lang = Some("de-DE".into());
    }
    d
}

/// Today as a letter writes it: `October 10, 2026` or `10. Oktober 2026`.
fn letter_date(lang: Lang) -> String {
    match lang {
        Lang::English => crate::cmd::insert::format_date("MMMM d, yyyy"),
        Lang::German => crate::cmd::insert::format_date_in("d. MMMM yyyy", Lang::German),
    }
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
    handbook(Lang::English)
}

fn handbook(lang: Lang) -> Document {
    let l = |en, de| lang.pick(en, de);
    let mut d = Document::new();
    let title = l("The Open Studio Handbook", "Handbuch für das offene Atelier");
    d.core.title = title.into();
    d.core.creator = "WordCraft".into();
    let mut b: Vec<Block> = Vec::new();
    b.push(Block::Para(styled(title, "Title")));
    b.push(Block::Para(styled(
        l("How a small community of makers shares space, tools and time", "Wie eine kleine Gemeinschaft von Kreativen Raum, Werkzeug und Zeit teilt"),
        "Subtitle",
    )));
    let mut intro = para(l(
        "Every Thursday evening the old print shop on Alder Street fills with people who make things. ",
        "Jeden Donnerstagabend füllt sich die alte Druckerei in der Erlenstraße mit Menschen, die Dinge herstellen. ",
    ));
    add(&mut intro, l("Painters", "Malerinnen und Maler"), bold());
    add(&mut intro, l(" set up by the north windows, ", " bauen an den Nordfenstern auf, die "), CharProps::default());
    add(&mut intro, l("letterpress", "Buchdruck"), italic());
    add(
        &mut intro,
        l(
            " regulars claim the back room, and somebody always brings bread. This handbook collects what we have learned about running a shared studio: the routines that keep it pleasant, the tools we pool, and the small agreements that let thirty strangers become a crew.",
            "-Stammgäste besetzen den Hinterraum, und irgendwer bringt immer Brot mit. Dieses Handbuch sammelt, was wir über den Betrieb eines gemeinsamen Ateliers gelernt haben: die Abläufe, die es angenehm machen, die Werkzeuge, die wir teilen, und die kleinen Vereinbarungen, durch die aus dreißig Fremden eine Mannschaft wird.",
        ),
        CharProps::default(),
    );
    b.push(Block::Para(intro));
    b.push(Block::Para(styled(l("Why share a studio?", "Warum ein Atelier teilen?"), "Heading1")));
    b.push(Block::Para(para(l(
        "Rent is the obvious reason, but it is not the best one. A shared room puts your work next to work you would never have seen otherwise. A ceramicist explains glaze chemistry to a comic artist; a photographer borrows a weaver's loom for a still life. The cross-talk is the point.",
        "Die Miete ist der naheliegende Grund, aber nicht der beste. In einem gemeinsamen Raum steht Ihre Arbeit neben Arbeiten, die Sie sonst nie gesehen hätten. Eine Keramikerin erklärt einem Comiczeichner die Chemie der Glasuren; ein Fotograf leiht sich für ein Stillleben den Webstuhl einer Weberin. Genau um diesen Austausch geht es.",
    ))));
    let mut quote =
        styled(l("A studio is a place where unfinished things are welcome.", "Ein Atelier ist ein Ort, an dem Unfertiges willkommen ist."), "Quote");
    quote.props.space_before = Some(12.0);
    b.push(Block::Para(quote));
    b.push(Block::Para(styled(l("What we agree on", "Worauf wir uns einigen"), "Heading2")));
    let mut num = d.numbering.clone();
    let bullets = num.add_list(ListKind::Bullet);
    let steps = num.add_list(ListKind::Numbered);
    d.numbering = num;
    let agreements = match lang {
        Lang::English => [
            ("Clean as you go. ", "Leave a table the way you would like to find it."),
            ("Label what is yours. ", "Unlabelled supplies on the commons shelf are fair game."),
            ("Ask before you borrow. ", "Even from the shelf, a quick note helps everyone keep track."),
            ("Quiet hours ", "run from 9 to 11 on weekday mornings."),
        ],
        Lang::German => [
            ("Gleich aufräumen. ", "Hinterlassen Sie einen Tisch so, wie Sie ihn gern vorfinden würden."),
            ("Eigenes beschriften. ", "Unbeschriftetes Material im Gemeinschaftsregal darf jeder nutzen."),
            ("Erst fragen, dann leihen. ", "Auch beim Regal hilft eine kurze Notiz, den Überblick zu behalten."),
            ("Ruhezeiten ", "gelten werktags von 9 bis 11 Uhr."),
        ],
    };
    for (lead, rest) in agreements {
        let mut p = Paragraph::new().styled("ListParagraph");
        add(&mut p, lead, bold());
        add(&mut p, rest, CharProps::default());
        p.props.numbering = Some(NumRef { num: bullets, level: 0 });
        b.push(Block::Para(p));
    }
    b.push(Block::Para(styled(l("Shared equipment", "Gemeinsame Ausstattung"), "Heading2")));
    b.push(Block::Para(para(l(
        "The table below lists what the membership owns together and who keeps each item in working order.",
        "Die folgende Tabelle zeigt, was den Mitgliedern gemeinsam gehört und wer sich um welches Gerät kümmert.",
    ))));
    let rows = match lang {
        Lang::English => [
            ["Equipment", "Location", "Steward", "Booking"],
            ["Etching press", "Back room", "Mara O.", "Sign-up sheet"],
            ["Kiln (cone 10)", "Yard shed", "Theo P.", "48 h notice"],
            ["Large-format printer", "Office", "Ines R.", "Calendar"],
            ["Light table", "North windows", "Anyone", "First come"],
        ],
        Lang::German => [
            ["Gerät", "Standort", "Zuständig", "Buchung"],
            ["Radierpresse", "Hinterraum", "Mara O.", "Eintragsliste"],
            ["Brennofen (Kegel 10)", "Schuppen im Hof", "Theo P.", "48 h vorher"],
            ["Großformatdrucker", "Büro", "Ines R.", "Kalender"],
            ["Leuchttisch", "Nordfenster", "Alle", "Wer zuerst kommt"],
        ],
    };
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
    b.push(Block::Para(styled(l("Opening the studio", "Das Atelier aufschließen"), "Heading2")));
    let steps_text = match lang {
        Lang::English => [
            "Switch on the main breaker by the door.",
            "Open both skylights unless it is raining.",
            "Start the kettle. This step is not optional.",
            "Check the booking sheet for the kiln and press.",
        ],
        Lang::German => [
            "Den Hauptschalter neben der Tür einschalten.",
            "Beide Oberlichter öffnen, außer es regnet.",
            "Wasserkocher anstellen. Dieser Schritt ist Pflicht.",
            "Die Buchungsliste für Brennofen und Presse prüfen.",
        ],
    };
    for s in steps_text {
        let mut p = para(s).styled("ListParagraph");
        p.props.numbering = Some(NumRef { num: steps, level: 0 });
        b.push(Block::Para(p));
    }
    b.push(Block::Para(styled(l("Membership", "Mitgliedschaft"), "Heading1")));
    let mut m = para(l("Membership is ", "Der Mitgliedsbeitrag ist "));
    add(
        &mut m,
        l("pay what you can", "so viel, wie Sie können"),
        CharProps { bold: Some(true), color: Some(TextColor::Rgb(Rgb(0x15, 0x60, 0x82))), ..Default::default() },
    );
    add(
        &mut m,
        l(
            ", reviewed every season. New members shadow a regular for their first two visits, and everyone takes one cleaning shift a month. If you have never made anything before, you are exactly who we hoped would show up.",
            ", überprüft wird er jede Saison. Neue Mitglieder begleiten bei ihren ersten beiden Besuchen ein erfahrenes Mitglied, und alle übernehmen einmal im Monat eine Putzschicht. Wenn Sie noch nie etwas selbst gemacht haben, sind Sie genau die Person, auf die wir gehofft haben.",
        ),
        CharProps::default(),
    );
    b.push(Block::Para(m));
    let mut h = para(l(
        "Questions go to the front desk, or to the notice board by the sink. ",
        "Fragen gehen an den Empfang oder an das Schwarze Brett beim Waschbecken. Termine finden Sie auf der ",
    ));
    add(
        &mut h,
        l("Visit the community board", "Community-Pinnwand"),
        CharProps { style: Some("Hyperlink".into()), link: Some("https://getartcraft.com/".into()), ..Default::default() },
    );
    add(&mut h, l(" for events.", "."), CharProps::default());
    b.push(Block::Para(h));
    d.body = b.into_iter().map(std::sync::Arc::new).collect();
    // Footer: centred page number.
    let mut f = Paragraph::new().styled("Footer");
    f.props.align = Some(Align::Center);
    let _ = f.insert_object(0, InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false }, &CharProps::default());
    let fid = d.add_part(PartKind::Footer, vec![para_block(f)]);
    d.last_section.footers.default = Some(fid);
    let mut hd = styled(&format!("{title}\t\t{}", l("Spring edition", "Frühjahrsausgabe")), "Header");
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
    letter_in(Lang::English)
}

fn letter_in(lang: Lang) -> Document {
    let mut d = Document::new();
    let lines: Vec<Paragraph> = match lang {
        Lang::English => vec![
            styled("Your Name", "Title"),
            para("123 Your Street · City, ST 00000 · you@example.com"),
            para(""),
            para(&letter_date(lang)),
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
        ],
        // A German business letter: recipient block, place and date on the right, a bold
        // subject line, no comma after the closing, and the text after the salutation starting
        // in lower case.
        Lang::German => {
            let mut date = para(&format!("Musterstadt, {}", letter_date(lang)));
            date.props.align = Some(Align::Right);
            let mut subject = Paragraph::new();
            add(&mut subject, "Betreff", bold());
            vec![
                styled("Ihr Name", "Title"),
                para("Musterstraße 1 · 12345 Musterstadt · ihr.name@example.com"),
                para(""),
                para("Firma"),
                para("Name der Empfängerin oder des Empfängers"),
                para("Straße und Hausnummer"),
                para("PLZ Ort"),
                para(""),
                date,
                para(""),
                subject,
                para(""),
                para("Sehr geehrte Damen und Herren,"),
                para(
                    "beginnen Sie mit einem Satz, der sagt, warum Sie schreiben. Halten Sie die Absätze kurz und konkret, und schließen Sie mit dem, was als Nächstes geschehen soll.",
                ),
                para("Mit freundlichen Grüßen"),
                para(""),
                para("Ihr Name"),
            ]
        }
    };
    d.body = lines.into_iter().map(para_block).collect();
    d
}

/// A one-page résumé.
pub fn resume() -> Document {
    resume_in(Lang::English)
}

fn resume_in(lang: Lang) -> Document {
    let mut d = Document::new();
    let (name, contact, sections) = match lang {
        Lang::English => (
            "Alex Rivera",
            "Illustrator · alex@example.com · portfolio.example.com",
            [
                ("Experience", vec![("Lead Illustrator, Harbor Books", "2021 – present"), ("Freelance Illustrator", "2016 – 2021")]),
                ("Education", vec![("BFA Illustration, Coastal College of Art", "2016")]),
                ("Skills", vec![("Ink, gouache, digital painting, book layout, lettering", "")]),
            ],
        ),
        Lang::German => (
            "Erika Mustermann",
            "Illustratorin · erika@example.com · portfolio.example.com",
            [
                ("Berufserfahrung", vec![("Leitende Illustratorin, Hafenverlag", "2021 – heute"), ("Freiberufliche Illustratorin", "2016 – 2021")]),
                ("Ausbildung", vec![("B.A. Illustration, Kunsthochschule an der Küste", "2016")]),
                ("Kenntnisse", vec![("Tusche, Gouache, digitale Malerei, Buchgestaltung, Lettering", "")]),
            ],
        ),
    };
    // The date column's right tab sits at the right margin (A4 is narrower than Letter).
    let right_tab = if lang == Lang::German { 595.28 - 2.0 * 2.5 * 72.0 / 2.54 } else { 468.0 };
    let mut b: Vec<Block> = vec![Block::Para(styled(name, "Title")), Block::Para(para(contact))];
    for (head, items) in sections {
        b.push(Block::Para(styled(head, "Heading1")));
        for (what, when) in items {
            let mut p = para(what);
            p.props.tabs = Some(vec![TabStop { pos: right_tab, align: TabAlign::Right, leader: TabLeader::None }]);
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
    report_in(Lang::English)
}

fn report_in(lang: Lang) -> Document {
    let l = |en, de| lang.pick(en, de);
    let mut d = Document::new();
    let b: Vec<Block> = vec![
        Block::Para(styled(l("Report Title", "Titel des Berichts"), "Title")),
        Block::Para(styled(l("Subtitle or date", "Untertitel oder Datum"), "Subtitle")),
        Block::Para(styled(l("Summary", "Zusammenfassung"), "Heading1")),
        Block::Para(para(l("Summarise the findings in two or three sentences.", "Fassen Sie die Ergebnisse in zwei oder drei Sätzen zusammen."))),
        Block::Para(styled(l("Background", "Hintergrund"), "Heading1")),
        Block::Para(para(l(
            "Describe the context and the question the report answers.",
            "Beschreiben Sie den Zusammenhang und die Frage, die der Bericht beantwortet.",
        ))),
        Block::Para(styled(l("Findings", "Ergebnisse"), "Heading1")),
        Block::Para(styled(l("First finding", "Erstes Ergebnis"), "Heading2")),
        Block::Para(para(l("Explain the evidence.", "Erläutern Sie die Belege."))),
        Block::Para(styled(l("Recommendations", "Empfehlungen"), "Heading1")),
        Block::Para(para(l("List what should happen next.", "Führen Sie auf, was als Nächstes geschehen soll."))),
    ];
    d.body = b.into_iter().map(std::sync::Arc::new).collect();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_valid() {
        for lang in [Lang::English, Lang::German] {
            for name in ["sample", "letter", "resume", "report"] {
                let d = template(name, lang);
                assert!(d.word_count() > 5, "{name} {lang:?}");
                let l = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
                assert!(!l.pages.is_empty());
            }
        }
    }

    #[test]
    fn german_templates_are_german_a4_documents() {
        for name in ["blank", "sample", "letter", "resume", "report"] {
            let d = template(name, Lang::German);
            assert_eq!(d.styles.default_chr.lang.as_deref(), Some("de-DE"), "{name}");
            assert_eq!(wordcraft_geom::paper_name(d.last_section.page_w, d.last_section.page_h), Some("A4"), "{name}");
            assert!((d.last_section.margin_left - 70.87).abs() < 0.01 && (d.last_section.margin_bottom - 56.69).abs() < 0.01);
        }
        let letter = template("letter", Lang::German).plain_text(wordcraft_doc::StoryRef::Body);
        assert!(letter.contains("Sehr geehrte Damen und Herren,") && letter.contains("Mit freundlichen Grüßen"));
        assert!(!letter.contains("Mit freundlichen Grüßen,"), "no comma after a German closing");
        let date = letter_date(Lang::German);
        assert!(date.contains(". ") && !date.contains(','), "{date}");
        // English stays as it was: US Letter, no explicit language change.
        let en = template("letter", Lang::English);
        assert_eq!(wordcraft_geom::paper_name(en.last_section.page_w, en.last_section.page_h), Some("Letter"));
        assert_eq!(en.styles.default_chr.lang.as_deref(), Some("en-US"));
        assert_eq!(Lang::from_tag("de-AT"), Lang::German);
        assert_eq!(Lang::from_tag("de_CH.UTF-8"), Lang::German);
        assert_eq!(Lang::from_tag("en-US"), Lang::English);
        assert_eq!(Lang::from_tag("ja"), Lang::English);
        assert_eq!(Lang::from_tag("dex"), Lang::English);
    }
}
