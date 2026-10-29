use super::*;
use crate::model::FBlock;

fn paras(src: &str) -> Vec<Para> {
    parse(src).blocks.into_iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
}

fn texts(src: &str) -> Vec<String> {
    paras(src).iter().map(|p| p.text()).collect()
}

#[test]
fn empty_accent_group_is_plain_character() {
    let t = texts("\\begin{document}x\\^{}y \\~{}z 2\\^{}10 \\texttt{\\~{}/dir}\\end{document}");
    assert_eq!(t, vec!["x^y ~z 2^10 ~/dir".to_string()]);
    // The acute and the diaeresis alone are accents, not an apostrophe or a quotation mark.
    assert_eq!(texts("\\begin{document}\\'{} \\\"{}\\end{document}"), vec!["\u{B4} \u{A8}".to_string()]);
}

#[test]
fn unclosed_begin_keeps_its_brace() {
    // `env_name` used to consume the `{` before giving up on a missing `}`.
    let toks = tokenize("\\begin{ x");
    assert_eq!(toks.first(), Some(&Tok::Cmd("begin".into())), "{toks:?}");
    assert!(toks.contains(&Tok::Open), "{toks:?}");
    let toks = tokenize(&format!("\\end{{{}}}", "n".repeat(100)));
    assert!(toks.contains(&Tok::Open), "{toks:?}");
}

#[test]
fn spec_widths_repeats_are_bounded() {
    assert_eq!(spec_widths("*{3}{p{1cm}}").0, 3);
    assert_eq!(spec_widths("*{2}{*{2}{l}}").0, 4);
    let deep = format!("{}l{}", "*{1}{".repeat(MAX_NEST + 5), "}".repeat(MAX_NEST + 5));
    let (cols, widths) = spec_widths(&deep);
    assert!(cols <= wordcraft_doc::table::MAX_COLS && widths.is_empty());
    let wide = format!("{}p{{1cm}}{}", "*{64}{".repeat(30), "}".repeat(30));
    let (cols, widths) = spec_widths(&wide);
    assert_eq!(cols, wordcraft_doc::table::MAX_COLS);
    assert!(widths.len() <= wordcraft_doc::table::MAX_COLS);
}

#[test]
fn latex_length_pt_case_and_plus() {
    assert_eq!(length_pt("12pt"), Some(12.0));
    assert_eq!(length_pt("12PT"), Some(12.0));
    assert_eq!(length_pt("+12pt"), Some(12.0));
    assert_eq!(length_pt("1IN"), Some(72.27));
    assert_eq!(length_pt("+2cm"), Some(56.9));
}

const ARTICLE: &str = r#"\documentclass[11pt]{article}
\usepackage[utf8]{inputenc}
\usepackage{amsmath} % maths
\newcommand{\R}{\mathbb{R}}
\title{On \emph{Sample} Documents}
\author{Ada Lovelace \and Charles Babbage}
\date{\today}
\begin{document}
\maketitle

\section{Introduction}\label{sec:intro}
This is \textbf{bold}, \textit{italic}, \underline{underlined} and \texttt{code}.
TeX quotes ``like this'' and dashes -- or --- with 50\% and \$5 \& more~text.

A second paragraph with a footnote\footnote{The note.} and a
\href{https://example.com}{link}. % a comment
See Section~\ref{sec:intro}.

\subsection*{Lists}
\begin{itemize}
  \item First
  \item Second
  \begin{enumerate}
    \item Nested one
    \item[b)] Nested two
  \end{enumerate}
\end{itemize}

\begin{quote}
Quoted text.
\end{quote}

\begin{center}
Centered.
\end{center}

\begin{verbatim}
fn main() {
    println!("{}", 1 % 2);
}
\end{verbatim}

\begin{table}[h]
\centering
\begin{tabular}{|p{2cm}|l|}
\hline
\textbf{Name} & Value \\
\hline
\multicolumn{2}{|c|}{Wide} \\
alpha & $x^2$ \\
\hline
\end{tabular}
\caption{A table.}
\end{table}

Energy: $E = mc^2$ and
\[ \frac{a+b}{2} \leq \sqrt{\alpha} \]
\begin{equation}
  \sum_{i=1}^{n} i = \frac{n(n+1)}{2}
\end{equation}

\newpage
Caf\'e na\"ive \c{c}a \v{s}\'{e} and {\bfseries grouped bold} after.
{\color{red} red text} \textcolor[HTML]{00FF00}{green}.
\end{document}
"#;

