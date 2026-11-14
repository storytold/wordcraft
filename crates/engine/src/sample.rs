//! Built-in sample documents and templates (original text, generated in code).

use std::sync::Arc;
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
    let mut num = (*d.numbering).clone();
    let bullets = num.add_list(ListKind::Bullet);
    let steps = num.add_list(ListKind::Numbered);
    d.numbering = Arc::new(num);
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

/// The body text width of a default page (8.5 in less two 1 in margins), in points.
const TEXT_W: f32 = 468.0;

/// A table of plain text cells in `style`, with column widths in points. Cells in `right`
/// columns are right-aligned (amounts); the first row repeats as a header row.
fn grid(rows: &[&[&str]], widths: &[f32], style: &str, right: &[usize]) -> Table {
    let mut t = Table::new(rows.len(), widths.len(), widths.iter().sum());
    t.props.style = Some(style.into());
    t.grid = widths.to_vec();
    for (r, row) in rows.iter().enumerate() {
        for (c, txt) in row.iter().enumerate() {
            if let Some(cell) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                let mut p = para(txt);
                if right.contains(&c) {
                    p.props.align = Some(Align::Right);
                }
                cell.props.width = widths.get(c).copied();
                cell.blocks = vec![para_block(p)];
            }
        }
    }
    if let Some(r) = t.rows.first_mut() {
        r.props.header = true;
    }
    // Plain rows: only the header row takes the style's emphasis.
    t.props.look.first_column = false;
    t
}

/// Bold the last row's cells from column `from` on (a totals row).
fn bold_last_row(t: &mut Table, from: usize) {
    if let Some(total) = t.rows.last_mut() {
        for cell in total.cells.iter_mut().skip(from) {
            if let Some(Block::Para(p)) = cell.blocks.first_mut().map(Arc::make_mut) {
                let n = p.len();
                let _ = p.format(0, n, &|c| c.bold = Some(true));
            }
        }
    }
}

/// A "Label:<tab>value" line, the label bold, the value starting at a fixed tab stop.
fn field_line(label: &str, value: &str) -> Paragraph {
    let mut p = Paragraph::new();
    add(&mut p, label, bold());
    add(&mut p, &format!("\t{value}"), CharProps::default());
    p.props.tabs = Some(vec![TabStop { pos: 90.0, align: TabAlign::Left, leader: TabLeader::None }]);
    p.props.space_after = Some(4.0);
    p
}

/// A thin rule under a paragraph.
fn rule_below(p: &mut Paragraph, width: f32, color: Rgb) {
    p.props.borders =
        Some(Borders { bottom: Some(Border { style: BorderStyle::Single, width, color: Some(color), space: 4.0 }), ..Default::default() });
}

/// Paragraphs of `items` as one list of `kind`, styled as list paragraphs.
fn list(d: &mut Document, kind: ListKind, items: &[&str]) -> Vec<Block> {
    let mut num = (*d.numbering).clone();
    let id = num.add_list(kind);
    d.numbering = Arc::new(num);
    items
        .iter()
        .map(|s| {
            let mut p = para(s).styled("ListParagraph");
            p.props.numbering = Some(NumRef { num: id, level: 0 });
            Block::Para(p)
        })
        .collect()
}

fn finish(d: &mut Document, title: &str, blocks: Vec<Block>) {
    d.core.title = title.into();
    d.body = blocks.into_iter().map(Arc::new).collect();
}

