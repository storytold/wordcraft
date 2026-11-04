//! A small synthetic Hangul document built in code, for tests (here and in the engine): a HWPX
//! package with a section definition (A5 page), a run of bold text, a centred paragraph, Korean
//! text and a 2×2 table whose first row is one merged cell. Nothing here comes from a file
//! Hangul wrote.

use std::io::Write;

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