#[test]
fn alignment_declarations_end_with_their_group() {
    // #440: `{\centering …\par}` centres only inside the group; the outer alignment returns.
    let align = |body: &str| -> Vec<(String, Option<Align>)> {
        paras(&format!("\\documentclass{{article}}\\begin{{document}}{body}\\end{{document}}")).iter().map(|p| (p.text(), p.align)).collect()
    };
    let a = |t: &str, al: Option<Align>| (t.to_string(), al);
    assert_eq!(align(r"{\centering Center\par}After\par"), vec![a("Center", Some(Align::Center)), a("After", None)]);
    assert_eq!(
        align(r"\raggedleft Outer\par{\centering Inner\par}Restored\par"),
        vec![a("Outer", Some(Align::Right)), a("Inner", Some(Align::Center)), a("Restored", Some(Align::Right))]
    );
    assert_eq!(align(r"\centering One\par Two\par"), vec![a("One", Some(Align::Center)), a("Two", Some(Align::Center))]);
}

#[test]
fn article_structure() {
    let flow = parse(ARTICLE);
    assert_article_metadata(&flow);
    let ps = paras(ARTICLE);
    assert_article_blocks(&ps);
    assert_article_formatting(&ps);
    assert_article_preamble_is_not_text();
}

fn assert_article_metadata(flow: &Flow) {
    assert_eq!(flow.meta.title, "On Sample Documents");
    assert_eq!(flow.meta.author, "Ada Lovelace Charles Babbage");
}

fn assert_article_blocks(ps: &[Para]) {
    let find = |t: &str| ps.iter().find(|p| p.text().trim() == t).unwrap_or_else(|| panic!("no {t:?} in {:?}", texts(ARTICLE)));
    assert_eq!(find("On Sample Documents").kind, Kind::Title);
    assert_eq!(find("Introduction").kind, Kind::Heading(1));
    assert_eq!(find("Lists").kind, Kind::Heading(2));
    assert_eq!(find("First").list, Some(ListInfo { ordered: false, level: 0, start: 1 }));
    assert_eq!(find("Nested one").list, Some(ListInfo { ordered: true, level: 1, start: 1 }));
    assert!(ps.iter().any(|p| p.text().trim() == "b) Nested two" && p.list.is_some()), "{:?}", texts(ARTICLE));
    assert_eq!(find("Quoted text.").kind, Kind::Quote);
    assert_eq!(find("Centered.").align, Some(Align::Center));
    assert_eq!(find("fn main() {").kind, Kind::Code);
    assert_eq!(find("println!(\"{}\", 1 % 2);").kind, Kind::Code, "verbatim keeps % and indentation");
    assert!(ps.iter().any(|p| p.text() == "    println!(\"{}\", 1 % 2);"));
    assert_eq!(find("A table.").align, Some(Align::Center));
    let second = ps.iter().find(|p| p.text().starts_with("A second")).unwrap();
    assert!(second.text().contains("footnote (The note.) and a link."), "{}", second.text());
    assert!(second.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "link" && f.link.as_deref() == Some("https://example.com"))));
    assert!(!second.text().contains("comment"));
}

fn assert_article_formatting(ps: &[Para]) {
    let body = ps.iter().find(|p| p.text().starts_with("This is")).unwrap();
    let has = |t: &str, f: &dyn Fn(&Fmt) -> bool| body.inlines.iter().any(|i| matches!(i, Inline::Text(x, fm) if x == t && f(fm)));
    assert!(has("bold", &|f| f.bold));
    assert!(has("italic", &|f| f.italic));
    assert!(has("underlined", &|f| f.underline));
    assert!(has("code", &|f| f.code));
    assert!(body.text().contains("“like this” and dashes – or — with 50% and $5 & more\u{A0}text."), "{}", body.text());
    let accents = ps.iter().find(|p| p.text().starts_with("Café")).unwrap();
    assert!(accents.page_break, "\\newpage");
    assert!(accents.text().starts_with("Café naïve ça šé and grouped bold after."), "{}", accents.text());
    assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "grouped bold" && f.bold)));
    assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t.starts_with(" after.") && !f.bold)), "{:?}", accents.inlines);
    assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t.contains("red text") && f.color == Some(Rgb(255, 0, 0)))));
    assert!(accents.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "green" && f.color == Some(Rgb(0, 255, 0)))));
}

fn assert_article_preamble_is_not_text() {
    let all = texts(ARTICLE).join("\n");
    for gone in ["mathbb", "utf8", "11pt", "today", "sec:intro}", "amsmath", "maths"] {
        assert!(!all.contains(gone), "{gone:?} leaked: {all}");
    }
}