/// An interoffice memo: a MEMO heading, To/From/Date/Subject lines and a short body.
pub fn memo() -> Document {
    let mut d = Document::new();
    let mut head = styled("Memo", "Title");
    rule_below(&mut head, 1.5, Rgb(0x15, 0x60, 0x82));
    let date = crate::cmd::insert::format_date("MMMM d, yyyy");
    let mut subject = field_line("Subject:", "Office move to the third floor");
    rule_below(&mut subject, 0.5, Rgb(0xBF, 0xBF, 0xBF));
    subject.props.space_after = Some(18.0);
    let mut b = vec![
        Block::Para(head),
        Block::Para(field_line("To:", "All staff")),
        Block::Para(field_line("From:", "Facilities team")),
        Block::Para(field_line("Cc:", "Department leads")),
        Block::Para(field_line("Date:", &date)),
        Block::Para(subject),
        Block::Para(para(
            "Our team moves to the third floor next month. This memo explains what changes, what you need to do before moving day, and who to ask for help.",
        )),
        Block::Para(styled("Before moving day", "Heading2")),
    ];
    b.extend(list(
        &mut d,
        ListKind::Bullet,
        &[
            "Pack your desk into the crates provided and label each one with your name.",
            "Back up anything stored only on your desktop computer.",
            "Return borrowed equipment to the supply room.",
        ],
    ));
    b.push(Block::Para(styled("On the day", "Heading2")));
    b.push(Block::Para(para("Movers arrive at 8:00. Work from home if you can; the new floor opens at 13:00 with desks, network and phones ready.")));
    b.push(Block::Para(para("Send questions to the facilities desk by the end of this week.")));
    finish(&mut d, "Memo", b);
    d
}

/// A meeting agenda: meeting details, an objective, a timed agenda table and preparation notes.
pub fn agenda() -> Document {
    let mut d = Document::new();
    let date = crate::cmd::insert::format_date("dddd, MMMM d, yyyy");
    let mut b = vec![
        Block::Para(styled("Team Meeting Agenda", "Title")),
        Block::Para(styled("Quarterly planning", "Subtitle")),
        Block::Para(field_line("Date:", &date)),
        Block::Para(field_line("Time:", "10:00 – 11:30")),
        Block::Para(field_line("Location:", "Room 4B and video call")),
        Block::Para(field_line("Facilitator:", "Jordan Lee")),
        Block::Para(field_line("Note taker:", "Sam Patel")),
        Block::Para(styled("Objective", "Heading1")),
        Block::Para(para("Agree on the three priorities for next quarter and who owns each one.")),
        Block::Para(styled("Agenda", "Heading1")),
        Block::Table(grid(
            &[
                &["Time", "Item", "Lead", "Outcome"],
                &["10:00", "Welcome and review of last actions", "Jordan", "Update"],
                &["10:10", "Results from this quarter", "Priya", "Discussion"],
                &["10:35", "Proposed priorities", "Sam", "Decision"],
                &["11:05", "Owners and next steps", "Jordan", "Decision"],
                &["11:25", "Any other business", "All", "—"],
            ],
            &[60.0, 216.0, 84.0, 108.0],
            "GridTable4AccentBlue",
            &[],
        )),
        Block::Para(styled("Please prepare", "Heading1")),
    ];
    b.extend(list(
        &mut d,
        ListKind::Bullet,
        &["Read the quarterly results summary before the meeting.", "Bring one proposal for a priority, with a rough estimate of effort."],
    ));
    finish(&mut d, "Meeting Agenda", b);
    d
}

/// Meeting minutes: attendees, discussion, decisions and an action-item table.
pub fn minutes() -> Document {
    let mut d = Document::new();
    let date = crate::cmd::insert::format_date("MMMM d, yyyy");
    let mut b = vec![
        Block::Para(styled("Meeting Minutes", "Title")),
        Block::Para(styled("Neighbourhood Association · Monthly meeting", "Subtitle")),
        Block::Para(field_line("Date:", &date)),
        Block::Para(field_line("Present:", "R. Okafor (chair), L. Moreau, D. Kim, A. Novak, T. Haddad")),
        Block::Para(field_line("Apologies:", "M. Santos")),
        Block::Para(field_line("Minutes:", "D. Kim")),
        Block::Para(styled("1. Approval of last minutes", "Heading2")),
        Block::Para(para("The minutes of the previous meeting were approved without changes.")),
        Block::Para(styled("2. Community garden", "Heading2")),
        Block::Para(para(
            "L. Moreau reported that the raised beds are finished and twelve households have signed up. Water access remains the main open question.",
        )),
        Block::Para(styled("3. Autumn street fair", "Heading2")),
        Block::Para(para("The committee discussed dates and a budget for stalls, music and cleanup.")),
        Block::Para(styled("Decisions", "Heading1")),
    ];
    b.extend(list(
        &mut d,
        ListKind::Numbered,
        &["The street fair will be held on the last Saturday of September.", "A budget of up to 800 is approved for the fair."],
    ));
    b.push(Block::Para(styled("Action items", "Heading1")));
    b.push(Block::Table(grid(
        &[
            &["Action", "Owner", "Due"],
            &["Ask the city about a garden water tap", "L. Moreau", "Next meeting"],
            &["Book the stage and sound system", "A. Novak", "In two weeks"],
            &["Draft the fair flyer", "T. Haddad", "End of month"],
        ],
        &[264.0, 102.0, 102.0],
        "GridTable4AccentGreen",
        &[],
    )));
    let mut next = para("Next meeting: ");
    add(&mut next, "first Tuesday of next month, 19:00, community hall.", bold());
    next.props.space_before = Some(12.0);
    b.push(Block::Para(next));
    finish(&mut d, "Meeting Minutes", b);
    d
}

