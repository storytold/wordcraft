#!/usr/bin/env python3
"""Write original OOXML leading controls to a new, caller-selected directory.

Python standard library only. No font files, native exports or machine paths are read.
Generated documents request fonts by name; installed face selection must be checked.
"""

import argparse
import hashlib
import json
from pathlib import Path
from xml.etree import ElementTree
from xml.sax.saxutils import escape
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PACKAGE_REL = "http://schemas.openxmlformats.org/package/2006/relationships"
LATIN = ("Alpha control one", "Alpha control two")
CHINESE = ("中文基线一", "中文基线二")
KOREAN = ("한글 기준선 하나", "한글 기준선 둘")
AUTO = "auto"
EXACT = "exact"
ZIP_DATE = (2026, 10, 17, 0, 0, 0)


def case(key, font, size, rule=AUTO, texts=LATIN, language="en-US"):
    return dict(id=key, font=font, size=size, rule=rule, texts=texts, language=language)


def groups():
    v3 = [case("D1", "Arial", 18), case("D2", "Arial Unicode MS", 18)]
    for key, font, size in [
        ("D3", "Songti SC", 18), ("D4", "Songti SC", 36),
        ("D5", "Apple SD Gothic Neo", 18), ("D6", "Apple SD Gothic Neo", 36),
    ]:
        v3.append(case(key, font, size))
    v4 = [case("E1", "Arial", 18), case("E2", "Brush Script MT", 18),
          case("E3", "Brush Script MT", 72), case("E4", "Hiragino Sans GB", 36)]
    cjk = []
    for font, size, texts, language in [
        ("Songti SC", 18, CHINESE, "zh-CN"),
        ("Songti SC", 36, CHINESE, "zh-CN"),
        ("Apple SD Gothic Neo", 18, KOREAN, "ko-KR"),
        ("Apple SD Gothic Neo", 36, KOREAN, "ko-KR"),
        ("Arial", 18, LATIN, "en-US"),
    ]:
        for rule in (AUTO, EXACT):
            cjk.append(case(f"Z{len(cjk) + 1}", font, size, rule, texts, language))
    return {"v3": v3, "v4": v4, "cjk": cjk}


def formatting(font, size, language):
    slots = " ".join(f'w:{slot}="{escape(font)}"' for slot in ("ascii", "hAnsi", "eastAsia", "cs"))
    italic = '<w:i/><w:iCs/>' if font == "Brush Script MT" else ""
    return (f'{italic}<w:rFonts {slots}/><w:sz w:val="{size * 2}"/>'
            f'<w:szCs w:val="{size * 2}"/><w:rtl w:val="0"/><w:cs w:val="0"/>'
            f'<w:lang w:val="{language}" w:eastAsia="{language}"/>')


def paragraph(texts, font="Arial", size=8, rule=AUTO, language="en-US"):
    fmt = formatting(font, size, language)
    line = size * 20 if rule == EXACT else 240
    text = '<w:br/>'.join(f'<w:t>{escape(text)}</w:t>' for text in texts)
    return (f'<w:p><w:pPr><w:snapToGrid w:val="0"/><w:spacing w:before="0" w:after="0" '
            f'w:line="{line}" w:lineRule="{rule}"/><w:rPr>{fmt}</w:rPr>'
            f'</w:pPr><w:r><w:rPr>{fmt}</w:rPr>{text}</w:r></w:p>')


def document_parts(cases):
    body = paragraph(("Font leading control",), size=11)
    for row in cases:
        label = f'{row["id"]} {row["font"]} {row["size"]}pt {row["rule"]}'
        body += paragraph((label,))
        body += paragraph(row["texts"], row["font"], row["size"], row["rule"], row["language"])
    body += ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>'
             '<w:pgMar w:top="600" w:right="600" w:bottom="600" w:left="600" w:header="0" w:footer="0" w:gutter="0"/></w:sectPr>')
    content = ('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
               '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
               '<Default Extension="xml" ContentType="application/xml"/>')
    for name, kind in (("document", "document.main"), ("settings", "settings")):
        content += (f'<Override PartName="/word/{name}.xml" ContentType="application/'
                    f'vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"/>')
    return {
        "[Content_Types].xml": content + "</Types>",
        "_rels/.rels": (f'<Relationships xmlns="{PACKAGE_REL}"><Relationship Id="rId1" '
                        f'Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>'),
        "word/_rels/document.xml.rels": (f'<Relationships xmlns="{PACKAGE_REL}"><Relationship Id="rId1" '
                                        f'Type="{REL}/settings" Target="settings.xml"/></Relationships>'),
        "word/document.xml": f'<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>',
        "word/settings.xml": (f'<w:settings xmlns:w="{W}"><w:compat><w:compatSetting '
                              'w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" '
                              'w:val="15"/></w:compat></w:settings>'),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the source checkout")
    args = parser.parse_args()
    source = Path(__file__).resolve().parents[3]
    output = args.output.resolve()
    if output == source or source in output.parents:
        parser.error("output must be outside the source checkout")
    output.mkdir(parents=True, exist_ok=False)
    manifest = {"scope": "New synthetic controls, not byte replicas of the measured private inputs. "
                         "No native baseline oracle; verify selected faces before comparison. E4 remains excluded.",
                "documents": {}}
    for name, cases in groups().items():
        path = output / f"leading-{name}.docx"
        with ZipFile(path, "x") as archive:
            for part, xml in document_parts(cases).items():
                ElementTree.fromstring(xml)
                info = ZipInfo(part, ZIP_DATE)
                info.compress_type = ZIP_DEFLATED
                archive.writestr(info, xml)
        manifest["documents"][path.name] = {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "cases": cases,
        }
    (output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
