use super::*;

/// A 40 × 20 px SVG: red left half, blue right half.
fn halves() -> Vec<u8> {
    br##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 40 20">
  <rect x="0" y="0" width="20" height="20" fill="#ff0000"/>
  <rect x="20" y="0" width="20" height="20" fill="#0000ff"/>
</svg>"##
        .to_vec()
}

fn px(r: &Raster, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * r.width + x) * 4) as usize;
    let p = r.rgba.get(i..i + 4).unwrap();
    [p[0], p[1], p[2], p[3]]
}

#[test]
fn recognises_svg_files() {
    assert!(is_svg(&halves()));
    assert!(is_svg(b"  <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"/>"));
    assert!(is_svg(b"\xEF\xBB\xBF<!-- c --><!DOCTYPE svg><svg width=\"1\" height=\"1\"/>"));
    assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
    assert!(!is_svg(b"<?xml version=\"1.0\"?><w:document/>"));
    assert!(!is_svg(b""));
    // Compressed SVG is refused (a gzip bomb could expand without bound).
    assert!(!is_svg(&[0x1f, 0x8b, 8, 0, 0, 0]));
}

#[test]
fn natural_size_is_css_pixels_at_96_ppi() {
    assert_eq!(natural_size(&halves()), Some((30.0, 15.0)));
    // A viewBox alone gives the size.
    let vb = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48"/>"#;
    assert_eq!(natural_size(vb), Some((72.0, 36.0)));
    // Huge pictures are scaled down to 22 inches on the longer side.
    let big = br#"<svg xmlns="http://www.w3.org/2000/svg" width="96000" height="48000"/>"#;
    assert_eq!(natural_size(big), Some((1584.0, 792.0)));
    assert_eq!(natural_size(b"not svg"), None);
}

#[test]
fn renders_at_the_requested_resolution() {
    let svg = parse(&halves()).unwrap();
    assert_eq!(svg.size_px(), (40.0, 20.0));
    // Ten times the natural size: the edge between the halves is still one pixel sharp.
    let r = svg.render(400, 200, [10.0, 0.0, 0.0, 10.0, 0.0, 0.0]).unwrap();
    assert_eq!((r.width, r.height), (400, 200));
    assert_eq!(px(&r, 199, 100), [255, 0, 0, 255]);
    assert_eq!(px(&r, 200, 100), [0, 0, 255, 255]);
}

#[test]
fn fallback_png_is_twice_the_natural_size() {
    let png = fallback_png(&halves()).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    // IHDR width and height.
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 80);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 40);
    let big = br#"<svg xmlns="http://www.w3.org/2000/svg" width="96000" height="48000"><rect width="10" height="10"/></svg>"#;
    let png = fallback_png(big).unwrap();
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), MAX_FALLBACK_SIDE);
}

#[test]
fn hostile_svgs_are_errors_not_panics() {
    for bad in [
        b"".as_slice(),
        b"<svg",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"0\" height=\"0\"/>",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"-5\" height=\"NaN\"><rect/></svg>",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1e39\" height=\"1e39\"/>",
        b"\xff\xfe<\x00s\x00",
    ] {
        assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn deep_nesting_is_refused() {
    // Refused before usvg recurses into it.
    let deep = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}{}</svg>"#, "<g>".repeat(5000), "</g>".repeat(5000));
    assert_eq!(parse(deep.as_bytes()).err(), Some(Error::TooDeep));
}

#[test]
fn oversized_files_are_refused() {
    let mut huge = halves();
    huge.resize(MAX_SVG_BYTES + 1, b' ');
    assert_eq!(parse(&huge).err(), Some(Error::TooLarge));
}

#[test]
fn billion_laughs_is_bounded() {
    let lol = r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;"><!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;"><!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;"><!ENTITY e "&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;"><!ENTITY f "&e;&e;&e;&e;&e;&e;&e;&e;&e;&e;"><!ENTITY g "&f;&f;&f;&f;&f;&f;&f;&f;&f;&f;"><!ENTITY h "&g;&g;&g;&g;&g;&g;&g;&g;&g;&g;">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>&h;</text></svg>"#;
    assert!(parse(lol.as_bytes()).is_err(), "100 MB of expanded text is refused");
}

/// Review 1 #1: one wide entity referenced many times at depth 0 (which roxmltree allows without
/// limit) would expand to gigabytes. Scaled down: 64 KB × 2000 = 128 MB, refused before parsing.
#[test]
fn wide_entity_expansion_is_refused() {
    let doc = format!(
        r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY e "{}">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>{}</text></svg>"#,
        "a".repeat(16 << 10),
        "&e;".repeat(8000)
    );
    assert!(is_svg(doc.as_bytes()));
    let t = std::time::Instant::now();
    let r = parse(doc.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("entit")), "refused for its entities: {:?}", r.err());
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
}

/// Short entities, as Illustrator writes for its namespaces, still read.
#[test]
fn small_namespace_entities_are_accepted() {
    let doc = r##"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
	<!ENTITY ns_svg "http://www.w3.org/2000/svg">
	<!ENTITY ns_xlink "http://www.w3.org/1999/xlink">
]>
<svg version="1.1" xmlns="&ns_svg;" xmlns:xlink="&ns_xlink;" width="20" height="10"><rect width="10" height="10" fill="#f00"/></svg>"##;
    let svg = parse(doc.as_bytes()).unwrap();
    assert_eq!(svg.size_px(), (20.0, 10.0));
}

/// Review 1 #2: the declaration scan looked for `[` to the end of the file at every `<!`, which
/// is quadratic: a few MB of `<!a>` never finished.
#[test]
fn declarations_scan_in_linear_time() {
    let doc = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}</svg>"#, "<!a>".repeat(1 << 16));
    let t = std::time::Instant::now();
    let _ = parse(doc.as_bytes());
    assert!(t.elapsed() < std::time::Duration::from_secs(1), "{:?}", t.elapsed());
    let prolog = format!("{}<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"/>", "<!DOCTYPE a>".repeat(5000));
    let t = std::time::Instant::now();
    for _ in 0..20 {
        let _ = is_svg(prolog.as_bytes());
    }
    assert!(t.elapsed() < std::time::Duration::from_secs(1), "{:?}", t.elapsed());
}

/// Review 1 #3: text in an SVG draws with the fonts documents use (Source Sans 3 is compiled in,
/// so this holds without system fonts or craft-fonts).
#[test]
fn text_is_drawn_with_document_fonts() {
    let doc = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><text x="5" y="50" font-family="Source Sans 3" font-size="50" fill="black">WW</text></svg>"#;
    let r = parse(doc).unwrap().render(100, 60, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    let inked = r.rgba.chunks(4).filter(|p| p[3] > 128).count();
    assert!(inked > 200, "the text is drawn: {inked} inked pixels");
}

/// Review 1 #13: a prefixed root element is still an SVG.
#[test]
fn prefixed_svg_root_is_accepted() {
    let doc = br#"<svg:svg xmlns:svg="http://www.w3.org/2000/svg" width="10" height="10"><svg:rect width="10" height="10"/></svg:svg>"#;
    assert!(is_svg(doc));
    assert_eq!(natural_size(doc), Some((7.5, 7.5)));
    assert!(!is_svg(b"<w:svgish/>"));
}

#[test]
fn rasters_past_the_cap_are_refused() {
    let svg = parse(&halves()).unwrap();
    assert!(svg.render(100_000, 100_000, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).is_none());
    assert!(svg.render(0, 10, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).is_none());
    assert!(svg.render(10, 10, [f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]).is_none());
}

#[test]
fn external_pictures_are_never_loaded() {
    // A real PNG on disk, linked by absolute path and by file: URL: neither is read.
    let dir = std::env::temp_dir().join(format!("wordcraft-svg-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png_path = dir.join("red.png");
    let png = fallback_png(&halves()).unwrap();
    std::fs::write(&png_path, &png).unwrap();
    let p = png_path.to_string_lossy();
    let doc = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20" height="20"><image href="{p}" width="10" height="20"/><image xlink:href="file://{p}" x="10" width="10" height="20"/></svg>"#
    );
    let svg = parse(doc.as_bytes()).unwrap();
    let r = svg.render(20, 20, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    assert!(r.rgba.iter().all(|b| *b == 0), "nothing from the file is drawn");
    let _ = std::fs::remove_dir_all(&dir);
    // A data: picture is drawn.
    let b64 = base64_encode(&png);
    let inline = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><image href="data:image/png;base64,{b64}" width="20" height="20"/></svg>"#
    );
    let r = parse(inline.as_bytes()).unwrap().render(20, 20, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    assert!(r.rgba.iter().any(|b| *b != 0), "the data: picture is drawn");
}

fn base64_encode(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= ch.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[test]
fn chained_uses_and_nested_pictures_stay_within_the_stack() {
    // Each level wraps the previous one 200 deep through `use`: about 1000 levels once expanded.
    // Parsing and drawing must not overflow a test thread's 2 MB stack.
    let wrap = |inner: &str| format!("{}{inner}{}", "<g>".repeat(200), "</g>".repeat(200));
    let mut defs = format!(r#"<g id="l0">{}</g>"#, wrap(r##"<rect width="10" height="10" fill="#f00"/>"##));
    for i in 1..6 {
        defs.push_str(&format!(r##"<g id="l{i}">{}</g>"##, wrap(&format!(r##"<use href="#l{}"/>"##, i - 1))));
    }
    let doc = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs>{defs}</defs><use href="#l5"/></svg>"##);
    if let Ok(svg) = parse(doc.as_bytes()) {
        let _ = svg.render(10, 10, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
    // SVG pictures inside SVG pictures (data: links), nested 12 deep.
    let mut svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#f00"/></svg>"##.to_string();
    for _ in 0..12 {
        let b64 = base64_encode(svg.as_bytes());
        svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}<image href="data:image/svg+xml;base64,{b64}" width="10" height="10"/>{}</svg>"#,
            "<g>".repeat(200),
            "</g>".repeat(200)
        );
    }
    if let Ok(s) = parse(svg.as_bytes()) {
        let _ = s.render(10, 10, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
    // A gzip-compressed SVG inside a data: link is not expanded.
    let gz = base64_encode(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1, 2, 3]);
    let doc = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="data:image/svg+xml;base64,{gz}" width="10" height="10"/></svg>"#
    );
    assert!(parse(doc.as_bytes()).is_ok());
}

/// Review 2 N1: every entity declaration outside a DOCTYPE rescanned the whole file for `&`.
#[test]
fn many_entity_declarations_scan_quickly() {
    let doc = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}</svg>"#, "<!ENTITY a \"\">".repeat(20_000));
    let t = std::time::Instant::now();
    let _ = parse(doc.as_bytes());
    assert!(t.elapsed() < std::time::Duration::from_millis(500), "{:?}", t.elapsed());
}

/// Review 2 N2: each reference inside an entity value looked the name up in a list (quadratic),
/// and the number of declarations wasn't capped.
#[test]
fn entity_declarations_are_capped() {
    let mut decls = String::from("<!ENTITY e0 \"x\">");
    for i in 1..20_000 {
        decls.push_str(&format!("<!ENTITY e{i} \"&e{};\">", i - 1));
    }
    // After the root, so the sniff (which reads 64 KB of prolog) lets the file through to the scan.
    let doc = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{decls}</svg>"#);
    let t = std::time::Instant::now();
    let r = parse(doc.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("entit")), "{:?}", r.err());
    assert!(t.elapsed() < std::time::Duration::from_millis(500), "{:?}", t.elapsed());
}

/// Review 2 N3: text is laid out with real fonts now, so a huge text body is refused before usvg
/// shapes it — written out, or expanded from entities.
#[test]
fn huge_text_bodies_are_refused() {
    let plain = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>{}</text></svg>"#, "a".repeat(1_200_000));
    let r = parse(plain.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("text")), "{:?}", r.err());
}

#[test]
fn huge_entity_expanded_text_is_refused() {
    let expanded = format!(
        r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY e "{}">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text><tspan>{}</tspan></text></svg>"#,
        "a".repeat(4 << 10),
        "&e;".repeat(300)
    );
    let r = parse(expanded.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("text")), "{:?}", r.err());
}

#[test]
fn text_outside_text_elements_is_not_counted() {
    // Ordinary labels are fine, and text outside text elements (a title) doesn't count.
    let ok =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><title>{}</title><text>Label</text></svg>"#, "t".repeat(2 << 20));
    assert!(parse(ok.as_bytes()).is_ok());
}

/// An SVG picture with `n` bytes of (blank) text in a text element, as a `data:` link.
fn nested_text_picture(n: usize) -> String {
    let inner = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>{}a</text></svg>"#, " ".repeat(n));
    format!(r#"<image href="data:image/svg+xml;base64,{}" width="10" height="10"/>"#, base64_encode(inner.as_bytes()))
}

/// Review 3 #1: the text cap covers the whole picture, nested `data:` SVG pictures included,
/// rather than giving each nested picture a budget of its own.
#[test]
fn text_cap_is_shared_with_nested_pictures() {
    let one = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}</svg>"#, nested_text_picture(600_000));
    assert!(parse(one.as_bytes()).is_ok(), "one nested picture with 600 KB of text is fine");
    let two = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}{}</svg>"#,
        nested_text_picture(600_000),
        nested_text_picture(600_000)
    );
    let r = parse(two.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("text")), "1.2 MB of text across nested pictures is refused: {:?}", r.err());
    // The outer picture's own text counts too.
    let mixed = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>{}a</text>{}</svg>"#,
        " ".repeat(600_000),
        nested_text_picture(600_000)
    );
    let r = parse(mixed.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("text")), "{:?}", r.err());
}

