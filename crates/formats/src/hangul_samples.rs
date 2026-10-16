//! Small synthetic Hangul documents built in code, for tests (here and in the engine): a HWP 5.0
//! compound file and a HWPX package, each with a section definition (A5 page), a run of bold
//! text, a centred paragraph, Korean text and a 2×2 table whose first row is one merged cell.
//! Nothing here comes from a file Hangul wrote.

use std::io::Write;

/// One HWP record: header (tag, level, size; extended size past 4094 bytes) and payload.
fn rec(tag: u16, level: u16, data: &[u8]) -> Vec<u8> {
    let size = data.len() as u32;
    let mut out = Vec::new();
    let small = size.min(0xFFF);
    out.extend((u32::from(tag) | (u32::from(level) << 10) | (small << 20)).to_le_bytes());
    if small == 0xFFF {
        out.extend(size.to_le_bytes());
    }
    out.extend(data);
    out
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn hwp_string(s: &str) -> Vec<u8> {
    let mut v = (s.encode_utf16().count() as u16).to_le_bytes().to_vec();
    v.extend(utf16(s));
    v
}

/// An extended control character (8 UTF-16 units) for control `id`.
fn ctrl_char(code: u16, id: &[u8; 4]) -> Vec<u8> {
    let mut v = code.to_le_bytes().to_vec();
    v.extend(u32::from_be_bytes(*id).to_le_bytes());
    v.extend([0u8; 8]);
    v.extend(code.to_le_bytes());
    v
}

fn char_shape(bold: bool) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend([0u8; 14]); // font ids: Hangul 0, Latin 0, …
    v.extend([100u8; 7]); // ratios
    v.extend([0u8; 7]); // spacing
    v.extend([100u8; 7]); // relative sizes
    v.extend([0u8; 7]); // offsets
    v.extend(1200i32.to_le_bytes()); // 12 pt
    v.extend((if bold { 2u32 } else { 0 }).to_le_bytes());
    v.extend([0u8; 2]);
    v.extend(0x0000_00FFu32.to_le_bytes()); // red text (0x00BBGGRR)
    v.extend(0u32.to_le_bytes());
    v.extend(0xFFFF_FFFFu32.to_le_bytes()); // no shading
    v.extend(0u32.to_le_bytes());
    v
}

fn para_shape(align: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend((align << 2).to_le_bytes()); // percent line spacing
    for x in [0i32, 0, 0, 0, 1000, 160] {
        v.extend(x.to_le_bytes()); // margins, indent, before, after (5 pt at twice HWPUNIT), 160 %
    }
    v.extend([0u8; 6]); // tab def, numbering, border fill
    v.extend([0u8; 8]);
    v
}

fn doc_info() -> Vec<u8> {
    let mut v = Vec::new();
    let mut ids = 0u32.to_le_bytes().to_vec(); // pictures
    ids.extend(1u32.to_le_bytes()); // Hangul faces
    ids.extend(1u32.to_le_bytes()); // Latin faces
    ids.extend([0u8; 4 * 9]);
    v.extend(rec(0x11, 0, &ids));
    let mut face = vec![0u8];
    face.extend(hwp_string("함초롬바탕"));
    v.extend(rec(0x13, 1, &face));
    let mut face = vec![0u8];
    face.extend(hwp_string("Times New Roman"));
    v.extend(rec(0x13, 1, &face));
    // Border fill 1: solid 0.12 mm black on all four sides.
    let mut bf = 0u16.to_le_bytes().to_vec();
    for _ in 0..4 {
        bf.extend([1u8, 1]);
        bf.extend(0u32.to_le_bytes());
    }
    bf.extend([0u8; 6]);
    bf.extend(0u32.to_le_bytes());
    bf.extend(0u32.to_le_bytes());
    v.extend(rec(0x14, 1, &bf));
    v.extend(rec(0x15, 1, &char_shape(false)));
    v.extend(rec(0x15, 1, &char_shape(true)));
    v.extend(rec(0x19, 1, &para_shape(0)));
    v.extend(rec(0x19, 1, &para_shape(3)));
    v
}

fn para_header(level: u16, shape: u16) -> Vec<u8> {
    let mut d = 0u32.to_le_bytes().to_vec();
    d.extend(0u32.to_le_bytes());
    d.extend(shape.to_le_bytes());
    d.extend([0u8, 0]);
    rec(0x42, level, &d)
}

