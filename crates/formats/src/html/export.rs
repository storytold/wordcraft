use super::{Align, Document, FBlock, FTable, Flow, Fmt, Inline, Kind, Para, VAlign, base64_encode, mime_of, model};

// ---------------------------------------------------------------------------------------------
// Export

/// Escape text for HTML content and attribute values.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\u{000C}' | '\u{000E}' => {}
            _ => o.push(c),
        }
    }
    o
}

fn align_css(a: Option<Align>) -> Option<&'static str> {
    match a? {
        Align::Center => Some("text-align:center"),
        Align::Right => Some("text-align:right"),
        Align::Justify | Align::Distribute => Some("text-align:justify"),
        Align::Left => None,
    }
}

fn text_html(t: &str, at_start: bool) -> String {
    let mut o = String::new();
    let mut prev_space = at_start;
    for c in t.chars() {
        match c {
            '\n' => {
                o.push_str("<br>");
                prev_space = true;
                continue;
            }
            '\t' => o.push_str("<span style=\"white-space:pre\">\t</span>"),
            ' ' if prev_space => o.push_str("&nbsp;"),
            _ => o.push_str(&esc(&c.to_string())),
        }
        prev_space = c == ' ';
    }
    o
}

/// The tags that carry `f`'s character formatting: what opens them, and what closes them
/// (innermost last, so the closes are written in reverse).
fn fmt_tags(f: &Fmt) -> (String, Vec<&'static str>) {
    let mut open = String::new();
    let mut close: Vec<&str> = Vec::new();
    // A script or other non-web target is written as plain text (`is_safe_href`).
    if let Some(l) = f.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l)) {
        open.push_str(&format!("<a href=\"{}\">", esc(l)));
        close.push("</a>");
    }
    let mut tag = |o: &str, c: &'static str| {
        open.push_str(o);
        close.push(c);
    };
    if f.bold {
        tag("<strong>", "</strong>");
    }
    if f.italic {
        tag("<em>", "</em>");
    }
    if f.underline {
        tag("<u>", "</u>");
    }
    if f.strike {
        tag("<s>", "</s>");
    }
    if f.sup {
        tag("<sup>", "</sup>");
    } else if f.sub {
        tag("<sub>", "</sub>");
    }
    if f.code {
        tag("<code>", "</code>");
    }
    let mut css = Vec::new();
    if let Some(c) = f.color {
        css.push(format!("color:#{}", c.hex()));
    }
    if let Some(c) = f.background {
        css.push(format!("background-color:#{}", c.hex()));
    }
    if let Some(s) = f.size {
        css.push(format!("font-size:{}pt", trim_num(s)));
    }
    if let Some(fam) = &f.font {
        css.push(format!("font-family:'{}'", esc(&fam.replace('\'', ""))));
    }
    if !css.is_empty() {
        open.push_str(&format!("<span style=\"{}\">", css.join(";")));
        close.push("</span>");
    }
    (open, close)
}

/// Inline spans → HTML.
pub fn inlines_html(inlines: &[Inline]) -> String {
    let mut o = String::new();
    for i in inlines.iter() {
        match i {
            Inline::Text(t, f) => {
                let (open, close) = fmt_tags(f);
                let at_start = o.is_empty();
                o.push_str(&open);
                o.push_str(&text_html(t, at_start));
                for c in close.iter().rev() {
                    o.push_str(c);
                }
            }
            // A phonetic guide: the base text with its reading in `<rt>` (#1681). HTML ruby has
            // no alignment or size, so the guide's own attributes stay in the document model.
            Inline::Ruby { base, ruby, fmt } => {
                let (open, close) = fmt_tags(fmt);
                let at_start = o.is_empty();
                o.push_str(&open);
                o.push_str(&format!("<ruby>{}<rt>{}</rt></ruby>", text_html(base, at_start), text_html(ruby, false)));
                for c in close.iter().rev() {
                    o.push_str(c);
                }
            }
            Inline::Image(img) => {
                o.push_str(&format!(
                    "<img src=\"data:{};base64,{}\" alt=\"{}\" width=\"{}\" height=\"{}\">",
                    mime_of(&img.ext),
                    base64_encode(&img.data),
                    esc(&img.alt),
                    (img.w / 0.75).round(),
                    (img.h / 0.75).round()
                ));
            }
            Inline::Anchor(a) => o.push_str(&format!("<a id=\"{}\"></a>", esc(a))),
            // No picture to show: the browser shows the alt text in its place.
            Inline::Figure(alt) => o.push_str(&format!("<img alt=\"{}\">", esc(alt))),
            // MathML, which browsers, Word and LibreOffice show as an equation (#816).
            Inline::Equation { linear, display } => o.push_str(&crate::mathml::linear_to_mathml(linear, *display)),
        }
    }
    o
}