/// Review 3 #2: only `<use>` copies of text multiply the text drawn: a chart with 100 000
/// markers and a short caption is fine, while many copies of a long text are still refused.
#[test]
fn use_copies_count_only_when_they_copy_text() {
    let markers = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><circle id="dot" r="1"/></defs>{}<text>{}</text></svg>"##,
        r##"<use href="#dot"/>"##.repeat(100_000),
        "c".repeat(300)
    );
    let r = parse(markers.as_bytes());
    assert!(r.is_ok(), "marker-heavy chart: {:?}", r.err());
    // Markers drawn through a group of `<use>`s of a shape copy no text either.
    let grouped = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><circle id="dot" r="1"/><g id="m"><use href="#dot"/></g></defs>{}<text>{}</text></svg>"##,
        r##"<use href="#m"/>"##.repeat(100_000),
        "c".repeat(300)
    );
    let r = parse(grouped.as_bytes());
    assert!(r.is_ok(), "grouped markers: {:?}", r.err());
    let copies = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><text id="t">{}</text></defs>{}</svg>"##,
        "c".repeat(100_000),
        r##"<use xlink:href="#t" xmlns:xlink="http://www.w3.org/1999/xlink"/>"##.repeat(200)
    );
    let r = parse(copies.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("text")), "{:?}", r.err());
    // A group of text copies, itself copied (and referenced before it is defined).
    let chained = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{}<defs><g id="g">{}</g><text id="t">{}</text></defs></svg>"##,
        r##"<use href="#g"/>"##.repeat(150),
        r##"<use href="#t"/>"##.repeat(10),
        "c".repeat(100_000)
    );
    let r = parse(chained.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("text")), "{:?}", r.err());
}

/// Fix round 3: usvg reads a nested `data:` picture once per copy of it, so the nested pictures
/// read for one picture share a cap ([`MAX_SVG_BYTES`] in all) rather than each copy having its own.
#[test]
fn nested_pictures_read_per_copy_share_a_cap() {
    let inner = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><!--{}--></svg>"#, " ".repeat(100_000));
    let image = format!(r#"<image id="i" href="data:image/svg+xml;base64,{}" width="10" height="10"/>"#, base64_encode(inner.as_bytes()));
    let svg = |copies: usize| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs>{image}</defs>{}</svg>"##,
            r##"<use href="#i"/>"##.repeat(copies)
        )
    };
    assert!(parse(svg(10).as_bytes()).is_ok(), "ten copies (1 MB read) are fine");
    let r = parse(svg(400).as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("nested")), "40 MB of nested pictures: {:?}", r.err());
}

/// `copies` of `link` after the definitions `def`, whose `TEXT` is `n` bytes of text.
fn copied_text(def: &str, link: &str, copies: usize, n: usize) -> String {
    let def = def.replace("TEXT", &"c".repeat(n));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:z="urn:z" width="10" height="10"><defs><circle id="dot" r="1"/>{def}</defs>{}</svg>"##,
        link.repeat(copies)
    )
}

fn refused_as_text(doc: &str) -> bool {
    matches!(parse(doc.as_bytes()), Err(Error::Parse(m)) if m.contains("text"))
}