/// A paragraph of plain text at `level`.
fn para(level: u16, shape: u16, text: &str) -> Vec<u8> {
    let mut v = para_header(level, shape);
    let mut t = utf16(text);
    t.extend(13u16.to_le_bytes());
    v.extend(rec(0x43, level + 1, &t));
    v
}

fn cell(level: u16, col: u16, row: u16, cols: u16, text: &str) -> Vec<u8> {
    let mut d = 1u16.to_le_bytes().to_vec();
    d.extend((1u32 << 21).to_le_bytes()); // centred vertically
    d.extend(0u16.to_le_bytes());
    for x in [col, row, cols, 1] {
        d.extend(x.to_le_bytes());
    }
    d.extend((7200u32 * u32::from(cols)).to_le_bytes()); // 72 pt per column
    d.extend(2000u32.to_le_bytes());
    d.extend([0u8; 8]);
    d.extend(1u16.to_le_bytes()); // border fill 1
    let mut v = rec(0x48, level, &d);
    v.extend(para(level, 0, text));
    v
}

fn section() -> Vec<u8> {
    let mut v = Vec::new();
    // Paragraph 1: the section definition (A5 page), then "Hello " and a bold "bold".
    v.extend(para_header(0, 0));
    let mut t = ctrl_char(2, b"secd");
    t.extend(utf16("Hello bold"));
    t.extend(13u16.to_le_bytes());
    v.extend(rec(0x43, 1, &t));
    let mut cs = Vec::new();
    for (pos, id) in [(0u32, 0u32), (14, 1)] {
        cs.extend(pos.to_le_bytes());
        cs.extend(id.to_le_bytes());
    }
    v.extend(rec(0x44, 1, &cs));
    let mut secd = u32::from_be_bytes(*b"secd").to_le_bytes().to_vec();
    secd.extend([0u8; 16]);
    v.extend(rec(0x47, 1, &secd));
    let mut pd = Vec::new();
    for x in [41954u32, 59528, 5669, 5669, 4252, 4252, 2835, 2835, 0, 0] {
        pd.extend(x.to_le_bytes());
    }
    v.extend(rec(0x49, 2, &pd));
    // Paragraph 2: centred Korean text.
    v.extend(para(0, 1, "가운데 한글"));
    // Paragraph 3: a 2×2 table, its first row merged.
    v.extend(para_header(0, 0));
    let mut t = ctrl_char(11, b"tbl ");
    t.extend(13u16.to_le_bytes());
    v.extend(rec(0x43, 1, &t));
    let mut th = u32::from_be_bytes(*b"tbl ").to_le_bytes().to_vec();
    th.extend([0u8; 40]);
    v.extend(rec(0x47, 1, &th));
    let mut tb = 0u32.to_le_bytes().to_vec();
    tb.extend(2u16.to_le_bytes());
    tb.extend(2u16.to_le_bytes());
    tb.extend(0i16.to_le_bytes());
    for m in [510i16, 510, 141, 141] {
        tb.extend(m.to_le_bytes());
    }
    tb.extend(1u16.to_le_bytes());
    tb.extend(2u16.to_le_bytes());
    tb.extend(1u16.to_le_bytes());
    v.extend(rec(0x4D, 2, &tb));
    v.extend(cell(2, 0, 0, 2, "merged"));
    v.extend(cell(2, 0, 1, 1, "A2"));
    v.extend(cell(2, 1, 1, 1, "B2"));
    v.extend(para(0, 0, "After"));
    v
}

/// A HWP 5.0 file; `compressed` deflates `DocInfo` and the section as Hangul does by default.
pub fn hwp(compressed: bool) -> Vec<u8> {
    let mut header = b"HWP Document File".to_vec();
    header.resize(32, 0);
    header.extend(0x0500_0000u32.to_le_bytes());
    header.extend(u32::from(compressed).to_le_bytes());
    header.resize(256, 0);
    let pack = |d: Vec<u8>| if compressed { miniz_oxide::deflate::compress_to_vec(&d, 6) } else { d };
    let mut comp = match cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let _ = comp.create_storage("/BodyText");
    for (name, data) in [("/FileHeader", header), ("/DocInfo", pack(doc_info())), ("/BodyText/Section0", pack(section()))] {
        if let Ok(mut s) = comp.create_stream(name) {
            let _ = s.write_all(&data);
        }
    }
    let _ = comp.flush();
    comp.into_inner().into_inner()
}