fn trim_num(v: f32) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn para_attrs(p: &Para) -> String {
    let mut css: Vec<&str> = Vec::new();
    if let Some(a) = align_css(p.align) {
        css.push(a);
    }
    if p.page_break {
        css.push("break-before:page;page-break-before:always");
    }
    if css.is_empty() { String::new() } else { format!(" style=\"{}\"", css.join(";")) }
}

fn para_html(p: &Para) -> String {
    let body = inlines_html(&p.inlines);
    let attrs = para_attrs(p);
    match p.kind {
        Kind::Heading(n) => format!("<h{n}{attrs}>{body}</h{n}>", n = n.clamp(1, 6)),
        Kind::Title => format!("<h1{attrs}>{body}</h1>"),
        Kind::Rule => "<hr>".to_string(),
        _ if body.is_empty() => format!("<p{attrs}>&nbsp;</p>"),
        _ => format!("<p{attrs}>{body}</p>"),
    }
}

fn table_html(t: &FTable, out: &mut String, depth: usize) {
    out.push_str("<table style=\"border-collapse:collapse\">\n");
    for row in &t.rows {
        out.push_str("<tr>");
        for c in row {
            if c.covered {
                continue;
            }
            let tag = if c.header { "th" } else { "td" };
            let mut attrs = String::new();
            if c.colspan > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", c.colspan));
            }
            if c.rowspan > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", c.rowspan));
            }
            let v_css = match c.valign {
                Some(VAlign::Center) => "vertical-align:middle",
                Some(VAlign::Bottom) => "vertical-align:bottom",
                _ => "vertical-align:top",
            };
            let mut css = vec!["padding:4px 6px".to_string(), v_css.to_string()];
            if !t.borderless {
                css.insert(0, "border:1px solid #999".to_string());
            }
            if let Some(s) = c.shading {
                css.push(format!("background-color:#{}", s.hex()));
            }
            let single = match c.blocks.as_slice() {
                [FBlock::Para(p)] if p.kind == Kind::Normal && p.list.is_none() && !p.page_break => Some(p),
                _ => None,
            };
            if let Some(a) = single.and_then(|p| align_css(p.align)) {
                css.push(a.to_string());
            }
            out.push_str(&format!("<{tag}{attrs} style=\"{}\">", css.join(";")));
            match single {
                Some(p) => out.push_str(&inlines_html(&p.inlines)),
                None => {
                    out.push('\n');
                    blocks_html(&c.blocks, out, depth + 1);
                }
            }
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

fn blocks_html(blocks: &[FBlock], out: &mut String, depth: usize) {
    let mut i = 0;
    while let Some(b) = blocks.get(i) {
        match b {
            FBlock::Table(t) => {
                if depth < model::MAX_DEPTH {
                    table_html(t, out, depth);
                }
                i += 1;
            }
            FBlock::Para(p) if p.list.is_some() => {
                // Nested lists from consecutive list paragraphs.
                let mut stack: Vec<(bool, bool)> = Vec::new(); // (ordered, li open)
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    let Some(li) = q.list else { break };
                    let lv = li.level.min(8) as usize;
                    while stack.len() > lv + 1 {
                        close_list(&mut stack, out);
                    }
                    if stack.len() == lv + 1 && stack.last().is_some_and(|s| s.0 != li.ordered) {
                        close_list(&mut stack, out);
                    }
                    while stack.len() < lv + 1 {
                        if let Some(top) = stack.last_mut()
                            && !top.1
                        {
                            out.push_str("<li>");
                            top.1 = true;
                        }
                        out.push_str(if li.ordered { "<ol>" } else { "<ul>" });
                        stack.push((li.ordered, false));
                    }
                    if let Some(top) = stack.last_mut() {
                        if top.1 {
                            out.push_str("</li>\n");
                        }
                        top.1 = true;
                    }
                    let body = inlines_html(&q.inlines);
                    let attrs = para_attrs(q);
                    out.push_str(&format!("<li{attrs}>{body}"));
                    i += 1;
                }
                while !stack.is_empty() {
                    close_list(&mut stack, out);
                }
                out.push('\n');
            }
            FBlock::Para(p) if p.kind == Kind::Code => {
                out.push_str("<pre><code>");
                let mut first = true;
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Code || q.list.is_some() {
                        break;
                    }
                    if !first {
                        out.push('\n');
                    }
                    first = false;
                    out.push_str(&esc(&q.text()));
                    i += 1;
                }
                out.push_str("</code></pre>\n");
            }
            FBlock::Para(p) if p.kind == Kind::Quote => {
                out.push_str("<blockquote>\n");
                while let Some(FBlock::Para(q)) = blocks.get(i) {
                    if q.kind != Kind::Quote || q.list.is_some() {
                        break;
                    }
                    out.push_str(&para_html(&Para { kind: Kind::Normal, ..q.clone() }));
                    out.push('\n');
                    i += 1;
                }
                out.push_str("</blockquote>\n");
            }
            FBlock::Para(p) => {
                out.push_str(&para_html(p));
                out.push('\n');
                i += 1;
            }
        }
    }
}