/// Review 4 I-1: a `<use>` inside a group that points back at the group's ancestor is skipped by
/// usvg, but the text next to it is still built once per copy of the outer group.
#[test]
fn text_beside_a_self_reference_counts_per_copy() {
    let def = r##"<g id="P"><g id="t"><g id="G"><text>TEXT</text><use href="#t"/></g></g></g>"##;
    let doc = copied_text(def, r##"<use href="#P"/>"##, 200, 100_000);
    let t = std::time::Instant::now();
    assert!(refused_as_text(&doc), "20 MB of text drawn through copies of P");
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    // Without the copies it is an ordinary picture.
    assert!(parse(copied_text(def, "", 0, 100_000).as_bytes()).is_ok());
}

/// Review 4 I-2: links are read the way usvg reads them: character references resolved, spaces
/// around the `#id` allowed, and `xlink:href` taken before the first other attribute named `href`.
#[test]
fn use_links_are_read_like_usvg() {
    let def = r#"<text id="t">TEXT</text>"#;
    for link in [r##"<use href="&#35;t"/>"##, r##"<use href=" #t"/>"##, r##"<use href="#t "/>"##, r##"<use z:href="#dot" xlink:href="#t"/>"##] {
        let doc = copied_text(def, link, 200, 100_000);
        assert!(refused_as_text(&doc), "{link}");
    }
    // usvg looks a plain `href` up by its local name in any namespace (roxmltree's lookup), so a
    // link only in another namespace is followed too.
    assert!(refused_as_text(&copied_text(def, r##"<use z:href="#t"/>"##, 200, 100_000)));
}

/// Round 4: copies that lead back to themselves never end; usvg would build them a thousand
/// levels deep before giving up.
#[test]
fn endless_copies_are_refused() {
    let def = r##"<g id="a"><text>TEXT</text><use href="#b"/></g><g id="b"><use href="#a"/></g>"##;
    let r = parse(copied_text(def, "", 0, 1000).as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("never end")), "{:?}", r.err());
}

/// Round 4: every `<tref>` makes usvg look its target up by scanning the whole file, so many of
/// them in a large file would take minutes.
#[test]
fn many_trefs_in_a_large_file_are_refused_quickly() {
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>{}</text>{}<g id="far"/></svg>"##,
        r##"<tref href="#far"/>"##.repeat(20_000),
        "<g/>".repeat(200_000)
    );
    let t = std::time::Instant::now();
    let r = parse(doc.as_bytes());
    assert!(r.is_err(), "refused");
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
}

/// The number of text elements in a parsed picture (usvg's tree after conversion), counting those
/// drawn in markers, clip paths, masks, patterns and filter images too.
fn usvg_texts(g: &usvg::Group) -> u64 {
    g.children()
        .iter()
        .map(|n| {
            let mut k = match n {
                usvg::Node::Group(g) => usvg_texts(g),
                usvg::Node::Text(_) => 1,
                _ => 0,
            };
            // A nested picture's text is its own (counted when it is read).
            if !matches!(n, usvg::Node::Image(_)) {
                n.subroots(|r| k += usvg_texts(r));
            }
            k
        })
        .sum()
}

/// Round 4: the pre-check counts the text copies usvg builds, exactly, for small crafted
/// pictures (no `<defs>`, so every element built is drawn). A change in usvg's `<use>` rules
/// shows up here.
#[test]
fn text_copies_match_usvg() {
    let cases = [
        r##"<text>a</text><text>b</text>"##,
        r##"<text id="t">a</text><use href="#t"/><use href="#t"/><use href="#t"/>"##,
        r##"<g id="g"><text>a</text><text>b</text></g><use href="#g"/><use href="#g"/>"##,
        r##"<text id="t">a</text><g id="g1"><use href="#t"/><use href="#t"/></g><g id="g2"><use href="#g1"/><use href="#g1"/><use href="#g1"/></g><use href="#g2"/>"##,
        // Review 4 I-1, small.
        r##"<g id="P"><g id="t"><g id="G"><text>a</text><use href="#t"/></g></g></g><use href="#P"/><use href="#P"/><use href="#P"/>"##,
        // Review 4 I-2.
        r##"<text id="t">a</text><use href="&#35;t"/><use href=" #t "/><use z:href="#x" xlink:href="#t"/><use z:href="#t"/>"##,
        // A `<use>` of itself, of its own group, two `<use>`s of each other, a missing target.
        r##"<g id="g"><text>a</text><use id="u" href="#u"/><use href="#g"/></g><use id="a" href="#b"/><use id="b" href="#a"/><use href="#none"/>"##,
        // A group whose `<use>` points at a `<use>` of the group.
        r##"<g id="g1"><text>a</text><use href="#u2"/></g><use id="u2" href="#g1"/><use href="#g1"/>"##,
        // Later ids don't replace the first one.
        r##"<text id="t">a</text><text id="t">b</text><use href="#t"/>"##,
        // `<tref>` text, and a group holding a `<use>` of itself (which no `<use>` copies).
        r##"<text id="s">a</text><text>b<tref href="#s"/></text><g id="g"><text>c</text><use href="#g"/></g><use href="#g"/>"##,
        // Elements outside the SVG namespace aren't built.
        r##"<text id="t">a</text><z:g><text>b</text><use href="#t"/></z:g>"##,
        // Links are groups.
        r##"<a id="l" href="https://example.com"><text>a</text></a><use href="#l"/>"##,
        // A nested `data:` picture's text is counted when that picture is read, not here.
        &format!(r#"<text>a</text>{}"#, nested_text_picture(3)),
    ];
    let svg = |body: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:z="urn:z" width="100" height="100" font-family="Source Sans 3" font-size="10">{body}</svg>"#
        )
    };
    for body in cases {
        let doc = svg(body);
        let xml = usvg::roxmltree::Document::parse(&doc).unwrap();
        let counted = copies(&xml).unwrap().texts;
        let built = usvg_texts(parse(doc.as_bytes()).unwrap().tree().root());
        assert_eq!(counted, built, "{body}");
    }
    // Where usvg builds elements it doesn't draw (`<symbol>`, `<defs>`, the other `<switch>`
    // children), or draws copies the check bounds rather than counts (markers, clip paths, masks,
    // patterns, filter images), the count is at least what usvg draws.
    let bounded = [
        r##"<symbol id="s"><text>a</text></symbol><use href="#s"/><use href="#s"/>"##,
        r##"<defs><text id="t">a</text></defs><use href="#t"/>"##,
        r##"<switch><text>a</text><text>b</text></switch>"##,
        r##"<marker id="m" markerWidth="9" markerHeight="9" overflow="visible"><text>a</text></marker><polyline points="1,1 5,5 9,1 20,20" stroke="black" marker-start="url(#m)" marker-mid="url(#m)" marker-end="url(#m)"/>"##,
        r##"<clipPath id="c" clipPathUnits="objectBoundingBox"><text y="1" font-size="1">a</text></clipPath><rect width="9" height="9" clip-path="url(#c)"/><rect x="20" width="9" height="9" clip-path="url(#c)"/>"##,
        r##"<mask id="k"><text y="9">a</text></mask><rect width="9" height="9" mask="url(#k)"/><rect x="20" width="9" height="9" mask="url(#k)"/>"##,
        r##"<pattern id="p" width="9" height="9" patternUnits="userSpaceOnUse"><text y="9">a</text></pattern><rect width="9" height="9" fill="url(#p)"/><rect x="20" width="9" height="9" fill="url(#p)"/>"##,
        r##"<text id="t" y="9">a</text><filter id="f"><feImage href="#t"/></filter><rect width="9" height="9" filter="url(#f)"/><rect x="20" width="9" height="9" filter="url(#f)"/>"##,
        // Round 8: two markers in a loop (usvg builds each inside the other, never inside
        // itself), and a clip path taken again by `inherit`.
        r##"<marker id="a" overflow="visible"><text>a</text><path d="M0 0L5 5L9 0" stroke="black" marker-mid="url(#b)" marker-end="url(#a)"/></marker><marker id="b" overflow="visible"><text>b</text><path d="M0 0L5 5L9 0" stroke="black" marker-mid="url(#a)" marker-end="url(#b)"/></marker><polyline points="1,1 5,5 9,1 20,20" stroke="black" marker-mid="url(#a)"/>"##,
        r##"<clipPath id="c" clipPathUnits="objectBoundingBox"><text y="1" font-size="1">a</text></clipPath><g clip-path="url(#c)"><rect width="9" height="9" clip-path="inherit"/><rect x="20" width="9" height="9" clip-path="inherit"/></g>"##,
    ];
    for body in bounded {
        let doc = svg(body);
        let xml = usvg::roxmltree::Document::parse(&doc).unwrap();
        let counted = copies(&xml).unwrap().texts;
        let built = usvg_texts(parse(doc.as_bytes()).unwrap().tree().root());
        assert!(built > 0 && counted >= built, "{body}: counted {counted}, usvg drew {built}");
    }
}

/// `body` in a picture, with `TEXT` replaced by `n` bytes of text.
fn picture_with_text(body: &str, n: usize) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{}</svg>"#, body.replace("TEXT", &"c".repeat(n)))
}

fn refused_quickly(doc: &str) -> bool {
    let t = std::time::Instant::now();
    let r = parse(doc.as_bytes());
    let fast = t.elapsed() < std::time::Duration::from_secs(2);
    matches!(&r, Err(Error::Parse(m)) if m.contains("text")) && fast
}