/// A service invoice: sender, bill-to, invoice details, line items with totals and payment terms.
pub fn invoice() -> Document {
    let mut d = Document::new();
    let date = crate::cmd::insert::format_date("MMMM d, yyyy");
    let accent = Rgb(0x15, 0x60, 0x82);
    let mut head = Paragraph::new();
    add(&mut head, "Your Company", CharProps { bold: Some(true), size: Some(20.0), ..Default::default() });
    add(&mut head, "\tINVOICE", CharProps { bold: Some(true), size: Some(24.0), color: Some(TextColor::Rgb(accent)), ..Default::default() });
    head.props.tabs = Some(vec![TabStop { pos: TEXT_W, align: TabAlign::Right, leader: TabLeader::None }]);
    rule_below(&mut head, 1.5, accent);
    let right_tab = |left: &str, right: &str| {
        let mut p = para(&format!("{left}\t{right}"));
        p.props.tabs = Some(vec![TabStop { pos: TEXT_W, align: TabAlign::Right, leader: TabLeader::None }]);
        p.props.space_after = Some(0.0);
        p
    };
    let mut bill = Paragraph::new();
    add(&mut bill, "Bill to", bold());
    bill.props.space_before = Some(12.0);
    bill.props.space_after = Some(0.0);
    let mut b = vec![
        Block::Para(head),
        Block::Para(right_tab("123 Your Street, City, ST 00000", "Invoice no.: 0001")),
        Block::Para(right_tab("billing@example.com · (555) 010-0000", &format!("Date: {date}"))),
        Block::Para(right_tab("", "Due: 30 days from the invoice date")),
        Block::Para(bill),
        Block::Para(right_tab("Client Name", "")),
        Block::Para(right_tab("Client Company", "")),
        Block::Para(right_tab("Client Street Address, City, ST 00000", "")),
        Block::Para(para("")),
    ];
    let mut items = grid(
        &[
            &["Description", "Qty", "Unit price", "Amount"],
            &["Website design, home page and four inner pages", "1", "1,200.00", "1,200.00"],
            &["Copywriting (per page)", "5", "80.00", "400.00"],
            &["Stock photography licences", "6", "15.00", "90.00"],
            &["Hosting setup", "1", "60.00", "60.00"],
            &["", "", "Subtotal", "1,750.00"],
            &["", "", "Tax (10%)", "175.00"],
            &["", "", "Total due", "1,925.00"],
        ],
        &[240.0, 48.0, 90.0, 90.0],
        "ListTable3AccentBlue",
        &[1, 2, 3],
    );
    bold_last_row(&mut items, 2);
    b.push(Block::Table(items));
    b.push(Block::Para(styled("Payment terms", "Heading2")));
    b.push(Block::Para(para("Payment is due within 30 days. Please include the invoice number with your payment.")));
    b.push(Block::Para(para("Bank: Example Bank · Account name: Your Company · Account no.: 00000000")));
    let mut thanks = Paragraph::new();
    add(&mut thanks, "Thank you for your business!", italic());
    thanks.props.align = Some(Align::Center);
    thanks.props.space_before = Some(18.0);
    b.push(Block::Para(thanks));
    finish(&mut d, "Invoice", b);
    d
}