fn close_list(stack: &mut Vec<(bool, bool)>, out: &mut String) {
    if let Some((ordered, li_open)) = stack.pop() {
        if li_open {
            out.push_str("</li>");
        }
        out.push_str(if ordered { "</ol>" } else { "</ul>" });
    }
}

/// A complete HTML document.
pub fn export(doc: &Document) -> String {
    let flow = model::from_doc(doc);
    export_flow(&flow, doc.styles.default_chr.lang.as_deref().unwrap_or("en"))
}

/// `doc`'s content as an HTML fragment (no head or style sheet): what a copy puts on the system
/// clipboard beside its text, equations as MathML (#816).
pub fn export_fragment(doc: &Document) -> String {
    let flow = model::from_doc(doc);
    let mut out = String::from("<meta charset=\"utf-8\">");
    blocks_html(&flow.blocks, &mut out, 0);
    out
}

pub fn export_flow(flow: &Flow, lang: &str) -> String {
    let m = &flow.meta;
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n");
    out.push_str(&format!("<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n", esc(lang)));
    out.push_str("<meta name=\"generator\" content=\"WordCraft\">\n");
    out.push_str(&format!("<title>{}</title>\n", esc(&m.title)));
    for (k, v) in [("author", &m.author), ("description", &m.description), ("keywords", &m.keywords), ("subject", &m.subject)] {
        if !v.is_empty() {
            out.push_str(&format!("<meta name=\"{k}\" content=\"{}\">\n", esc(v)));
        }
    }
    out.push_str(
        "<style>\nbody{font-family:Aptos,Calibri,'Segoe UI',Arial,sans-serif;font-size:12pt;line-height:1.15;max-width:48em;margin:2em auto;padding:0 1em}\n\
         table{border-collapse:collapse;margin:0.5em 0}\nblockquote{margin:0.5em 2em;font-style:italic;color:#404040}\n\
         pre{background:#f5f5f5;padding:0.5em;font-family:'Courier New',monospace;font-size:10pt}\n</style>\n</head>\n<body>\n",
    );
    blocks_html(&flow.blocks, &mut out, 0);
    out.push_str("</body>\n</html>\n");
    out
}