#[test]
fn tables_and_math() {
    let flow = parse(ARTICLE);
    let t = flow.blocks.iter().find_map(|b| if let FBlock::Table(t) = b { Some(t) } else { None }).expect("table");
    assert_eq!(t.rows.len(), 3, "{t:?}");
    assert_eq!(t.rows[0].iter().map(Cell::text).collect::<Vec<_>>(), ["Name", "Value"]);
    assert_eq!(t.rows[1].len(), 1);
    assert_eq!(t.rows[1][0].colspan, 2);
    assert_eq!(t.rows[1][0].text(), "Wide");
    assert!(t.widths.is_empty(), "the l column has no width");
    let eq =
        |p: &Para| p.inlines.iter().find_map(|i| if let Inline::Equation { linear, display } = i { Some((linear.clone(), *display)) } else { None });
    let ps = paras(ARTICLE);
    let energy = ps.iter().find(|p| p.text().starts_with("Energy")).unwrap();
    assert_eq!(eq(energy), Some(("E = mc^2".to_string(), false)));
    let displays: Vec<_> = ps.iter().filter_map(eq).filter(|(_, d)| *d).map(|(l, _)| l).collect();
    assert_eq!(displays, ["(a+b)/2 ≤ √(α)", "∑_(i=1)^n i = (n(n+1))/2"]);
    let cell = &t.rows[2][1];
    let FBlock::Para(p) = &cell.blocks[0] else { panic!() };
    assert_eq!(eq(p), Some(("x^2".to_string(), false)));
}

#[test]
fn multirow_cells_and_widths() {
    let src = r"\begin{tabular}{p{1in}p{2cm}}
\multirow{2}{*}{Tall} & a \\
 & b \\
\end{tabular}";
    let flow = parse(src);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("{:?}", flow.blocks) };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[0][0].rowspan, 2);
    assert!(t.rows[1][0].covered);
    assert_eq!(t.rows[1][1].text(), "b");
    assert_eq!(t.widths.len(), 2);
    assert!((t.widths[0] - (72.27 + 12.0)).abs() < 0.1, "{:?}", t.widths);
}

#[test]
fn fragments_without_a_document_environment() {
    assert_eq!(texts("Just \\textbf{text}.\n\nTwo."), ["Just text.", "Two."]);
    assert_eq!(texts(""), Vec::<String>::new());
    assert_eq!(texts("\\unknown{kept} \\unknownopt[dropped]{kept too}"), ["kept kept too"]);
    assert_eq!(texts("a\\\\b\\newline c"), ["a\nb\nc"]);
    assert_eq!(texts("\\verb|\\x{}| y"), ["\\x{} y"]);
    assert_eq!(texts("\\chapter{C}\\section{S}").len(), 2);
    let ps = paras("\\chapter{C}\\section{S}");
    assert_eq!((ps[0].kind, ps[1].kind), (Kind::Heading(1), Kind::Heading(2)));
}

#[test]
fn math_conversions() {
    for (tex, linear) in [
        (r"\frac{1}{2}", "1/2"),
        (r"x_{i}^{2}", "x_i^2"),
        (r"x^{10}", "x^10"),
        (r"e^{i\pi}", "e^(iπ)"),
        (r"\sqrt[3]{x}", "√(3&x)"),
        (r"\left( a \right)", "( a )"),
        (r"\sin x + \text{if } y", "sin x + if y"),
        (r"a \cdot b \times c \neq d", "a ⋅ b × c ≠ d"),
        (r"a \bullet b", "a • b"),
        (r"x \top y \bot z", "x ⊤ y ⊥ z"),
        (r"f \colon X \to Y", "f : X → Y"),
        (r"\hbar \hslash", "ℏ ℏ"),
        (r"\bm{v}", "v"),
        (r"\acute{x} \grave{y} \breve{z} \check{w}", "x́ ỳ z̆ w̌"),
        (r"a \equiv b \pmod{m}", "a ≡ b (mod m)"),
        (r"a \bmod b", "a mod b"),
        (r"\unknowncmd", r"\unknowncmd"),
    ] {
        assert_eq!(latex_to_linear(tex), linear, "{tex}");
    }
    for (linear, tex) in [
        ("x=(-b±√(b^2-4ac))/(2a)", r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}"),
        ("α ≤ β", r"\alpha \leq \beta"),
        ("αβ", r"\alpha\beta"),
        ("πr", r"\pi r"),
        ("a • b", r"a \bullet b"),
        ("x ⊤ y", r"x \top y"),
        ("a_(i+1)", r"a_{i+1}"),
        ("50% & #", r"50\% \& \#"),
        ("√(3&x)", r"\sqrt[3]{x}"),
    ] {
        assert_eq!(linear_to_latex(linear), tex, "{linear}");
    }
}