/// A job-application cover letter.
pub fn cover_letter() -> Document {
    let mut d = Document::new();
    let mut name = styled("Your Name", "Title");
    rule_below(&mut name, 0.75, Rgb(0xBF, 0xBF, 0xBF));
    let lines: Vec<Paragraph> = vec![
        name,
        para("City, ST · (555) 010-0000 · you@example.com · linkedin.example.com/in/you"),
        para(""),
        para(&crate::cmd::insert::format_date("MMMM d, yyyy")),
        para(""),
        para("Hiring Manager"),
        para("Company Name"),
        para("Street Address, City, ST 00000"),
        para(""),
        para("Dear Hiring Manager,"),
        para(
            "I am writing to apply for the Project Coordinator role advertised on your careers page. With four years of experience keeping teams, schedules and budgets on track, I would welcome the chance to bring that to your organisation.",
        ),
        para(
            "In my current role I coordinate a team of twelve across three offices. I introduced a shared planning calendar that cut missed deadlines by a third, and I prepare the monthly reports our directors use to set priorities.",
        ),
        para(
            "What draws me to your company is its focus on community projects. I would bring clear communication, careful organisation and a habit of following through. I would be glad to talk about how I can help your team.",
        ),
        para("Thank you for considering my application."),
        para("Sincerely,"),
        para(""),
        para("Your Name"),
    ];
    finish(&mut d, "Cover Letter", lines.into_iter().map(Block::Para).collect());
    d
}

/// A project proposal: summary, goals, scope, a timeline table and a budget table.
pub fn proposal() -> Document {
    let mut d = Document::new();
    let mut b = vec![
        Block::Para(styled("Project Proposal", "Title")),
        Block::Para(styled("Library reading room renovation", "Subtitle")),
        Block::Para(field_line("Prepared for:", "Board of Trustees")),
        Block::Para(field_line("Prepared by:", "Your Name, Facilities")),
        Block::Para(field_line("Date:", &crate::cmd::insert::format_date("MMMM d, yyyy"))),
        Block::Para(styled("Executive summary", "Heading1")),
        Block::Para(para(
            "The reading room has not been updated in twenty years. This proposal asks for funding to improve lighting, seating and accessibility so the room can serve more visitors for longer hours.",
        )),
        Block::Para(styled("Goals", "Heading1")),
    ];
    b.extend(list(
        &mut d,
        ListKind::Numbered,
        &["Double the number of seats with power outlets.", "Make every table reachable by wheelchair.", "Cut lighting energy use by half."],
    ));
    b.push(Block::Para(styled("Scope", "Heading1")));
    b.push(Block::Para(para(
        "The work covers the reading room only: lighting, furniture, flooring and the entrance ramp. Book stacks and the heating system are out of scope.",
    )));
    b.push(Block::Para(styled("Timeline", "Heading1")));
    b.push(Block::Table(grid(
        &[
            &["Phase", "Work", "Duration"],
            &["1. Design", "Layout, lighting plan, supplier quotes", "4 weeks"],
            &["2. Build", "Flooring, ramp, electrical work", "6 weeks"],
            &["3. Fit-out", "Furniture and lighting installed", "2 weeks"],
            &["4. Opening", "Soft opening and visitor feedback", "1 week"],
        ],
        &[96.0, 270.0, 102.0],
        "GridTable4AccentBlue",
        &[],
    )));
    b.push(Block::Para(styled("Budget", "Heading1")));
    let mut budget = grid(
        &[
            &["Item", "Cost"],
            &["Lighting and electrical", "18,000"],
            &["Furniture", "24,500"],
            &["Flooring and ramp", "12,000"],
            &["Contingency (10%)", "5,450"],
            &["Total", "59,950"],
        ],
        &[348.0, 120.0],
        "PlainTable1",
        &[1],
    );
    bold_last_row(&mut budget, 0);
    b.push(Block::Table(budget));
    b.push(Block::Para(styled("Next steps", "Heading1")));
    b.push(Block::Para(para("With the board's approval, design work can start next month and the room can reopen within one season.")));
    finish(&mut d, "Project Proposal", b);
    d
}

