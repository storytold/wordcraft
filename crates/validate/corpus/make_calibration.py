#!/usr/bin/env python3
"""Calibration fixtures for the wordcraft-validate PR 1 rules.
00-control.docx is a rich valid base (proven pattern from the smoke test).
Every other file mutates exactly one thing; the filename says which rule it exercises.
Output: calibration/ directory beside this script.
"""
import zipfile, os, base64, warnings
warnings.filterwarnings("ignore")

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "calibration")
os.makedirs(OUT, exist_ok=True)
W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
R = 'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
EXTRA_NS = ('xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
            'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
            'xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"')
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==")
JPEG = bytes([0xFF, 0xD8, 0xFF, 0xE0]) + b"\x00" * 32

CT = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>
<Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>
<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"""

DOC_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"""

BODY = """<w:p><w:r><w:t xml:space="preserve">The quarterly report is </w:t></w:r><w:ins w:id="101" w:author="Ember Test" w:date="2026-11-06T12:00:00Z"><w:r><w:t xml:space="preserve">final </w:t></w:r></w:ins><w:del w:id="102" w:author="Ember Test" w:date="2026-11-06T12:00:00Z"><w:r><w:delText xml:space="preserve">draft </w:delText></w:r></w:del><w:r><w:t>ready for review.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="MyStyle"/></w:pPr><w:r><w:t xml:space="preserve">Revenue grew, and </w:t></w:r><w:commentRangeStart w:id="0"/><w:r><w:t>this clause</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:r><w:t xml:space="preserve"> needs a look</w:t></w:r><w:r><w:footnoteReference w:id="2"/></w:r><w:r><w:t>.</w:t></w:r></w:p>
<w:p><w:bookmarkStart w:id="7" w:name="bmA"/><w:r><w:t>Bookmarked text.</w:t></w:r><w:bookmarkEnd w:id="7"/></w:p>
<w:p><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Picture 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="Picture 1"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId4"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"""

DOC = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {W} {R} {EXTRA_NS}><w:body>
{BODY}
<w:sectPr><w:headerReference w:type="default" r:id="rId3"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
</w:body></w:document>"""

COMMENTS = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:comments {W}>
<w:comment w:id="0" w:author="Reviewer One" w:date="2026-11-06T12:05:00Z"><w:p><w:r><w:t>Check this clause.</w:t></w:r></w:p></w:comment>
</w:comments>"""

FOOTNOTES = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:footnotes {W}>
<w:footnote w:type="separator" w:id="0"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
<w:footnote w:type="continuationSeparator" w:id="1"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>
<w:footnote w:id="2"><w:p><w:r><w:t>Footnote text.</w:t></w:r></w:p></w:footnote>
</w:footnotes>"""

HEADER = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr {W}><w:p><w:r><w:t>TEST HEADER</w:t></w:r></w:p></w:hdr>"""