#[test]
fn equations_survive_a_round_trip() {
    let mut d = Document::new();
    let mut p = wordcraft_doc::Paragraph::with_text("Area ", Default::default());
    let n = p.len();
    p.insert_object(
        n,
        wordcraft_doc::para::InlineObject::Equation { linear: "π r^2".into(), display: false, math: Default::default() },
        &Default::default(),
    )
    .unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let tex = export(&d);
    assert!(tex.contains("$\\pi r^2$") && tex.contains("\\usepackage{amsmath}"), "{tex}");
    let back = import(tex.as_bytes()).unwrap();
    let wordcraft_doc::Block::Para(bp) = &*back.body[0] else { panic!() };
    assert!(
        bp.objects.iter().any(|o| matches!(o, wordcraft_doc::para::InlineObject::Equation { linear, .. } if linear == "π r^2")),
        "{:?}",
        bp.objects
    );
    // Formats without equations keep the text.
    let md = crate::markdown::export(&d);
    assert!(md.contains("π r^2"), "{md}");
}

#[test]
fn leading_brackets_stay_text() {
    // `\item [x]` and `\\ [1]` would read the bracket as an optional argument.
    assert_eq!(escape_text("[x] a\n [1] b [c]"), "{[}x] a\\\\\n {[}1] b [c]");
    let doc = crate::markdown::import("- [x] done");
    let back = import(export(&doc).as_bytes()).unwrap();
    assert_eq!(back.plain_text(Default::default()), "[x] done");
}

#[test]
fn special_characters_round_trip() {
    let text = "50% of $5 & #1_a {b} ~c^ \\d <e> |f| -- ``g'' tab\there";
    let d = Document::from_text(text);
    let tex = export(&d);
    let back = import(tex.as_bytes()).unwrap();
    assert_eq!(back.plain_text(Default::default()).trim(), text.replace('\t', "\u{2003}"), "{tex}");
}

#[test]
fn exported_tables_size_their_columns() {
    let mut t = FTable { rows: Vec::new(), widths: vec![100.0, 200.0], borderless: false };
    let cell = |s: &str| Cell {
        blocks: vec![FBlock::Para({
            let mut p = Para::default();
            p.push_text(s, &Fmt::default());
            p
        })],
        ..Default::default()
    };
    t.rows.push(vec![cell("a"), cell("b")]);
    t.rows.push(vec![Cell { colspan: 2, ..cell("wide") }]);
    let mut w = Writer::default();
    w.table(&t);
    assert!(w.out.contains("\\begin{tabular}{|p{88pt}|p{188pt}|}"), "{}", w.out);
    assert!(w.out.contains("\\multicolumn{2}{|p{288pt}|}{wide}"), "{}", w.out);
    let back = parse(&w.out);
    let FBlock::Table(bt) = &back.blocks[0] else { panic!() };
    assert_eq!(bt.widths, vec![100.0, 200.0]);
    assert_eq!(bt.rows[1][0].colspan, 2);
}

#[test]
fn urls_preserve_tildes() {
    let tex = r"\href{https://example.com/~user/page}{Homepage} and \url{https://example.com/~user/page}";
    let flow = parse(tex);
    let FBlock::Para(p) = &flow.blocks[0] else { panic!() };
    assert_eq!(p.inlines[0], Inline::Text("Homepage".to_string(), Fmt { link: Some("https://example.com/~user/page".into()), ..Default::default() }));
    assert_eq!(
        p.inlines[2],
        Inline::Text("https://example.com/~user/page".to_string(), Fmt { link: Some("https://example.com/~user/page".into()), ..Default::default() })
    );
}

#[test]
fn exported_borderless_tables_omit_rules() {
    let mut t = FTable { rows: Vec::new(), widths: vec![100.0, 200.0], borderless: true };
    let cell = |s: &str| Cell {
        blocks: vec![FBlock::Para({
            let mut p = Para::default();
            p.push_text(s, &Fmt::default());
            p
        })],
        ..Default::default()
    };
    t.rows.push(vec![cell("a"), cell("b")]);
    t.rows.push(vec![Cell { colspan: 2, ..cell("wide") }]);
    let mut w = Writer::default();
    w.table(&t);
    assert!(w.out.contains("\\begin{tabular}{p{88pt} p{188pt}}"), "{}", w.out);
    assert!(!w.out.contains("\\hline"), "{}", w.out);
    assert!(w.out.contains("\\multicolumn{2}{p{288pt}}{wide}"), "{}", w.out);
    let back = parse(&w.out);
    let FBlock::Table(bt) = &back.blocks[0] else { panic!() };
    assert!(bt.borderless, "parsed table should be borderless");
    assert_eq!(bt.widths, vec![100.0, 200.0]);
    assert_eq!(bt.rows[1][0].colspan, 2);
}