/// Review 5 I-1: usvg draws a marker's content once per vertex of every shape that uses it.
#[test]
fn text_in_markers_counts_per_vertex() {
    let points: String = (0..2000).map(|i| format!("{i},{} ", i % 7)).collect();
    let marker = |text: &str, n| {
        picture_with_text(
            &format!(r##"<marker id="m" overflow="visible">{text}</marker><polyline points="{points}" stroke="black" marker-mid="url(#m)"/>"##),
            n,
        )
    };
    assert!(refused_quickly(&marker("<text>TEXT</text>", 100_000)), "200 MB of marker text");
    // A chart's dot markers (no text) on 2000 points, with a caption, opens.
    let chart = picture_with_text(
        &format!(r##"<marker id="m"><circle r="2"/></marker><polyline points="{points}" marker-mid="url(#m)"/><text y="20">TEXT</text>"##),
        200,
    );
    assert!(parse(chart.as_bytes()).is_ok());
    // A short label on a few points opens.
    let few = picture_with_text(r##"<marker id="m"><text>TEXT</text></marker><polyline points="1,1 5,5 9,1" marker-mid="url(#m)"/>"##, 10);
    assert!(parse(few.as_bytes()).is_ok());
}

/// `n` rects using `attr`, after `def`.
fn many_users(def: &str, attr: &str, n: usize) -> String {
    format!("{def}{}", format!(r#"<rect width="9" height="9" {attr}/>"#).repeat(n))
}

/// Review 5 I-1: usvg builds an `objectBoundingBox` clip path's or mask's content once per
/// element that uses it.
#[test]
fn text_in_clip_paths_and_masks_counts_per_use() {
    let clip = r##"<clipPath id="c" clipPathUnits="objectBoundingBox"><text>TEXT</text></clipPath>"##;
    assert!(refused_quickly(&picture_with_text(&many_users(clip, r##"clip-path="url(#c)""##, 2000), 10_000)), "clip path");
    let mask = r##"<mask id="k"><text>TEXT</text></mask>"##;
    assert!(refused_quickly(&picture_with_text(&many_users(mask, r##"mask="url(#k)""##, 2000), 10_000)), "mask");
    let filter = r##"<text id="t">TEXT</text><filter id="f"><feImage href="#t"/></filter>"##;
    assert!(refused_quickly(&picture_with_text(&many_users(filter, r##"filter="url(#f)""##, 2000), 10_000)), "filter image");
    // Ordinary clip paths and masks open.
    let plain = r##"<clipPath id="c"><rect width="5" height="5"/></clipPath><mask id="k"><rect width="5" height="5" fill="white"/></mask>"##;
    let doc = picture_with_text(&format!("{}<text>TEXT</text>", many_users(plain, r##"clip-path="url(#c)" mask="url(#k)""##, 2000)), 200);
    assert!(parse(doc.as_bytes()).is_ok());
    // A short text clip used a few times opens.
    let few = picture_with_text(&many_users(clip, r##"clip-path="url(#c)""##, 5), 20);
    assert!(parse(few.as_bytes()).is_ok());
}

/// A picture around `body`, for the reference tests.
fn pic(body: &str) -> String {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">{body}</svg>"##)
}

/// Whether `doc` is refused for references that lead back to themselves. (usvg builds the things
/// such links name by building what they hold, without limit: a stack overflow, which aborts the
/// process, so each shape below is its own test.)
fn refused_as_loop(doc: &str) -> bool {
    matches!(parse(doc.as_bytes()), Err(Error::Parse(m)) if m.contains("in a loop"))
}

const PAT: &str = r#"patternUnits="userSpaceOnUse" width="10" height="10""#;

/// Review 6 C-1 (a): three patterns in a ring.
#[test]
fn loop_pattern_ring() {
    let d = pic(&format!(
        r##"<defs><pattern id="p1" {PAT}><rect width="5" height="5" fill="url(#p2)"/></pattern><pattern id="p2" {PAT}><rect width="5" height="5" fill="url(#p3)"/></pattern><pattern id="p3" {PAT}><rect width="5" height="5" fill="url(#p1)"/></pattern></defs><rect width="50" height="50" fill="url(#p1)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (b): two patterns, fill and stroke.
#[test]
fn loop_pattern_fill_and_stroke() {
    let d = pic(&format!(
        r##"<defs><pattern id="p1" {PAT}><rect width="5" height="5" fill="url(#p2)"/></pattern><pattern id="p2" {PAT}><rect width="5" height="5" stroke="url(#p1)"/></pattern></defs><rect width="50" height="50" fill="url(#p1)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (c): a pattern and a mask.
#[test]
fn loop_pattern_and_mask() {
    let d = pic(&format!(
        r##"<defs><pattern id="p" {PAT}><rect width="5" height="5" mask="url(#m)"/></pattern><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="50" height="50" fill="url(#p)"/></mask></defs><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (d): three masks in a ring.
#[test]
fn loop_mask_ring() {
    let m = |a: &str, b: &str| {
        format!(
            r##"<mask id="{a}" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="50" height="50" fill="white" mask="url(#{b})"/></mask>"##
        )
    };
    let d = pic(&format!(r##"<defs>{}{}{}</defs><rect width="50" height="50" mask="url(#m1)"/>"##, m("m1", "m2"), m("m2", "m3"), m("m3", "m1")));
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (e): three clip paths whose own `clip-path` leads round.
#[test]
fn loop_clip_path_ring() {
    let d = pic(
        r##"<defs><clipPath id="c1" clip-path="url(#c2)"><rect width="9" height="9"/></clipPath><clipPath id="c2" clip-path="url(#c3)"><rect width="9" height="9"/></clipPath><clipPath id="c3" clip-path="url(#c1)"><rect width="9" height="9"/></clipPath></defs><rect width="50" height="50" clip-path="url(#c1)"/>"##,
    );
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (f): a filter, its image, a pattern.
#[test]
fn loop_filter_image_pattern() {
    let d = pic(&format!(
        r##"<defs><filter id="f"><feImage href="#r"/></filter><rect id="r" width="5" height="5" fill="url(#p)"/><pattern id="p" {PAT}><rect width="5" height="5" filter="url(#f)"/></pattern></defs><rect width="50" height="50" filter="url(#f)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (g): a mask, a filter, its image.
#[test]
fn loop_mask_filter_image() {
    let d = pic(
        r##"<defs><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="50" height="50" fill="white" filter="url(#f)"/></mask><filter id="f"><feImage href="#r"/></filter><rect id="r" width="5" height="5" mask="url(#m)"/></defs><rect width="50" height="50" mask="url(#m)"/>"##,
    );
    assert!(refused_as_loop(&d));
}

/// Review 6 C-1 (j): a pattern inherits the paint of the group around it, which names the pattern.
#[test]
fn loop_through_inherited_paint() {
    let d = pic(&format!(r##"<g fill="url(#p)"><pattern id="p" {PAT}><rect width="5" height="5"/></pattern><rect width="50" height="50"/></g>"##));
    assert!(refused_as_loop(&d));
    // A pattern template chain that comes back to a later link (usvg's template walk never ends).
    let rho = pic(&format!(
        r##"<pattern id="a" {PAT} href="#b"/><pattern id="b" {PAT} href="#c"/><pattern id="c" {PAT} href="#b"><rect width="5" height="5"/></pattern><rect width="50" height="50" fill="url(#a)"/>"##
    ));
    assert!(refused_as_loop(&rho));
    // A pattern's own fill or stroke naming itself (in a style sheet, a `style` attribute, or
    // through a `<use>`) is removed by usvg, so these are drawn.
    let ok = |body: &str| {
        let r = parse(pic(body).as_bytes());
        assert!(r.is_ok(), "{body}: {:?}", r.err());
    };
    ok(&format!(
        r##"<style>.k{{fill:url(#p)}}</style><pattern id="p" {PAT}><rect class="k" width="5" height="5"/></pattern><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    ok(&format!(
        r##"<pattern id="p" {PAT}><rect style="stroke:url( '#p' )" width="5" height="5"/></pattern><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    ok(&format!(
        r##"<g id="g" fill="url(#p)"><rect width="5" height="5"/></g><pattern id="p" {PAT}><use href="#g"/></pattern><rect width="50" height="50" fill="url(#p)"/>"##
    ));
}

/// Review 6: shapes that don't loop, or that usvg stops itself, are drawn.
#[test]
fn references_without_loops_are_accepted() {
    let ok = |body: &str| {
        let r = parse(pic(body).as_bytes());
        assert!(r.is_ok(), "{body}: {:?}", r.err());
    };
    // A pattern whose marker draws the pattern (usvg doesn't build a marker inside itself).
    ok(&format!(
        r##"<pattern id="p" {PAT}><path d="M0 0L5 5L9 0" stroke="black" marker-mid="url(#m)"/></pattern><marker id="m" markerWidth="4" markerHeight="4"><rect width="2" height="2" fill="url(#p)"/></marker><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    // A `<use>` in a pattern that links to the shape using the pattern.
    ok(&format!(r##"<rect id="r" width="50" height="50" fill="url(#p)"/><pattern id="p" {PAT}><use href="#r"/></pattern>"##));
    // A style sheet's two-hop masks.
    ok(
        r##"<style>.a{mask:url(#m2)}.b{mask:url(#m1)}</style><mask id="m1" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect class="a" width="50" height="50" fill="white"/></mask><mask id="m2" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="50" height="50" fill="white"/></mask><rect class="b" width="50" height="50"/>"##,
    );
    // Chains that end, a gradient pattern, a clip path inside a masked group.
    ok(&format!(
        r##"<linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><clipPath id="c"><rect width="30" height="30"/></clipPath><pattern id="p" {PAT} href="#q"/><pattern id="q" {PAT}><rect width="5" height="5" fill="url(#g)" clip-path="url(#c)"/></pattern><g fill="url(#g)" clip-path="url(#c)"><rect width="50" height="50" fill="url(#p)"/></g>"##
    ));
}

/// Review 6 I-1: an Illustrator-style logo (style sheet with `url(`s, clip paths, a text mask,
/// many paths) is an ordinary picture.
#[test]
fn illustrator_style_logo_is_accepted() {
    let clips: String = (0..5).map(|i| format!(r##"<clipPath id="clippath-{i}"><rect width="100" height="100"/></clipPath>"##)).collect();
    let paths: String = (0..200).map(|i| format!(r##"<path class="cls-1" d="M{i} 0h5v5z"/>"##)).collect();
    let d = pic(&format!(
        r##"<defs><style>.cls-1{{fill:url(#lg)}}.cls-2{{clip-path:url(#clippath-0)}}.cls-3{{clip-path:url(#clippath-1)}}.cls-4{{mask:url(#mask)}}</style><linearGradient id="lg"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>{clips}<mask id="mask" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100"><rect width="100" height="100" fill="#fff"/><text y="50" font-size="10">ACME Studio</text></mask></defs><g class="cls-2"><g class="cls-3">{paths}</g></g><g class="cls-4"><rect width="100" height="100" fill="#ccc"/></g><text y="90" font-size="8">ACME Studio, since 1920</text>"##
    ));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Review 6 M-5: text in a pattern is built again for every element the pattern paints.
#[test]
fn text_in_patterns_counts_per_use() {
    let pattern = format!(r##"<pattern id="p" {PAT}><text y="9">TEXT</text></pattern>"##);
    assert!(refused_quickly(&picture_with_text(&many_users(&pattern, r##"fill="url(#p)""##, 2000), 10_000)), "pattern text");
    let few = picture_with_text(&many_users(&pattern, r##"fill="url(#p)""##, 5), 20);
    assert!(parse(few.as_bytes()).is_ok());
}

fn refused_as_elements(doc: &str) -> bool {
    let t = std::time::Instant::now();
    let r = parse(doc.as_bytes());
    let fast = t.elapsed() < std::time::Duration::from_secs(5);
    matches!(&r, Err(Error::Parse(m)) if m.contains("this many elements") || m.contains("draws this many")) && fast
}

/// Review 6 C-2: markers and clip paths copy their content (not only text) per vertex or per user.
#[test]
fn elements_built_through_markers_and_clips_are_capped() {
    // 10,000 rects in a marker, 100,000 vertices (a file of about half a megabyte).
    let marker = format!(
        r##"<marker id="m" overflow="visible">{}</marker><path d="M0 0{}" stroke="black" marker-mid="url(#m)"/>"##,
        r#"<rect width="1" height="1"/>"#.repeat(10_000),
        "h1".repeat(100_000)
    );
    assert!(refused_as_elements(&pic(&marker)));
    // Clip path shapes times users.
    let clip = format!(r##"<clipPath id="c" clipPathUnits="objectBoundingBox">{}</clipPath>"##, r#"<rect width="1" height="1"/>"#.repeat(5000));
    assert!(refused_as_elements(&pic(&many_users(&clip, r##"clip-path="url(#c)""##, 2000))));
    // A chart's dot markers on 2000 points open.
    let points: String = (0..2000).map(|i| format!("{i},{} ", i % 7)).collect();
    let chart =
        pic(&format!(r##"<marker id="m"><circle r="2"/><circle r="1"/><circle r="3"/></marker><polyline points="{points}" marker-mid="url(#m)"/>"##));
    let r = parse(chart.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Review 7 (n1): an earlier element with the same id doesn't hide a pattern from the check.
#[test]
fn loop_behind_a_duplicate_id() {
    let d = pic(&format!(
        r##"<defs><g id="p"/><pattern id="p" {PAT}><rect width="5" height="5" mask="url(#m)"/></pattern><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="50" height="50" fill="url(#p)"/></mask></defs><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    assert!(refused_as_loop(&d));
    let later = d.replace(r##"<g id="p"/>"##, "").replace("</defs>", r##"<g id="p"/></defs>"##);
    assert!(refused_as_loop(&later));
}

/// Review 7 (n2): a filter image draws its target's whole subtree, which may use the filter.
#[test]
fn loop_below_a_filter_image_target() {
    let d = pic(
        r##"<filter id="f"><feImage href="#g"/></filter><g id="g"><rect width="5" height="5" filter="url(#f)"/></g><rect width="50" height="50" filter="url(#f)"/>"##,
    );
    assert!(refused_as_loop(&d));
}

/// Review 7 (n3): style sheets are read in any namespace, as usvg reads them.
#[test]
fn loop_through_a_sheet_in_another_namespace() {
    let m = |a: &str, c: &str| {
        format!(
            r##"<mask id="{a}" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect class="{c}" width="50" height="50" fill="white"/></mask>"##
        )
    };
    let d = pic(&format!(
        r##"<x:style xmlns:x="urn:x">.a{{mask:url(#m2)}} .b{{mask:url(#m3)}} .c{{mask:url(#m1)}}</x:style><defs>{}{}{}</defs><rect width="50" height="50" mask="url(#m1)"/>"##,
        m("m1", "a"),
        m("m2", "b"),
        m("m3", "c")
    ));
    assert!(refused_as_loop(&d));
}

/// Review 7 (n4): a comment in a `style` attribute doesn't hide the property.
#[test]
fn loop_through_a_style_attribute_with_a_comment() {
    let d = pic(&format!(
        r##"<g style="/**/fill:url(#p)"><pattern id="p" {PAT}><rect width="5" height="5"/></pattern><rect width="50" height="50"/></g>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 7 (n5): a pattern's template is the first `href` attribute, whichever the `<use>` rule picks.
#[test]
fn loop_through_either_href() {
    let d = pic(&format!(
        r##"<pattern id="s" {PAT} href="#t" xlink:href="#u"/><pattern id="t" {PAT}><rect width="5" height="5" fill="url(#s)"/></pattern><pattern id="u" {PAT}><rect width="5" height="5"/></pattern><rect width="50" height="50" fill="url(#s)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 7 (n6): the same for a filter image.
#[test]
fn loop_through_either_filter_image_href() {
    let d = pic(&format!(
        r##"<defs><filter id="f"><feImage href="#r" xlink:href="#ok"/></filter><rect id="r" width="5" height="5" fill="url(#p)"/><rect id="ok" width="5" height="5"/><pattern id="p" {PAT}><rect width="5" height="5" filter="url(#f)"/></pattern></defs><rect width="50" height="50" filter="url(#f)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 7 (n7): a marker beside a loop that doesn't go through it must not hide the loop.
#[test]
fn loop_beside_a_marker() {
    let d = pic(&format!(
        r##"<defs><pattern id="p" {PAT}><path d="M0 0L5 5L9 0" stroke="black" marker-mid="url(#m)"/><rect width="5" height="5" mask="url(#q)"/></pattern><marker id="m" markerWidth="4" markerHeight="4"><rect width="2" height="2" mask="url(#q)"/></marker><mask id="q" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="50" height="50" fill="url(#p)"/></mask></defs><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 7 C-3: a marker holding a long path, used by a long path, builds a segment per segment pair.
#[test]
fn big_paths_in_markers_are_weighted() {
    let long = format!("M0 0{}", "h1".repeat(60_000));
    let d = pic(&format!(r##"<marker id="m" overflow="visible"><path d="{long}"/></marker><path d="{long}" stroke="black" marker-mid="url(#m)"/>"##));
    let t = std::time::Instant::now();
    let r = parse(d.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("many elements")), "{:?}", r.err());
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
}

fn refused_as_selectors(doc: &str) -> bool {
    let t = std::time::Instant::now();
    let r = parse(doc.as_bytes());
    matches!(&r, Err(Error::Parse(m)) if m.contains("selectors")) && t.elapsed() < std::time::Duration::from_secs(5)
}

/// Review 7 C-4: simplecss matches a selector by recursion along its combinators, deep enough to
/// overflow the stack; in any namespace, with or without clip paths and the like.
#[test]
fn long_selectors_are_refused() {
    let sel = "*+".repeat(100_000);
    let plain = pic(&format!("<style>{sel}*{{fill:red}}</style>{}", "<g/>".repeat(100_000)));
    assert!(refused_as_selectors(&plain));
    let foreign = pic(&format!(
        r##"<x:style xmlns:x="urn:x">{sel}*{{fill:red}}</x:style><clipPath id="c"><rect width="5" height="5"/></clipPath><rect width="9" height="9" clip-path="url(#c)"/>{}"##,
        "<g/>".repeat(100_000)
    ));
    assert!(refused_as_selectors(&foreign));
}

/// Review 7 I-3: a descendant selector with many hops against a deep tree backtracks forever.
#[test]
fn backtracking_selectors_are_refused() {
    let deep = format!("{}<rect width=\"5\" height=\"5\"/>{}", "<g>".repeat(200), "</g>".repeat(200));
    let d = pic(&format!("<style>q g g g g g g g g g g g g rect{{fill:red}}</style>{deep}"));
    assert!(refused_as_selectors(&d));
    // A few hops in an ordinary tree are fine.
    let ok = pic("<style>g g rect{fill:red} g > rect{fill:blue} .a + .b{fill:green}</style><g><g><rect width=\"5\" height=\"5\"/></g></g>");
    assert!(parse(ok.as_bytes()).is_ok());
}

/// Review 7 I-3: a gradient template chain that comes back to a later link never ends in usvg.
#[test]
fn gradient_template_chains_that_loop_are_refused() {
    let d = pic(
        r##"<linearGradient id="a" href="#b"/><linearGradient id="b" href="#c"/><linearGradient id="c" href="#b"><stop offset="0" stop-color="red"/></linearGradient><rect width="50" height="50" fill="url(#a)"/>"##,
    );
    assert!(refused_as_loop(&d));
    let ok = pic(
        r##"<linearGradient id="a" href="#b"/><linearGradient id="b"><stop offset="0" stop-color="red"/></linearGradient><rect width="50" height="50" fill="url(#a)"/>"##,
    );
    assert!(parse(ok.as_bytes()).is_ok());
}

/// Review 7 I-2: a web font and a long comment in a sheet are ordinary.
#[test]
fn sheets_with_fonts_and_comments_are_accepted() {
    let font = "A".repeat(40_000);
    let d = pic(&format!(
        r##"<style>@font-face{{font-family:"F";src:url(data:font/woff2;base64,{font})}} .a{{clip-path:url(#c)}}</style><clipPath id="c"><rect width="5" height="5"/></clipPath><rect class="a" width="9" height="9"/>"##
    ));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
    let comment = "x".repeat(2800);
    let d = pic(&format!(
        r##"<style>/* generated by {comment} */ .a{{clip-path:url(#c)}}</style><clipPath id="c"><rect width="5" height="5"/></clipPath><rect class="a" width="9" height="9"/>"##
    ));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Review 8 C-5 (h1_feimg_anc): a filter image's target is built where it stands, so it inherits
/// paint from the group around it; that paint leads back to the filter.
#[test]
fn loop_through_a_filter_image_targets_group() {
    let d = pic(&format!(
        r##"<filter id="f"><feImage href="#r"/></filter><g fill="url(#p)"><rect id="r" width="5" height="5"/></g><pattern id="p" {PAT}><rect width="5" height="5" filter="url(#f)"/></pattern><rect width="50" height="50" filter="url(#f)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 8 C-5 (h1c): the same, with the loop going on through a mask.
#[test]
fn loop_through_a_filter_image_targets_group_and_a_mask() {
    let d = pic(&format!(
        r##"<filter id="f"><feImage href="#r"/></filter><g fill="url(#p)"><rect id="r" width="5" height="5"/></g><pattern id="p" {PAT}><rect width="5" height="5" mask="url(#m)"/></pattern><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="50"><rect width="5" height="5" fill="white" filter="url(#f)"/></mask><rect width="50" height="50" filter="url(#f)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// Review 8 C-6 (h7_css_string): a quote at a style sheet's top level doesn't hide the selector
/// after it (simplecss skips to the next `{` and reads on).
#[test]
fn selectors_after_a_top_level_quote_are_measured() {
    let sel = "*+".repeat(100_000);
    let d = pic(&format!(r#"<style>"{{}} {sel}*{{fill:red}} "</style>{}"#, "<g/>".repeat(100_000)));
    assert!(refused_as_selectors(&d));
}

/// Round 8: `inherit` takes a clip path, mask or filter from the parent, though they are not
/// inherited otherwise (here a clip path's own clip path, from the group around it).
#[test]
fn loop_through_an_inherit_keyword() {
    let d = pic(
        r##"<g clip-path="url(#B)"><clipPath id="A" clip-path="inherit"><rect width="5" height="5"/></clipPath></g><clipPath id="B" clip-path="url(#C)"><rect width="5" height="5"/></clipPath><clipPath id="C" clip-path="url(#A)"><rect width="5" height="5"/></clipPath><rect width="50" height="50" clip-path="url(#A)"/>"##,
    );
    assert!(refused_as_loop(&d));
}

/// Round 8: the same for a filter image's target, whose filter comes from its group.
#[test]
fn loop_through_an_inherit_keyword_below_a_filter_image() {
    let d = pic(
        r##"<filter id="f1"><feImage href="#r1"/></filter><filter id="f2"><feImage href="#r2"/></filter><g filter="url(#f2)"><rect id="r1" width="5" height="5" filter="inherit"/></g><rect id="r2" width="5" height="5" filter="url(#f1)"/><rect width="50" height="50" filter="url(#f1)"/>"##,
    );
    assert!(refused_as_loop(&d));
}

/// Round 8: an `id` in another namespace before the element's own one doesn't rename it (usvg
/// skips the foreign attribute and reads the next).
#[test]
fn loop_behind_a_foreign_id() {
    let d = pic(&format!(
        r##"<pattern xmlns:x="urn:x" x:id="zzz" id="p" {PAT}><rect width="5" height="5" fill="url(#q)"/></pattern><pattern id="q" {PAT}><rect width="5" height="5" fill="url(#s)"/></pattern><pattern id="s" {PAT}><rect width="5" height="5" fill="url(#p)"/></pattern><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    assert!(refused_as_loop(&d));
}

/// An Illustrator-style export: a class rule per gradient (`.cls-N{fill:url(#linear-gradient-N)}`),
/// gradients chained by `xlink:href`, a clip path, many paths.
fn illustrator_export(gradients: usize, paths: usize, stops: usize) -> String {
    let mut css = String::from(".cls-1{fill:none;}.cls-2{fill:#231f20;}.cls-3{clip-path:url(#clippath);}.cls-4{opacity:.5;}");
    let mut defs = String::from(r#"<clipPath id="clippath"><rect class="cls-1" width="1000" height="1000"/></clipPath>"#);
    for g in 0..gradients {
        let id = if g == 0 { "linear-gradient".to_string() } else { format!("linear-gradient-{g}") };
        css.push_str(&format!(".cls-{}{{fill:url(#{id});}}", g + 5));
        if g % 2 == 1 {
            defs.push_str(&format!(
                r##"<linearGradient id="{id}" x1="{g}" y1="0" x2="{}" y2="40" xlink:href="#linear-gradient" gradientUnits="userSpaceOnUse"/>"##,
                g + 50
            ));
        } else {
            let st: String =
                (0..stops).map(|i| format!(r##"<stop offset="{}" stop-color="#{:06x}"/>"##, i as f32 / (stops - 1) as f32, i * 40_000)).collect();
            defs.push_str(&format!(
                r#"<linearGradient id="{id}" x1="{g}" y1="0" x2="{}" y2="40" gradientUnits="userSpaceOnUse">{st}</linearGradient>"#,
                g + 50
            ));
        }
    }
    let path = |p: usize| {
        let class = if p.is_multiple_of(3) { "cls-2".to_string() } else { format!("cls-{}", 5 + p % gradients) };
        format!(r#"<path class="{class}" d="M{}.5,{}.25c1.2-3.4,5.6-7.8,9-1.1s-2.2,6.6-4.4,8.8-6.1-1-5.6-4.4Z"/>"#, p % 900, p / 9 % 900)
    };
    let first: String = (0..paths / 2).map(path).collect();
    let rest: String = (paths / 2..paths).map(path).collect();
    format!(
        r#"<svg id="Layer_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 1000 1000"><defs><style>{css}</style>{defs}</defs><g class="cls-3"><g class="cls-4">{first}</g>{rest}</g></svg>"#
    )
}

/// Review 8 I-4: usvg keeps one paint server per gradient, so many paths painted with many
/// gradient classes are not copies of anything.
#[test]
fn illustrator_gradient_classes_are_accepted() {
    let d = illustrator_export(40, 3000, 5);
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Round 8: a filter list reads `url (#f)` with a space before the bracket, and an id up to the
/// next space or bracket.
#[test]
fn loop_through_a_spaced_filter_url() {
    let d = pic(
        r##"<filter id="f1"><feImage href="#r1"/></filter><filter id="f2"><feImage href="#r2"/></filter><rect id="r1" width="5" height="5" filter="url (#f2)"/><rect id="r2" width="5" height="5" filter="url( #f1 )"/><rect width="50" height="50" filter="url(#f1)"/>"##,
    );
    assert!(refused_as_loop(&d));
}

/// Round 8: a `<use>` copy inherits from the `<use>`, not from the elements around its target, so
/// a pattern may copy a shape that sits in a group painted with that pattern.
#[test]
fn use_copies_inherit_from_the_use() {
    let d = pic(&format!(
        r##"<g fill="url(#p)"><rect id="r" width="5" height="5"/></g><pattern id="p" {PAT}><use href="#r"/></pattern><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Round 8: gradients cost no copies (I-4), but usvg walks a template chain from the start for
/// every attribute it looks up, so long chains of references are refused, quickly.
#[test]
fn long_reference_chains_are_refused() {
    let n = 20_000;
    let chain: String = (0..n).map(|i| format!(r##"<linearGradient id="g{i}" href="#g{}"/>"##, i + 1)).collect();
    let users: String = (0..n).map(|i| format!(r##"<rect width="5" height="5" fill="url(#g{i})"/>"##)).collect();
    let d = pic(&format!(
        r#"{chain}<linearGradient id="g{n}"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient>{users}"#
    ));
    let t = std::time::Instant::now();
    let r = parse(d.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("this deeply")), "{:?}", r.err());
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    let clips: String =
        (0..2000).map(|i| format!(r##"<clipPath id="c{i}" clip-path="url(#c{})"><rect width="5" height="5"/></clipPath>"##, i + 1)).collect();
    let d =
        pic(&format!(r##"{clips}<clipPath id="c2000"><rect width="5" height="5"/></clipPath><rect width="50" height="50" clip-path="url(#c0)"/>"##));
    assert!(matches!(parse(d.as_bytes()), Err(Error::Parse(m)) if m.contains("this deeply")));
    // A few hops are ordinary.
    let d = pic(
        r##"<linearGradient id="a" href="#b"/><linearGradient id="b" href="#c"/><linearGradient id="c"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><clipPath id="k" clip-path="url(#l)"><rect width="9" height="9" fill="url(#a)"/></clipPath><clipPath id="l"><rect width="9" height="9"/></clipPath><rect width="50" height="50" fill="url(#a)" clip-path="url(#k)"/>"##,
    );
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Round 8: a loop through more markers than usvg can be counted on is refused; a loop through
/// one marker (a shape in it that inherits the marker) opens.
#[test]
fn loops_through_many_markers_are_refused() {
    let k = 8;
    let content: String = (0..k).map(|j| format!(r##"<path d="M0 0L1 1L2 0" stroke="black" marker-mid="url(#m{j})"/>"##)).collect();
    let markers: String = (0..k).map(|i| format!(r##"<marker id="m{i}" overflow="visible">{content}</marker>"##)).collect();
    let d = pic(&format!(r##"{markers}<path d="M0 0L1 1L2 0" stroke="black" marker-mid="url(#m0)"/>"##));
    assert!(refused_as_loop(&d));
    let d = pic(
        r##"<style>path{marker-end:url(#arrow)}</style><marker id="arrow" overflow="visible"><path d="M0 0L4 2L0 4z"/></marker><path d="M0 0L50 50" stroke="black"/>"##,
    );
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Round 8: an Illustrator-style export with a clip path per class rule
/// (`.cls-N{clip-path:url(#clippath-N)}`), each on a group of paths. Each clip path is built for
/// the elements its rule matches, not for every element of the picture.
#[test]
fn illustrator_clip_path_classes_are_accepted() {
    let (clips, groups) = (300, 300);
    let mut css = String::from(".cls-1{fill:none;}.cls-2{fill:#231f20;}");
    let mut defs = String::new();
    for c in 0..clips {
        let id = if c == 0 { "clippath".to_string() } else { format!("clippath-{c}") };
        css.push_str(&format!(".cls-{}{{clip-path:url(#{id});}}", c + 3));
        defs.push_str(&format!(r#"<clipPath id="{id}"><path class="cls-1" d="M{c},0h500v500h-500z"/></clipPath>"#));
    }
    let body: String = (0..groups)
        .map(|g| {
            let paths: String =
                (0..20).map(|p| format!(r#"<path class="cls-2" d="M{p}.5,{g}.25c1.2-3.4,5.6-7.8,9-1.1s-2.2,6.6-4.4,8.8Z"/>"#)).collect();
            format!(r#"<g class="cls-{}">{paths}</g>"#, g % clips + 3)
        })
        .collect();
    let d = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><defs><style>{css}</style>{defs}</defs>{body}</svg>"#);
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Round 8: `url(`s whose ids never end are read in linear time (each used to search to the end
/// of the text again).
#[test]
fn many_unended_urls_scan_quickly() {
    let urls = "url(#a".repeat(150_000);
    let d = pic(&format!(r#"<clipPath id="c"><rect width="5" height="5"/></clipPath><rect width="9" height="9" fill="{urls}"/>"#));
    let t = std::time::Instant::now();
    let _ = parse(d.as_bytes());
    assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
}

/// Review 9 C-7: usvg copies a bounding-box gradient's stops for every shape it paints (the
/// cached one is shared, so it can't be changed in place).
#[test]
fn gradient_stops_count_per_shape() {
    let stops: String = (0..20_000).map(|i| format!(r#"<stop offset="{:.6}"/>"#, i as f64 / 20_000.0)).collect();
    let users = r##"<rect width="1" height="1" fill="url(#g)"/>"##.repeat(1000);
    let d = pic(&format!(r#"<linearGradient id="g">{stops}</linearGradient>{users}"#));
    assert!(refused_as_elements(&d));
}

/// Review 9 C-7: an Illustrator export heavy in gradients (stops, `xlink:href` chains, classes)
/// still opens.
#[test]
fn gradient_heavy_illustrator_file_is_accepted() {
    let d = illustrator_export(120, 12_000, 6);
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// 250 groups with opacity, one inside the other, around a picture-sized rect.
fn nested_layers(n: usize) -> String {
    pic(&format!("{}<rect width=\"100\" height=\"100\" fill=\"red\"/>{}", "<g opacity=\"0.9\">".repeat(n), "</g>".repeat(n)))
}

/// Review 9 C-8: resvg holds a picture-sized layer for every group it draws apart, one inside the
/// other: a large raster of many nested layers is drawn smaller and scaled up, within the budget.
#[test]
fn nested_layers_draw_within_the_budget() {
    // Picture-sized layers (the group's bounds and two pixels each way) held at once, as the
    // drawing estimate counts them: what `layers` counted in round 9.
    let layers = |svg: &Svg, side: u32| {
        let k = side as f32 / 100.0;
        let raster = u64::from(side) * u64::from(side) * 4;
        let layer = u64::from(side + 4) * u64::from(side + 4) * 4;
        (svg.draw_bytes(side, side, [k, 0.0, 0.0, k, 0.0, 0.0]) - raster) / layer
    };
    let svg = parse(nested_layers(250).as_bytes()).unwrap();
    assert!(layers(&svg, 2048) >= 250, "{}", layers(&svg, 2048));
    let r = svg.render(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]).unwrap();
    assert_eq!((r.width, r.height), (2048, 2048));
    // The middle is painted (red, at 0.9^250 opacity: faint but not empty is not required).
    let few = parse(nested_layers(3).as_bytes()).unwrap();
    assert_eq!(layers(&few, 512), 3);
    let r = few.render(512, 512, [5.12, 0.0, 0.0, 5.12, 0.0, 0.0]).unwrap();
    assert!(px(&r, 256, 256)[3] > 0);
    // A deep `<use>` chain under nested groups, opacity on each.
    let uses: String = (1..380).map(|i| format!(r##"<use id="u{i}" href="#u{}" opacity="0.9"/>"##, i - 1)).collect();
    let d = pic(&format!(
        r##"<defs><rect id="u0" width="100" height="100"/>{uses}</defs>{}<use href="#u379" opacity="0.9"/>{}"##,
        "<g opacity=\"0.9\">".repeat(250),
        "</g>".repeat(250)
    ));
    if let Ok(svg) = parse(d.as_bytes()) {
        assert!(svg.render(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]).is_some());
    }
}

/// A group 250 deep holding 700 rects (for the selector tests).
fn deep_group() -> String {
    format!(r#"<g id="ch">{}{}{}</g>"#, "<g>".repeat(250), r#"<rect width="1" height="1"/>"#.repeat(700), "</g>".repeat(250))
}

/// Review 9 I-5: usvg matches the style sheets against every `<use>` copy.
#[test]
fn selector_work_counts_copies() {
    let rules = ".zz * rect{fill:red}".repeat(8);
    let d = pic(&format!(r##"<style>{rules}</style><defs>{}</defs>{}"##, deep_group(), r##"<use href="#ch"/>"##.repeat(5)));
    let t = std::time::Instant::now();
    let _ = parse(d.as_bytes());
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "copies: {:?}", t.elapsed());
}

/// Review 9 I-5: the check matches an element once, however many specials reach it.
#[test]
fn selector_work_counts_each_element_once() {
    let rules: String = (0..8).map(|i| format!(".zz * rect{{fill:url(#f{i})}}")).collect();
    let filters: String = (0..50).map(|i| format!(r##"<filter id="f{i}"><feImage href="#ch"/></filter>"##)).collect();
    let d = pic(&format!("<style>{rules}</style><defs>{filters}</defs>{}", deep_group()));
    let t = std::time::Instant::now();
    let _ = parse(d.as_bytes());
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "specials: {:?}", t.elapsed());
}

/// Review 9 I-6: how deep usvg recurses is measured (`<use>` chains and what specials hold), so
/// the web, without a large stack, can refuse what wouldn't fit.
#[test]
fn build_depth_is_measured() {
    let chain = |p: &str, n: usize, last: &str| {
        let mut s = format!(r#"<rect id="{p}0" width="100" height="100" fill="white" {last}/>"#);
        s.extend((1..n).map(|i| format!(r##"<use id="{p}{i}" href="#{p}{}" opacity="0.9"/>"##, i - 1)));
        s
    };
    let d = pic(&format!(
        r##"<defs>{}<clipPath id="k"><use href="#a379"/></clipPath>{}</defs>{}<use href="#c379"/>{}"##,
        chain("a", 380, ""),
        chain("c", 380, r##"clip-path="url(#k)""##),
        "<g opacity=\"0.9\">".repeat(250),
        "</g>".repeat(250)
    ));
    let xml = usvg::roxmltree::Document::parse(&d).unwrap();
    let deep = copies(&xml).unwrap().depth;
    assert!(deep > 1000, "{deep}");
    let ordinary = illustrator_export(8, 300, 4);
    let xml = usvg::roxmltree::Document::parse(&ordinary).unwrap();
    let shallow = copies(&xml).unwrap().depth;
    assert!(shallow < WEB_MAX_BUILD_DEPTH / 4, "{shallow}");
}

/// Review 9 M-11: a Mermaid-style flowchart, 2,000 edges with an arrow at the end: usvg draws one
/// end marker per edge, not one per byte of path data.
#[test]
fn mermaid_flowchart_is_accepted() {
    let mut css = String::new();
    for cls in ["label", "node rect", "node circle", "edgePath .path", "edgeLabel rect", "cluster rect", "flowchart-link", "arrowheadPath"] {
        for v in 0..8 {
            css.push_str(&format!("#mermaid-1 .{cls}{}{{fill:#{v}{v}{v};stroke:#333}}", if v > 0 { " " } else { "" }));
        }
    }
    let edge = |i: usize| {
        let mut d = format!("M{}.25,{}.5", i % 500, i / 4 % 500);
        for k in 0..8 {
            d.push_str(&format!("c{}.5,-{}.25 {}.75,{}.5 -{}.25,{}.75", k % 9, (k + i) % 9, (k * 3) % 9, k % 7, (i + 2) % 9, k % 5));
        }
        format!(r##"<path d="{d}z" id="L_{i}" class="edge-thickness-normal flowchart-link" style="" marker-end="url(#mermaid-1_pointEnd)"/>"##)
    };
    let edges: String = (0..2000).map(edge).collect();
    let nodes: String = (0..2000)
        .map(|i| format!(r#"<g class="node default" transform="translate({},{})"><rect x="-20" y="-10" width="40" height="20"/><g class="label"><text><tspan>N{i}</tspan></text></g></g>"#, i % 40 * 50, i / 40 * 50))
        .collect();
    let d = format!(
        r##"<svg id="mermaid-1" width="100%" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2000 2000"><style>{css}</style><g><marker id="mermaid-1_pointEnd" class="marker" viewBox="0 0 10 10" refX="5" refY="5" markerUnits="userSpaceOnUse" markerWidth="8" markerHeight="8" orient="auto"><path d="M 0 0 L 10 5 L 0 10 z" style="stroke-width: 1; stroke-dasharray: 1, 0;"/></marker><g class="root"><g class="edgePaths">{edges}</g><g class="nodes">{nodes}</g></g></g></svg>"##
    );
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Review 9 M-12: a hyperlink in a pattern, and a `<use>` of a mask in a pattern, build nothing
/// in usvg: no loop.
#[test]
fn links_that_build_nothing_are_no_loop() {
    let d = pic(&format!(
        r##"<defs><pattern id="p" {PAT}><a href="#r"><rect width="5" height="5"/></a></pattern></defs><g fill="url(#p)"><rect id="r" width="50" height="50"/></g>"##
    ));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "a: {:?}", r.err());
    let d = pic(&format!(
        r##"<defs><pattern id="p" {PAT}><use href="#m"/><mask id="m"><rect width="5" height="5" fill="url(#p)"/></mask></pattern></defs><rect width="50" height="50" fill="url(#p)"/>"##
    ));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "use of a mask: {:?}", r.err());
}

/// Review 10 C-9: resvg makes a raster of a filter's whole region for every step, and doesn't clip
/// the region: a flood over 100,000 units drawn at any size would take gigabytes.
#[test]
fn filter_regions_draw_within_the_budget() {
    let d = pic(
        r##"<filter id="f" filterUnits="userSpaceOnUse" x="-50000" y="-50000" width="100000" height="100000"><feFlood flood-color="red"/></filter><rect width="50" height="50" filter="url(#f)"/>"##,
    );
    let svg = parse(d.as_bytes()).unwrap();
    for side in [64u32, 2048] {
        let k = side as f32 / 100.0;
        assert!(svg.draw_bytes(side, side, [k, 0.0, 0.0, k, 0.0, 0.0]) > MAX_DRAW_BYTES);
        let _ = svg.render(side, side, [k, 0.0, 0.0, k, 0.0, 0.0]);
    }
    let _ = fallback_png(d.as_bytes());
    // Many steps over an ordinary region.
    let floods = "<feFlood flood-color=\"red\"/>".repeat(400);
    let d = pic(&format!(r##"<filter id="f">{floods}</filter><rect width="100" height="100" filter="url(#f)"/>"##));
    let svg = parse(d.as_bytes()).unwrap();
    assert!(svg.render(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]).is_some());
}

/// Review 10 C-10: a layer is the group's bounds clipped to five times the raster each way, so up
/// to 25 times the raster.
#[test]
fn wide_layers_draw_within_the_budget() {
    let d = pic(&format!(r#"{}<rect x="-1500" y="-1500" width="3000" height="3000"/>{}"#, "<g opacity=\"0.9\">".repeat(6), "</g>".repeat(6)));
    let svg = parse(d.as_bytes()).unwrap();
    assert!(svg.draw_bytes(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]) > 6 * 2048 * 2048 * 4);
    assert!(svg.render(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]).is_some());
    // The same as an inserted picture's fallback.
    let d = d.replace("width=\"100\" height=\"100\"", "width=\"1024\" height=\"1024\"");
    assert!(fallback_png(d.as_bytes()).is_ok());
}

/// Review 10 C-12: stroking a path takes memory for every segment, more for a long curve drawn
/// large: 250,000 stroked arcs are drawn small, or not at all, rather than abort.
#[test]
fn huge_stroked_paths_draw_within_the_budget() {
    let d = pic(&format!(r#"<path d="M0 0{}" stroke="black" fill="none"/>"#, "a50,50 0 1,1 1,0".repeat(250_000)));
    let svg = parse(d.as_bytes()).unwrap();
    assert!(svg.draw_bytes(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]) > MAX_DRAW_BYTES);
    let _ = svg.render(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]);
}

/// Review 10 M-15: pictures nested side by side don't add up their depth (only one is being read
/// at a time).
#[test]
fn sibling_nested_pictures_are_as_deep_as_the_deepest() {
    let inner = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">{}<rect width=\"5\" height=\"5\"/>{}</svg>",
        "<g>".repeat(50),
        "</g>".repeat(50)
    );
    let b64 = base64_encode(inner.as_bytes());
    let icons = format!(r#"<image href="data:image/svg+xml;base64,{b64}" width="10" height="10"/>"#).repeat(6);
    let d = pic(&icons);
    let budget = Budget::default();
    assert!(parse_nested(d.as_bytes(), 0, 0, &budget).is_ok());
    let deepest = budget.deepest.load(Ordering::Relaxed);
    assert!(deepest > 50 && deepest < WEB_MAX_BUILD_DEPTH, "{deepest}");
}

/// Review 10 M-16: a clip path's clip path's clip path each take a raster of the layer.
#[test]
fn clip_path_chains_are_counted_whole() {
    let one = pic(
        r##"<clipPath id="a"><rect width="90" height="90"/></clipPath><g clip-path="url(#a)" opacity="0.5"><rect width="100" height="100"/></g>"##,
    );
    let three = pic(
        r##"<clipPath id="c"><rect width="70" height="70"/></clipPath><clipPath id="b" clip-path="url(#c)"><rect width="80" height="80"/></clipPath><clipPath id="a" clip-path="url(#b)"><rect width="90" height="90"/></clipPath><g clip-path="url(#a)" opacity="0.5"><rect width="100" height="100"/></g>"##,
    );
    let at = |d: &str| parse(d.as_bytes()).unwrap().draw_bytes(1000, 1000, [10.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
    let layer = 1004u64 * 1004 * 4;
    assert!(at(&three) >= at(&one) + 2 * layer, "{} {}", at(&three), at(&one));
}

/// Review 10: ordinary pictures draw at full size (the budget only bites on hostile ones).
#[test]
fn ordinary_pictures_draw_at_full_size() {
    for d in [illustrator_export(40, 3000, 5), illustrator_export(8, 300, 4)] {
        let svg = parse(d.as_bytes()).unwrap();
        for side in [1024u32, 2048] {
            let k = side as f32 / 1000.0;
            let need = svg.draw_bytes(side, side, [k, 0.0, 0.0, k, 0.0, 0.0]);
            assert!(need <= MAX_DRAW_BYTES, "{need}");
        }
    }
    let blur = pic(r##"<filter id="b"><feGaussianBlur stdDeviation="2"/></filter><circle cx="50" cy="50" r="30" filter="url(#b)" opacity="0.5"/>"##);
    let svg = parse(blur.as_bytes()).unwrap();
    assert!(svg.draw_bytes(2048, 2048, [20.48, 0.0, 0.0, 20.48, 0.0, 0.0]) <= MAX_DRAW_BYTES);
    assert!(svg.pdf_vectors_fit(4.0));
}

/// Review 11 C-13: a raster side of one pixel can't shrink, so the estimate is for the size drawn:
/// a thin picture's PNG fallback, and a strip of a picture (the rows the window shows).
#[test]
fn thin_rasters_are_estimated_as_drawn() {
    let thin = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="1"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="-50000000" width="100000" height="100000000"><feFlood flood-color="red"/></filter><rect width="100000" height="1" filter="url(#f)"/></svg>"##;
    let _ = fallback_png(thin.as_bytes());
    let d = pic(
        r##"<filter id="f" filterUnits="userSpaceOnUse" x="-50000" y="-50000" width="100000" height="100000"><feFlood flood-color="red"/></filter><rect width="50" height="50" filter="url(#f)"/>"##,
    );
    let svg = parse(d.as_bytes()).unwrap();
    for rows in [1u32, 3] {
        let _ = svg.render(1024, rows, [10.24, 0.0, 0.0, 10.24, 0.0, 0.0]);
    }
}

/// Review 11 I-7: tiny-skia measures the curves themselves; the lines through their points can be
/// much longer, so a count past a million is still up to a million dashes, each with its caps.
#[test]
fn dashes_count_whatever_the_curve() {
    let cur = format!("M500 500{}", " c500 0 -500 0 0 0".repeat(900));
    let d = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="1000"><path d="{cur}" fill="none" stroke="black" stroke-width="1" stroke-linecap="round" stroke-linejoin="round" stroke-dasharray="0.3 0.3"/></svg>"##
    );
    let svg = parse(d.as_bytes()).unwrap();
    // About a million dashes, each with two round caps: past the budget, as tiny-skia's 1.1 GB.
    assert!(svg.draw_bytes(1024, 1024, [1.024, 0.0, 0.0, 1.024, 0.0, 0.0]) > MAX_DRAW_BYTES);
    let _ = svg.render(1024, 1024, [1.024, 0.0, 0.0, 1.024, 0.0, 0.0]);
}

/// Review 11 C-14: krilla-svg's raster scale for a filter is bounded from above: a rotated thin
/// layer, and a filter in a nested picture drawn 100 times larger, don't fit as vectors.
#[test]
fn pdf_filter_scale_is_bounded_from_above() {
    let rotated = pic(
        r##"<defs><filter id="o" filterUnits="userSpaceOnUse" x="0" y="50" width="100" height="0.02"><feOffset/></filter><filter id="i" filterUnits="userSpaceOnUse" x="-500" y="-500" width="1000" height="1000"><feFlood flood-color="red"/></filter></defs><g transform="rotate(90 50 50)"><g filter="url(#o)"><rect width="100" height="100" filter="url(#i)"/></g></g>"##,
    );
    assert!(!parse(rotated.as_bytes()).unwrap().pdf_vectors_fit(1.0));
    let inner = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><filter id="o" x="0" y="0" width="1" height="1"><feOffset/></filter><filter id="i" filterUnits="userSpaceOnUse" x="-500" y="-500" width="1000" height="1000"><feFlood flood-color="red"/></filter><g filter="url(#o)"><rect width="1" height="1" filter="url(#i)"/></g></svg>"##;
    let nested = pic(&format!(r#"<image width="100" height="100" href="data:image/svg+xml;base64,{}"/>"#, base64_encode(inner.as_bytes())));
    assert!(!parse(nested.as_bytes()).unwrap().pdf_vectors_fit(1.0));
    // An ordinary blur stays vectors.
    let blur = pic(r##"<filter id="b"><feGaussianBlur stdDeviation="2"/></filter><circle cx="50" cy="50" r="30" filter="url(#b)"/>"##);
    assert!(parse(blur.as_bytes()).unwrap().pdf_vectors_fit(3.0));
}

/// A PNG whose header says `w` × `h` (its pixel data cut short).
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    v.extend_from_slice(b"\0\0\0\x01IDAT\0\0\0\0\0");
    v
}

/// Review 11 C-15: a raster image in the picture is checked by its header before krilla decodes
/// it for a PDF: one past the budget, or unreadable, means no vectors.
#[test]
fn pdf_embedded_images_are_checked() {
    let with = |png: &[u8]| pic(&format!(r#"<image width="100" height="100" href="data:image/png;base64,{}"/>"#, base64_encode(png)));
    let big = parse(with(&png_header(30_000, 30_000)).as_bytes()).unwrap();
    assert!(!big.pdf_vectors_fit(1.0));
    assert_eq!(big.raster_images().len(), 1);
    let small = parse(with(&png_header(100, 100)).as_bytes()).unwrap();
    assert!(small.pdf_vectors_fit(1.0));
}

/// Review 12 C-16: a filter group stretched one way under a rotation: krilla's ratio follows
/// the group's own (untransformed) layer, here 0.1 by 1000.
#[test]
fn pdf_filter_scale_follows_the_own_layer() {
    for above in ["rotate(90 50 50)", "skewX(45)", "rotate(45 50 50)"] {
        let d = pic(&format!(
            r##"<defs><filter id="o" filterUnits="userSpaceOnUse" x="0" y="0" width="0.1" height="1000"><feOffset/></filter><filter id="i" filterUnits="userSpaceOnUse" x="0" y="0" width="0.5" height="5000"><feFlood flood-color="red"/></filter></defs><g transform="{above}"><g transform="scale(1000 0.1)" filter="url(#o)"><rect width="0.1" height="1000" filter="url(#i)"/></g></g>"##
        ));
        assert!(!parse(d.as_bytes()).unwrap().pdf_vectors_fit(0.75), "{above}");
    }
}

/// A `width` × `height` PNG of noise (it doesn't compress).
fn noise_png(width: u32, height: u32) -> Vec<u8> {
    let mut pm = resvg::tiny_skia::Pixmap::new(width, height).unwrap();
    let mut x = 0x2545_f491u32;
    for b in pm.data_mut().chunks_mut(4) {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        b.copy_from_slice(&[x as u8, (x >> 8) as u8, (x >> 16) as u8, 255]);
    }
    pm.encode_png().unwrap()
}

/// Review 12 C-17: usvg decodes a `data:` image once per `<use>` copy and keeps each: the image
/// data a picture may hold is capped, copies counted.
#[test]
fn image_data_copies_are_capped() {
    let png = base64_encode(&noise_png(400, 400));
    let uses = r##"<use href="#p"/>"##.repeat(5000);
    let d = pic(&format!(r#"<defs><image id="p" width="10" height="10" href="data:image/png;base64,{png}"/></defs>{uses}"#));
    let r = parse(d.as_bytes());
    assert!(matches!(&r, Err(Error::Parse(m)) if m.contains("picture data")), "{:?}", r.err());
    // An icon set reusing one small image a few hundred times opens.
    let icon = base64_encode(&noise_png(32, 32));
    let uses = r##"<use href="#p"/>"##.repeat(500);
    let d = pic(&format!(r#"<defs><image id="p" width="10" height="10" href="data:image/png;base64,{icon}"/></defs>{uses}"#));
    let r = parse(d.as_bytes());
    assert!(r.is_ok(), "{:?}", r.err());
}

/// Review 12 M-18: krilla embeds a JPEG without decoding it, so a big photo keeps a PDF vectors.
#[test]
fn pdf_jpegs_are_not_charged_as_decoded() {
    // A JPEG header (start, a baseline frame of 6000 × 8000, end).
    let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    jpg.extend_from_slice(&8000u16.to_be_bytes());
    jpg.extend_from_slice(&6000u16.to_be_bytes());
    jpg.extend_from_slice(&[0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xD9]);
    let d = pic(&format!(r#"<image width="100" height="100" href="data:image/jpeg;base64,{}"/>"#, base64_encode(&jpg)));
    let svg = parse(d.as_bytes()).unwrap();
    assert!(svg.pdf_vectors_fit(1.0));
    assert_eq!(svg.raster_images().first().map(|(k, _)| *k), Some("jpeg"));
}