/// A one-page event flyer: a large centred headline, the when and where, and highlights.
pub fn flyer() -> Document {
    let mut d = Document::new();
    let accent = Rgb(0xE9, 0x71, 0x32);
    let centred = |text: &str, props: CharProps, after: f32| {
        let mut p = Paragraph::new();
        add(&mut p, text, props);
        p.props.align = Some(Align::Center);
        p.props.space_after = Some(after);
        p
    };
    let mut b = vec![
        Block::Para(centred(
            "COMMUNITY",
            CharProps { bold: Some(true), size: Some(28.0), color: Some(TextColor::Rgb(accent)), ..Default::default() },
            0.0,
        )),
        Block::Para(centred("Summer Market", CharProps { bold: Some(true), size: Some(54.0), ..Default::default() }, 6.0)),
        Block::Para(centred(
            "Local food · Crafts · Live music · Games for kids",
            CharProps { italic: Some(true), size: Some(16.0), ..Default::default() },
            24.0,
        )),
    ];
    let mut when = centred("Saturday, June 14 · 10:00 – 16:00", CharProps { bold: Some(true), size: Some(20.0), ..Default::default() }, 4.0);
    when.props.shading = Some(Rgb(0xFB, 0xE2, 0xD5));
    let mut where_ = centred("Riverside Park, Main Pavilion", CharProps { size: Some(18.0), ..Default::default() }, 24.0);
    where_.props.shading = Some(Rgb(0xFB, 0xE2, 0xD5));
    b.push(Block::Para(when));
    b.push(Block::Para(where_));
    b.push(Block::Para(styled("What's on", "Heading1")));
    b.extend(list(
        &mut d,
        ListKind::Bullet,
        &[
            "More than forty stalls from local growers, bakers and makers",
            "Live acoustic sets every hour on the pavilion stage",
            "Face painting, a treasure hunt and a free bike check",
            "Food trucks and a shaded picnic area",
        ],
    ));
    let mut foot = centred("Free entry · Everyone welcome", CharProps { bold: Some(true), size: Some(16.0), ..Default::default() }, 4.0);
    foot.props.space_before = Some(24.0);
    b.push(Block::Para(foot));
    b.push(Block::Para(centred("More information: events@example.com", CharProps::default(), 0.0)));
    finish(&mut d, "Event Flyer", b);
    d
}

/// The built-in template ids `file.new` accepts, besides `blank`.
pub const TEMPLATE_IDS: [&str; 11] =
    ["sample", "letter", "resume", "report", "memo", "agenda", "minutes", "invoice", "cover-letter", "proposal", "flyer"];

/// The built-in template with this id, or `None` for an unknown id.
pub fn template(id: &str) -> Option<Document> {
    Some(match id {
        "sample" => sample_document(),
        "letter" => letter(),
        "resume" => resume(),
        "report" => report(),
        "memo" => memo(),
        "agenda" => agenda(),
        "minutes" => minutes(),
        "invoice" => invoice(),
        "cover-letter" => cover_letter(),
        "proposal" => proposal(),
        "flyer" => flyer(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_id_builds_a_document() {
        for id in TEMPLATE_IDS {
            assert!(template(id).is_some(), "{id}");
        }
        assert!(template("blank").is_none() && template("nope").is_none());
    }

    #[test]
    fn templates_round_trip_through_docx() {
        for id in TEMPLATE_IDS {
            let Some(d) = template(id) else { panic!("{id}") };
            let bytes = crate::io::save_bytes("x.docx", &d).unwrap();
            assert!(!bytes.is_empty(), "{id}");
        }
    }

    #[test]
    fn samples_are_valid() {
        assert_eq!(sample_document().word_count(), 361);
        for d in [sample_document(), letter(), resume(), report(), memo(), agenda(), minutes(), invoice(), cover_letter(), proposal(), flyer()] {
            assert!(d.word_count() > 5);
            let l = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
            assert!(!l.pages.is_empty());
        }
    }
}