STYLES = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles {W}>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="MyStyle"><w:name w:val="My Style"/></w:style>
</w:styles>"""

def base_parts():
    """Build and return the dictionary of base package parts representing a valid document."""
    return {
        "[Content_Types].xml": CT,
        "_rels/.rels": ROOT_RELS,
        "word/document.xml": DOC,
        "word/_rels/document.xml.rels": DOC_RELS,
        "word/comments.xml": COMMENTS,
        "word/footnotes.xml": FOOTNOTES,
        "word/header1.xml": HEADER,
        "word/styles.xml": STYLES,
        "word/media/image1.png": PNG,
    }

def _info(name):
    # A fixed entry timestamp keeps the archive byte-identical between runs.
    zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    zi.compress_type = zipfile.ZIP_DEFLATED
    return zi

def write(name, parts, raw=None):
    """Write package parts as a ZIP archive or raw bytes to the output calibration directory."""
    path = os.path.join(OUT, name)
    if raw is not None:
        with open(path, "wb") as f:
            f.write(raw)
    else:
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
            for k, v in parts.items():
                z.writestr(_info(k), v)
    print("wrote", name)

# control
write("00-control.docx", base_parts())
# batch 1
write("01-not-zip.docx", None, raw=b"This is plain text, not a zip archive at all.")
with zipfile.ZipFile(os.path.join(OUT, "02-dup-entry.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    for k, v in base_parts().items():
        z.writestr(_info(k), v)
    z.writestr(_info("word/document.xml"), DOC)
print("wrote 02-dup-entry.docx")
p = base_parts(); p["word/Document.xml"] = DOC; write("03-case-collision.docx", p)
p = base_parts(); del p["[Content_Types].xml"]; write("04-no-content-types.docx", p)
p = base_parts(); p["_rels/.rels"] = ROOT_RELS.replace(' Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"', ' Type="http://example.invalid/core-properties"'); write("05-no-main.docx", p)
p = base_parts(); p["word/blob.xyzq"] = b"\x01\x02\x03"; write("06-ct-undeclared.docx", p)
p = base_parts(); p["[Content_Types].xml"] = CT.replace("wordprocessingml.document.main+xml", "xml"); write("07-ct-main-type.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("</w:ins>", ""); write("08-xml-malformed-main.docx", p)
p = base_parts(); p["word/unused.xml"] = "<w:unused><w:open></w:unused>"; write("09-xml-malformed-unused.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("</w:body>", "<w:p><w:r><w:t>x</w:t></w:r><w15:foo/></w:p></w:body>"); write("10-xml-undeclared-prefix.docx", p)
p = base_parts(); p["word/_rels/document.xml.rels"] = DOC_RELS.replace('Id="rId5"', 'Id="rId3"'); write("11-rel-dup-id.docx", p)
p = base_parts(); p["word/_rels/document.xml.rels"] = DOC_RELS.replace('Target="media/image1.png"', 'Target="media/missing.png"'); write("12-rel-dangling.docx", p)
p = base_parts(); p["word/_rels/document.xml.rels"] = DOC_RELS.replace('Target="header1.xml"', 'Target="media/image1.png"'); write("13-rel-typed.docx", p)
# batch 2
p = base_parts(); p["word/comments.xml"] = COMMENTS.replace("</w:comments>", '<w:comment w:id="0" w:author="Second" w:date="2026-11-06T12:06:00Z"><w:p><w:r><w:t>Duplicate id.</w:t></w:r></w:p></w:comment></w:comments>'); write("14-comment-dup.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace('<w:commentReference w:id="0"/>', '<w:commentReference w:id="9"/>'); write("15-comment-ref.docx", p)
drawing2 = '<w:p><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Picture 2"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="Picture 2"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId4"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>'
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", drawing2 + "<w:sectPr>"); write("16-docpr-dup.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", '<w:p><w:bookmarkStart w:id="7" w:name="bmB"/><w:r><w:t>Second bookmark, same id.</w:t></w:r><w:bookmarkEnd w:id="7"/></w:p><w:sectPr>'); write("17-bookmark-dup.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", '<w:p><w:ins w:id="101" w:author="Ember Test" w:date="2026-11-06T12:01:00Z"><w:r><w:t>second insert, same id</w:t></w:r></w:ins></w:p><w:sectPr>'); write("18-revision-dup.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace('<w:footnoteReference w:id="2"/>', '<w:footnoteReference w:id="9"/>'); write("19-note-ref.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace('r:id="rId3"', 'r:id="rId99"'); write("20-header-ref.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace('w:val="MyStyle"', 'w:val="NoSuchStyle"'); write("21-style-ref.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", '<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r></w:p><w:sectPr>'); write("22-fld-unbalanced.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="1000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="1000" w:type="dxa"/></w:tcPr></w:tc></w:tr></w:tbl><w:sectPr>'); write("23-tc-no-p.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", '<w:p><w:del w:id="301" w:author="Ember Test" w:date="2026-11-06T12:02:00Z"><w:r><w:t>wrong element</w:t></w:r></w:del></w:p><w:sectPr>'); write("24-deltext.docx", p)
p = base_parts(); p["word/document.xml"] = DOC.replace("<w:sectPr>", '<w:p><w:moveFromRangeStart w:id="5" w:name="mv1" w:author="Ember Test" w:date="2026-11-06T12:03:00Z"/><w:r><w:t>moved text</w:t></w:r></w:p><w:sectPr>'); write("25-move-unpaired.docx", p)
p = base_parts(); p["word/media/image1.png"] = JPEG; write("26-media-magic.docx", p)
print("done:", len(os.listdir(OUT)), "files")