#[test]
fn table_borders_and_borderless_round_trip() {
    use std::sync::Arc;
    use wordcraft_doc::{Block, CharProps, Paragraph, Table, para_block};

    // 1. Borderless table.
    let mut doc = Document::new();
    let mut t_borderless = Table::new(2, 2, 400.0);
    t_borderless.props.style = None;
    for r in 0..2 {
        for c in 0..2 {
            t_borderless.rows[r].cells[c].blocks = vec![para_block(Paragraph::with_text("cell", CharProps::default()))];
        }
    }
    doc.body = vec![Arc::new(Block::Table(t_borderless))];
    let tex = export(&doc);
    assert!(!tex.contains("\\hline"));
    let back = import(tex.as_bytes()).unwrap();
    let Some(Block::Table(bt)) = back.body.first().map(|b| &**b) else { panic!() };
    assert!(bt.props.style.is_none(), "imported table should remain borderless");

    // 2. Bordered table.
    let mut doc_bordered = Document::new();
    let mut t_bordered = Table::new(2, 2, 400.0);
    t_bordered.props.style = Some("TableGrid".to_string());
    for r in 0..2 {
        for c in 0..2 {
            t_bordered.rows[r].cells[c].blocks = vec![para_block(Paragraph::with_text("cell", CharProps::default()))];
        }
    }
    doc_bordered.body = vec![Arc::new(Block::Table(t_bordered))];
    let tex_bordered = export(&doc_bordered);
    assert!(tex_bordered.contains("\\hline"));
    let back_bordered = import(tex_bordered.as_bytes()).unwrap();
    let Some(Block::Table(bt_bordered)) = back_bordered.body.first().map(|b| &**b) else { panic!() };
    assert!(bt_bordered.props.style.is_some(), "imported table should remain bordered");
}

#[test]
fn verbatim_end_is_kept_out_after_control_characters_go() {
    // A control character inside `\end{verbatim}` was removed after the check for it, so
    // the code line closed the environment and the rest ran as TeX.
    let mut d = Document::new();
    let line = "\\end{verb\u{E}atim}\\input{/etc/passwd}";
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(line, Default::default()).styled(model::CODE_STYLE))];
    let out = export(&d);
    assert_eq!(out.matches("\\end{verbatim}").count(), 1, "{out}");
    assert!(out.contains("\\end {verbatim}\\input"), "{out}");
}

#[test]
fn nested_multicolumn_wrappers_are_bounded() {
    // Each `\multicolumn` unwrapped rescanned the rest of the cell; nested ones were all
    // unwrapped, quadratic in the nesting.
    let n = 50_000;
    let src = format!("\\begin{{tabular}}{{l}}{}x{}\\\\\\end{{tabular}}", "\\multicolumn{1}{c}{".repeat(n), "}".repeat(n));
    let t = std::time::Instant::now();
    let f = parse(&src);
    // Under a second on a loaded machine; unwrapping every wrapper took 213 s.
    assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
    assert!(f.blocks.iter().any(|b| matches!(b, FBlock::Table(_))));
    // One or two wrappers, as written in real tables, are still read.
    let f = parse("\\begin{tabular}{ll}\\multicolumn{2}{c}{\\multirow{2}{*}{Both}}\\\\\\end{tabular}");
    let Some(FBlock::Table(t)) = f.blocks.first() else { panic!("{:?}", f.blocks) };
    assert_eq!((t.rows[0][0].colspan, t.rows[0][0].rowspan), (2, 2));
    assert_eq!(t.rows[0][0].text(), "Both");
}

/// #1341: a source at the import limit opens whole; one byte more is an error naming the
/// limit, never a silently cut prefix.
#[test]
fn source_over_the_limit_is_an_error_not_a_prefix() {
    let at = format!("{}ENDMARK", "a".repeat(57));
    assert_eq!(at.len(), 64);
    let d = import_limited(at.as_bytes(), 64).unwrap();
    assert!(d.plain_text(Default::default()).ends_with("ENDMARK"), "{:?}", d.plain_text(Default::default()));
    let over = format!("{at}Z");
    let err = import_limited(over.as_bytes(), 64).unwrap_err();
    assert!(err.contains("64 bytes"), "{err}");
    // The real limit, named in MiB.
    let big = vec![b'a'; MAX_SOURCE + 1];
    let err = import(&big).unwrap_err();
    assert!(err.contains("32 MiB"), "{err}");
    assert!(crate::import("tex", &big).is_some_and(|r| r.is_err()));
}