const HEADER_XML: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hh:refList>
<hh:fontfaces><hh:fontface lang="HANGUL"><hh:font id="0" face="함초롬바탕"/></hh:fontface><hh:fontface lang="LATIN"><hh:font id="0" face="Times New Roman"/></hh:fontface></hh:fontfaces>
<hh:borderFills><hh:borderFill id="1"><hh:leftBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:rightBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:topBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:bottomBorder type="SOLID" width="0.12 mm" color="#000000"/><hc:fillBrush><hc:winBrush faceColor="#FFFF00"/></hc:fillBrush></hh:borderFill></hh:borderFills>
<hh:charProperties><hh:charPr id="0" height="1200" textColor="#FF0000"><hh:fontRef hangul="0" latin="0"/></hh:charPr><hh:charPr id="1" height="1200" textColor="#FF0000"><hh:fontRef hangul="0" latin="0"/><hh:bold/></hh:charPr></hh:charProperties>
<hh:tabProperties><hh:tabPr id="1"><hh:tabItem pos="20000" type="RIGHT" leader="DOT"/></hh:tabPr></hh:tabProperties>
<hh:paraProperties><hh:paraPr id="0"><hh:align horizontal="JUSTIFY"/><hp:switch><hp:case hp:required-namespace="http://www.hancom.co.kr/hwpml/2016/HwpUnitChar"><hh:margin><hc:intent value="0"/><hc:left value="0"/><hc:right value="0"/><hc:prev value="0"/><hc:next value="500"/></hh:margin><hh:lineSpacing type="PERCENT" value="160"/></hp:case><hp:default><hh:margin><hc:intent value="0"/><hc:left value="0"/><hc:right value="0"/><hc:prev value="0"/><hc:next value="1000"/></hh:margin><hh:lineSpacing type="PERCENT" value="160"/></hp:default></hp:switch></hh:paraPr>
<hh:paraPr id="1" tabPrIDRef="1"><hh:align horizontal="CENTER"/></hh:paraPr></hh:paraProperties>
</hh:refList></hh:head>"##;

const SECTION_XML: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core">
<hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr landscape="WIDELY" width="41954" height="59528"><hp:margin header="2835" footer="2835" gutter="0" left="5669" right="5669" top="4252" bottom="4252"/></hp:pagePr></hp:secPr><hp:t>Hello </hp:t></hp:run><hp:run charPrIDRef="1"><hp:t>bold</hp:t></hp:run></hp:p>
<hp:p paraPrIDRef="1"><hp:run charPrIDRef="0"><hp:t>가운데<hp:tab/>한글</hp:t></hp:run></hp:p>
<hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:tbl rowCnt="2" colCnt="2" borderFillIDRef="1"><hp:sz width="14400" height="4000"/><hp:pos treatAsChar="1"/><hp:inMargin left="510" right="510" top="141" bottom="141"/>
<hp:tr><hp:tc borderFillIDRef="1"><hp:subList vertAlign="CENTER"><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>merged</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="2" rowSpan="1"/><hp:cellSz width="14400" height="2000"/></hp:tc></hp:tr>
<hp:tr><hp:tc borderFillIDRef="1"><hp:subList><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>A2</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="7200" height="2000"/></hp:tc><hp:tc borderFillIDRef="1"><hp:subList><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>B2</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="1" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="7200" height="2000"/></hp:tc></hp:tr>
</hp:tbl><hp:t/></hp:run></hp:p>
<hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>After</hp:t></hp:run></hp:p>
</hs:sec>"##;

/// A HWPX package.
pub fn hwpx() -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let parts: [(&str, &str); 5] = [
        ("mimetype", "application/hwp+zip"),
        ("version.xml", r#"<?xml version="1.0"?><hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" xmlVersion="1.4"/>"#),
        (
            "Contents/content.hpf",
            r#"<?xml version="1.0"?><opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="header"/><opf:itemref idref="section0"/></opf:spine></opf:package>"#,
        ),
        ("Contents/header.xml", HEADER_XML),
        ("Contents/section0.xml", SECTION_XML),
    ];
    for (name, text) in parts {
        if z.start_file(name, stored).is_ok() {
            let _ = z.write_all(text.as_bytes());
        }
    }
    z.finish().map(|c| c.into_inner()).unwrap_or_default()
}
