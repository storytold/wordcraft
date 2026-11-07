#!/usr/bin/env python3
"""Seeded stress-file generator for wordcraft-validate (storytold/wordcraft #5848).

Builds .docx files that are VALID BY CONSTRUCTION but maximal: every Word
feature family at scale, in combination. Two consumers:
  loop 1 - the structural validator must PASS every file (false-positive hunt)
  loop 2 - the extreme files go to Word for the web; if Word flags a
           by-construction-legal file, we dissect why.

Pure Python 3 stdlib (zipfile + string templates), in the style of
~/workspace/word-web-oracle/make_calibration.py. Deterministic: a fixed seed
reproduces the same bytes (zip entry timestamps are pinned to 1980-01-01).

Usage:
  python3 gen_stress.py --all                 # build the whole corpus
  python3 gen_stress.py --preset monster --seed 7 -o /tmp/m.docx
  python3 gen_stress.py --paragraphs 500 --comment-threads 40 -o /tmp/x.docx
"""
import argparse, binascii, io, os, random, struct, sys, zipfile, zlib
import xml.etree.ElementTree as ET
from datetime import datetime, timedelta

XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'

NS = {
    "w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "r": "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "wp": "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    "wp14": "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
    "a": "http://schemas.openxmlformats.org/drawingml/2006/main",
    "pic": "http://schemas.openxmlformats.org/drawingml/2006/picture",
    "c": "http://schemas.openxmlformats.org/drawingml/2006/chart",
    "m": "http://schemas.openxmlformats.org/officeDocument/2006/math",
    "v": "urn:schemas-microsoft-com:vml",
    "o": "urn:schemas-microsoft-com:office:office",
    "w14": "http://schemas.microsoft.com/office/word/2010/wordml",
    "w15": "http://schemas.microsoft.com/office/word/2012/wordml",
    "w16cid": "http://schemas.microsoft.com/office/word/2016/wordml/cid",
    "wps": "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    "mc": "http://schemas.openxmlformats.org/markup-compatibility/2006",
    "ds": "http://schemas.openxmlformats.org/officeDocument/2006/customXml",
}
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PKG_REL = "http://schemas.openxmlformats.org/package/2006/relationships"
CT_NS = "http://schemas.openxmlformats.org/package/2006/content-types"

WORDS = ("the of and a an in is you that it he was for on are as with his they "
         "at be this have from or one had by word but not what all were we when your can "
         "said there use each which she do how their if will up other about out many then "
         "them these so some her would make like him into time has look two more write go "
         "see number no way could people my than first water been call who oil its now "
         "find long down day did get come made may part over under through much before "
         "line right too mean old any same tell follow around form three small set put "
         "end does another well large must big even such because turn here why ask went "
         "men read need land different home us move try kind hand picture again change "
         "off play spell air away animal house point page letter mother answer found "
         "study still learn should world high every near add food between own below "
         "country plant last school father keep tree never start city earth eye light "
         "thought head under story saw left few while along might close something seem "
         "next hard open example begin life always those both paper together got group "
         "often run report revenue quarter growth margin review clause section audit "
         "café naïve résumé — “quoted”").split()

LATENT_NAMES = [
    "Normal", "heading 1", "heading 2", "heading 3", "heading 4", "heading 5",
    "heading 6", "heading 7", "heading 8", "heading 9", "index 1", "index 2",
    "toc 1", "toc 2", "toc 3", "caption", "Title", "Subtitle", "Quote",
    "Intense Quote", "List Paragraph", "TOC Heading", "Table Grid",
    "No Spacing", "Light Shading", "Light List", "Light Grid", "Medium Shading 1",
    "Medium List 1", "Medium Grid 1", "Dark List", "Colorful Shading",
    "Colorful List", "Colorful Grid", "Default Paragraph Font", "Normal Table",
    "No List", "Placeholder Text", "Revision", "Strong", "Emphasis",
    "Intense Emphasis", "Book Title", "Bibliography", "Hyperlink",
]


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def esca(s):
    return esc(s).replace('"', "&quot;")


def make_png(rng, width, height):
    """A truecolour PNG of seeded noise (incompressible, like real photos)."""
    raw = bytearray()
    for _ in range(height):
        raw.append(0)  # filter: none
        raw += rng.randbytes(width * 3)
    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", binascii.crc32(tag + data) & 0xFFFFFFFF))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 6)) + chunk(b"IEND", b""))


THEME = XML_DECL + """<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="50000"/><a:satMod val="300000"/></a:schemeClr></a:gs><a:gs pos="35000"><a:schemeClr val="phClr"><a:tint val="37000"/><a:satMod val="300000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:tint val="15000"/><a:satMod val="350000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="16200000" scaled="1"/></a:gradFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:shade val="51000"/><a:satMod val="130000"/></a:schemeClr></a:gs><a:gs pos="80000"><a:schemeClr val="phClr"><a:shade val="93000"/><a:satMod val="130000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="94000"/><a:satMod val="135000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="16200000" scaled="0"/></a:gradFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="12700" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="19050" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"""


def make_xlsx(cats, vals):
    """Minimal valid .xlsx (for the chart's embedded workbook part)."""
    ct = (XML_DECL + f'<Types xmlns="{CT_NS}">'
          '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
          '<Default Extension="xml" ContentType="application/xml"/>'
          '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
          '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
          '</Types>')
    rels = (XML_DECL + f'<Relationships xmlns="{PKG_REL}">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
            '</Relationships>')
    wb = (XML_DECL + '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
          'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
          '<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>')
    wb_rels = (XML_DECL + f'<Relationships xmlns="{PKG_REL}">'
               '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
               '</Relationships>')
    rows = ['<row r="1"><c r="A1" t="inlineStr"><is><t>Category</t></is></c>'
            '<c r="B1" t="inlineStr"><is><t>Series 1</t></is></c></row>']
    for i, (cname, v) in enumerate(zip(cats, vals), start=2):
        rows.append(f'<row r="{i}"><c r="A{i}" t="inlineStr"><is><t>{esc(cname)}</t></is></c>'
                    f'<c r="B{i}"><v>{v}</v></c></row>')
    sheet = (XML_DECL + '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
             '<sheetData>' + "".join(rows) + '</sheetData></worksheet>')
    import io
    bio = io.BytesIO()
    with zipfile.ZipFile(bio, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in (("[Content_Types].xml", ct), ("_rels/.rels", rels),
                           ("xl/workbook.xml", wb), ("xl/_rels/workbook.xml.rels", wb_rels),
                           ("xl/worksheets/sheet1.xml", sheet)):
            zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(zi, data)
    return bio.getvalue()


class Rels:
    """Relationship set for one part; ids are rId1.. in insertion order."""
    def __init__(self):
        self.items = []  # (id, type, target, external)

    def add(self, rtype, target, external=False):
        rid = f"rId{len(self.items) + 1}"
        self.items.append((rid, rtype, target, external))
        return rid

    def render(self):
        out = [XML_DECL, f'<Relationships xmlns="{PKG_REL}">']
        for rid, rtype, target, external in self.items:
            mode = ' TargetMode="External"' if external else ""
            out.append(f'<Relationship Id="{rid}" Type="{rtype}" Target="{esca(target)}"{mode}/>')
        out.append('</Relationships>')
        return "".join(out)


BASE_DATE = datetime(2026, 11, 6, 12, 0, 0)
CUSTOMXML_GUID = "{7E3A91C4-5B2D-4F18-9C6E-2D8A41B05F77}"
CUSTOMXML_SCHEMA_URI = "urn:wordcraft:stress:customXmlSchema"


class Ctx:
    """All mutable build state: seeded RNG, id spaces, part registry."""

    def __init__(self, knobs, seed):
        self.k = knobs
        self.rng = random.Random(seed)
        self.parts = {}            # partname -> (content_type_or_None_for_default, bytes)
        self.defaults = {"rels": "application/vnd.openxmlformats-package.relationships+xml",
                         "xml": "application/xml",
                         "png": "image/png"}
        self.overrides = {}        # partname -> content type
        self.doc_rels = Rels()
        self.root_rels = Rels()
        # id spaces
        self.docpr = 0             # wp:docPr / pic:cNvPr — document-wide
        self.bookmark_id = 0       # document-wide across story parts
        self.rev_id = 0            # ins/del/*Change — document-wide
        self.sdt_id = 1000
        self.anchor_height = 251659264
        self.date_tick = 0
        # plans filled by plan_* methods
        self.authors = [f"Author {i + 1}" for i in range(max(1, knobs["authors"]))]
        self.style_ids = []        # paragraph styles usable in body
        self.char_style_ids = []
        self.num_ids = []          # w:numId values referenced by paragraphs
        self.bookmark_names = []
        self.footnotes = []        # (id, text)
        self.endnotes = []
        self.comment_plan = []     # dicts: cid, author, paras, replies[]
        self.media = []            # (partname, rel_id, w, h)
        self.para_ids = {}         # partname -> set of w14:paraId values used
        # bisect3: resolved flavour sub-flags (None sub-knob follows flavour)
        def _sub(name):
            v = knobs.get(name)
            return knobs["flavour"] if v is None else v
        self.f_proof = _sub("proof_err")
        self.f_sdt = _sub("inline_sdt")
        self.f_smart = _sub("smart_tags")
        self.f_links = _sub("hyperlinks")
        # bisect3: rsid pool shared by settings.xml and in-document rsid
        # attributes; only built when rsid_attrs is on (RNG untouched else)
        self.rsid_pool = None
        if knobs.get("rsid_attrs"):
            pool, seen = [], set()
            while len(pool) < max(knobs["rsids"], 64):
                v = "%08X" % self.rng.randrange(0, 0xFFFFFFFF)
                if v not in seen:
                    seen.add(v)
                    pool.append(v)
            self.rsid_pool = pool

    # -- tiny helpers ------------------------------------------------------
    def next_docpr(self):
        self.docpr += 1
        return self.docpr

    def next_bookmark(self):
        self.bookmark_id += 1
        return self.bookmark_id

    def next_rev(self):
        self.rev_id += 1
        return self.rev_id

    def next_sdt(self):
        self.sdt_id += 1
        return self.sdt_id

    def date(self):
        self.date_tick += 1
        return (BASE_DATE + timedelta(minutes=self.date_tick)).strftime("%Y-%m-%dT%H:%M:%SZ")

    def author(self):
        return self.rng.choice(self.authors)

    def para_id(self, part):
        used = self.para_ids.setdefault(part, set())
        while True:
            v = self.rng.randrange(1, 0x7FFFFFFF)
            if v not in used:
                used.add(v)
                return "%08X" % v

    def sentence(self, n=None):
        n = n or self.rng.randint(6, 14)
        s = " ".join(self.rng.choice(WORDS) for _ in range(n))
        return s[0].upper() + s[1:] + "."

    def text(self, sentences=None):
        sentences = sentences or self.rng.randint(2, 4)
        return " ".join(self.sentence() for _ in range(sentences))

    def add_part(self, name, data, ctype=None, override=False):
        if isinstance(data, str):
            data = data.encode("utf-8")
        self.parts[name] = data
        if override and ctype:
            self.overrides[name] = ctype
        elif ctype:  # remember explicit type even when reached via Default
            self.overrides[name] = ctype
        return name


def rpr(ctx, rich=True):
    """A run-properties fragment in schema order (subset)."""
    if not rich:
        return ""
    rng = ctx.rng
    out = []
    if rng.random() < 0.3:
        out.append('<w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/>')
    if rng.random() < 0.3:
        out.append("<w:b/>")
    if rng.random() < 0.25:
        out.append("<w:i/>")
    if rng.random() < 0.15:
        out.append("<w:caps/>")
    if rng.random() < 0.1:
        out.append("<w:strike/>")
    if rng.random() < 0.2:
        out.append(f'<w:color w:val="{rng.choice(["404040", "1F4E79", "833C00", "375623"])}"/>')
    if rng.random() < 0.25:
        out.append(f'<w:sz w:val="{rng.choice([18, 20, 22, 24, 28])}"/>')
    if rng.random() < 0.1:
        out.append('<w:highlight w:val="yellow"/>')
    if rng.random() < 0.15:
        out.append('<w:u w:val="single"/>')
    if rng.random() < 0.1:
        out.append('<w:lang w:val="en-GB"/>')
    if not out:
        return ""
    return "<w:rPr>" + "".join(out) + "</w:rPr>"


def run(ctx, text=None, rich=True):
    attrs = ""
    if ctx.k.get("rsid_attrs") and ctx.rsid_pool and ctx.rng.random() < 0.6:
        attrs = (f' w:rsidR="{ctx.rng.choice(ctx.rsid_pool)}"'
                 f' w:rsidRPr="{ctx.rng.choice(ctx.rsid_pool)}"')
    return f"<w:r{attrs}>{rpr(ctx, rich)}<w:t xml:space=\"preserve\">{esc(text if text is not None else ctx.text(1))}</w:t></w:r>"


def revised_runs(ctx):
    """One paragraph's worth of tracked-change content (ins/del/move)."""
    rng = ctx.rng
    a = ctx.author()
    out = [run(ctx, ctx.sentence() + " ")]
    kind = rng.random()
    if kind < 0.45:
        ins_rpr = ""
        if rng.random() < 0.3:  # rPrChange snapshot inside the inserted run
            ins_rpr = ('<w:rPr><w:rPrChange w:id="%d" w:author="%s" w:date="%s">'
                       '<w:rPr><w:i/></w:rPr></w:rPrChange></w:rPr>'
                       % (ctx.next_rev(), esca(a), ctx.date()))
        out.append(f'<w:ins w:id="{ctx.next_rev()}" w:author="{esca(a)}" w:date="{ctx.date()}">'
                   f'<w:r>{ins_rpr}<w:t xml:space="preserve">{esc(ctx.sentence())} </w:t></w:r></w:ins>')
        out.append(f'<w:del w:id="{ctx.next_rev()}" w:author="{esca(a)}" w:date="{ctx.date()}">'
                   f'<w:r><w:delText xml:space="preserve">{esc(ctx.sentence())} </w:delText></w:r></w:del>')
    elif kind < 0.7:
        out.append(f'<w:moveFrom w:id="{ctx.next_rev()}" w:author="{esca(a)}" w:date="{ctx.date()}">'
                   f'<w:r><w:t xml:space="preserve">{esc(ctx.sentence())} </w:t></w:r></w:moveFrom>')
        out.append(f'<w:moveTo w:id="{ctx.next_rev()}" w:author="{esca(a)}" w:date="{ctx.date()}">'
                   f'<w:r><w:t xml:space="preserve">{esc(ctx.sentence())} </w:t></w:r></w:moveTo>')
    else:
        # property-change records ride along on a following plain paragraph
        out.append(run(ctx, ctx.sentence()))
    out.append(run(ctx, ctx.sentence()))
    return "".join(out)


def ppr(ctx, style=None, num=None, sectpr=None, extra_rpr_change=False):
    """Paragraph properties in CT_PPr sequence order (subset)."""
    rng = ctx.rng
    out = []
    if style:
        out.append(f'<w:pStyle w:val="{style}"/>')
    if rng.random() < 0.1:
        out.append("<w:keepNext/>")
    if num is not None:
        out.append(f'<w:numPr><w:ilvl w:val="{num[1]}"/><w:numId w:val="{num[0]}"/></w:numPr>')
    if rng.random() < 0.08:
        out.append('<w:tabs><w:tab w:val="left" w:pos="1440"/><w:tab w:val="right" w:pos="9360"/></w:tabs>')
    if rng.random() < 0.2:
        out.append(f'<w:spacing w:after="{rng.choice([0, 80, 160])}" w:line="276" w:lineRule="auto"/>')
    if rng.random() < 0.15:
        out.append(f'<w:ind w:left="{rng.choice([360, 720])}"/>')
    if rng.random() < 0.1:
        out.append(f'<w:jc w:val="{rng.choice(["center", "right", "both"])}"/>')
    if extra_rpr_change:
        a = ctx.author()
        out.append('<w:rPr>' + f'<w:ins w:id="{ctx.next_rev()}" w:author="{esca(a)}" w:date="{ctx.date()}"/>' + '</w:rPr>')
    if sectpr:
        out.append(sectpr)
    if not out:
        return ""
    return "<w:pPr>" + "".join(out) + "</w:pPr>"


def para(ctx, content, style=None, num=None, part="word/document.xml", paraid_p=0.35):
    attrs = ""
    if ctx.rng.random() < paraid_p:
        attrs = f' w14:paraId="{ctx.para_id(part)}" w14:textId="{ctx.para_id(part)}"'
    if ctx.k.get("rsid_attrs") and ctx.rsid_pool:
        attrs += (f' w:rsidR="{ctx.rng.choice(ctx.rsid_pool)}"'
                  f' w:rsidP="{ctx.rng.choice(ctx.rsid_pool)}"')
    return f"<w:p{attrs}>{ppr(ctx, style, num)}{content}</w:p>"


# ---------------------------------------------------------------- fields
def field_complex(ctx, instr, result_text=None):
    """begin -> instrText -> separate -> result -> end, one paragraph's runs."""
    res = esc(result_text if result_text is not None else ctx.sentence())
    return ('<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
            f'<w:r><w:instrText xml:space="preserve"> {esc(instr)} </w:instrText></w:r>'
            '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
            f'<w:r><w:t>{res}</w:t></w:r>'
            '<w:r><w:fldChar w:fldCharType="end"/></w:r>')


def field_simple(ctx, instr, result_text=None):
    res = esc(result_text if result_text is not None else ctx.sentence())
    return f'<w:fldSimple w:instr=" {esca(instr)} "><w:r><w:t>{res}</w:t></w:r></w:fldSimple>'


FIELD_INSTRS = ["DATE", "TIME", "NUMPAGES", "AUTHOR", "TITLE", "FILENAME",
                "CREATEDATE", "SAVEDATE", "SECTIONPAGES", "EDITTIME"]


def field_run(ctx):
    rng = ctx.rng
    kind = rng.random()
    if kind < 0.12 and ctx.k["form_fields"]:  # legacy form field: ffData rides inside the begin fldChar
        return ('<w:r><w:fldChar w:fldCharType="begin"><w:ffData>'
                f'<w:name w:val="StressField{rng.randint(1, 999)}"/><w:enabled/>'
                '<w:calcOnExit w:val="0"/><w:textInput><w:type w:val="regular"/>'
                '<w:default w:val="placeholder"/><w:maxLength w:val="40"/>'
                '</w:textInput></w:ffData></w:fldChar></w:r>'
                '<w:r><w:instrText xml:space="preserve"> FORMTEXT </w:instrText></w:r>'
                '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
                '<w:r><w:t>placeholder</w:t></w:r>'
                '<w:r><w:fldChar w:fldCharType="end"/></w:r>')
    if kind < 0.35 and ctx.bookmark_names:
        bm = rng.choice(ctx.bookmark_names)
        instr = rng.choice([f'REF {bm} \\h', f'PAGEREF {bm} \\h'])
        return field_complex(ctx, instr, str(rng.randint(1, 99)))
    if kind < 0.55:
        return field_simple(ctx, rng.choice(["MERGEFIELD CustomerName", "MERGEFIELD Amount"]),
                            ctx.sentence(3))
    return field_complex(ctx, rng.choice(FIELD_INSTRS), ctx.sentence(2))


def hyperlink_runs(ctx):
    """External (r:id) or internal (w:anchor) hyperlink content."""
    rng = ctx.rng
    if rng.random() < 0.5:
        rid = ctx.doc_rels.add(REL + "/hyperlink",
                               "https://example.invalid/page-%d" % rng.randint(1, 10**6),
                               external=True)
        return (f'<w:hyperlink r:id="{rid}" w:history="1">'
                f'<w:r>{rpr(ctx)}<w:t>{esc(ctx.sentence(3))}</w:t></w:r></w:hyperlink>')
    if ctx.bookmark_names:
        bm = rng.choice(ctx.bookmark_names)
        return (f'<w:hyperlink w:anchor="{esca(bm)}" w:history="1">'
                f'<w:r>{rpr(ctx)}<w:t>{esc(ctx.sentence(3))}</w:t></w:r></w:hyperlink>')
    return run(ctx)


# ---------------------------------------------------------------- math
def m_run(text):
    return f"<m:r><m:t>{esc(text)}</m:t></m:r>"


def math_expr(ctx):
    rng = ctx.rng
    kind = rng.randrange(8)
    a, b = str(rng.randint(1, 99)), str(rng.randint(2, 40))
    if kind == 0:  # fraction
        return (f"<m:f><m:num>{m_run(a)}</m:num><m:den>{m_run(b)}</m:den></m:f>")
    if kind == 1:  # radical with hidden degree
        return (f'<m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/>'
                f'<m:e>{m_run("x+" + a)}</m:e></m:rad>')
    if kind == 2:  # nary sum
        return (f'<m:nary><m:naryPr><m:chr m:val="&#8721;"/><m:limLoc m:val="undOvr"/>'
                f'<m:subHide m:val="0"/><m:supHide m:val="0"/></m:naryPr>'
                f'<m:sub>{m_run("i=1")}</m:sub><m:sup>{m_run("n")}</m:sup>'
                f'<m:e>{m_run("x")}</m:e></m:nary>')
    if kind == 3:  # subscript / superscript pair
        return (f"<m:sSubSup><m:e>{m_run('x')}</m:e><m:sub>{m_run(a)}</m:sub>"
                f"<m:sup>{m_run(b)}</m:sup></m:sSubSup>")
    if kind == 4:  # delimiter
        return (f'<m:d><m:dPr><m:begChr m:val="("/><m:endChr m:val=")"/></m:dPr>'
                f'<m:e>{m_run("a+b")}</m:e></m:d>')
    if kind == 5:  # function
        return (f"<m:func><m:fName>{m_run('sin')}</m:fName><m:e>{m_run('&#952;')}</m:e></m:func>")
    if kind == 6:  # matrix 2x2
        cells = "".join(f"<m:e>{m_run(str(rng.randint(0, 9)))}</m:e>" for _ in range(2))
        return (f'<m:m><m:mPr><m:baseJc m:val="center"/><m:plcHide m:val="1"/>'
                f'<m:mcs><m:mc><m:mcPr><m:count m:val="2"/><m:mcJc m:val="center"/></m:mcPr></m:mc></m:mcs></m:mPr>'
                f"<m:mr>{cells}</m:mr><m:mr>{cells}</m:mr></m:m>")
    # accent + limit
    return (f'<m:acc><m:accPr><m:chr m:val="&#770;"/></m:accPr><m:e>{m_run("x")}</m:e></m:acc>'
            f'<m:limLow><m:e>{m_run("lim")}</m:e><m:lim>{m_run("x&#8594;0")}</m:lim></m:limLow>')


# ---------------------------------------------------------------- drawings
def picture_xml(ctx, rid, name):
    """The corrected pic:pic — nvPicPr AND spPr children are mandatory."""
    d = ctx.next_docpr()
    return ('<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
            '<wp:extent cx="914400" cy="914400"/>'
            '<wp:effectExtent l="0" t="0" r="19050" b="0"/>'
            f'<wp:docPr id="{d}" name="Picture {d}"/>'
            '<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>'
            '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
            f'<pic:pic><pic:nvPicPr><pic:cNvPr id="{d}" name="{esca(name)}"/><pic:cNvPicPr/></pic:nvPicPr>'
            f'<pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
            '<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>'
            '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>'
            '</a:graphicData></a:graphic></wp:inline></w:drawing>')


def anchored_picture_xml(ctx, rid):
    d = ctx.next_docpr()
    ctx.anchor_height += 8192
    return (f'<w:drawing><wp:anchor simplePos="0" relativeHeight="{ctx.anchor_height}" '
            'behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">'
            '<wp:simplePos x="0" y="0"/>'
            '<wp:positionH relativeFrom="column"><wp:align>right</wp:align></wp:positionH>'
            '<wp:positionV relativeFrom="paragraph"><wp:align>top</wp:align></wp:positionV>'
            '<wp:extent cx="685800" cy="685800"/><wp:effectExtent l="0" t="0" r="0" b="0"/>'
            '<wp:wrapSquare wrapText="bothSides"/>'
            f'<wp:docPr id="{d}" name="Picture {d}"/>'
            '<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>'
            '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
            f'<pic:pic><pic:nvPicPr><pic:cNvPr id="{d}" name="Picture {d}"/><pic:cNvPicPr/></pic:nvPicPr>'
            f'<pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
            '<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="685800" cy="685800"/></a:xfrm>'
            '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>'
            '</a:graphicData></a:graphic></wp:anchor></w:drawing>')


SHAPETYPE = ('<v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" '
             'path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/>'
             '<v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype>')


def textbox_xml(ctx):
    """mc:AlternateContent — DrawingML text box (Choice) + VML (Fallback)."""
    d = ctx.next_docpr()
    inner = (f"<w:p>{run(ctx, 'Text box: ' + ctx.sentence())}</w:p>"
             f"<w:p>{run(ctx, ctx.sentence())}</w:p>")
    choice = ('<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
              '<wp:extent cx="1828800" cy="914400"/><wp:effectExtent l="0" t="0" r="0" b="0"/>'
              f'<wp:docPr id="{d}" name="TextBox {d}"/>'
              '<a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">'
              '<wps:wsp><wps:cNvSpPr txBox="1"><a:spLocks noChangeArrowheads="1"/></wps:cNvSpPr>'
              '<wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/></a:xfrm>'
              '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>'
              '<a:solidFill><a:schemeClr val="bg1"/></a:solidFill>'
              '<a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></wps:spPr>'
              f'<wps:txbx><w:txbxContent>{inner}</w:txbxContent></wps:txbx>'
              '<wps:bodyPr rot="0" vert="horz" wrap="square" lIns="91440" tIns="45720" '
              'rIns="91440" bIns="45720" anchor="t"><a:lstStyle/></wps:bodyPr>'
              '</wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing>')
    fallback = ('<w:pict>' + SHAPETYPE +
                f'<v:shape id="_x0000_s{1024 + d}" type="#_x0000_t202" '
                'style="width:144pt;height:72pt" filled="t" fillcolor="white" stroked="t">'
                f'<v:textbox inset="7.2pt,3.6pt,7.2pt,3.6pt"><w:txbxContent>{inner}</w:txbxContent></v:textbox>'
                '</v:shape></w:pict>')
    return (f'<mc:AlternateContent><mc:Choice Requires="wps">{choice}</mc:Choice>'
            f'<mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>')


# ---------------------------------------------------------------- tables
def table_xml(ctx, rows, cols, depth):
    """A table with exact grid math: every row's gridSpan sum == cols."""
    rng = ctx.rng
    grid = "<w:tblGrid>" + "".join('<w:gridCol w:w="1440"/>' for _ in range(cols)) + "</w:tblGrid>"
    if ctx.k["revisions"] and rng.random() < 0.3:
        grid = grid.replace("</w:tblGrid>",
                            f'<w:tblGridChange w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" '
                            f'w:date="{ctx.date()}"><w:tblGrid>'
                            + "".join('<w:gridCol w:w="1400"/>' for _ in range(cols))
                            + "</w:tblGrid></w:tblGridChange></w:tblGrid>")
    out = ["<w:tbl><w:tblPr>"
           + (f'<w:tblStyle w:val="{ctx.table_style_id}"/>' if getattr(ctx, "table_style_id", None) else "")
           + '<w:tblW w:w="0" w:type="auto"/><w:jc w:val="center"/>'
             '<w:tblBorders><w:top w:val="single" w:sz="4" w:color="BFBFBF"/>'
             '<w:left w:val="single" w:sz="4" w:color="BFBFBF"/>'
             '<w:bottom w:val="single" w:sz="4" w:color="BFBFBF"/>'
             '<w:right w:val="single" w:sz="4" w:color="BFBFBF"/>'
             '<w:insideH w:val="single" w:sz="4" w:color="BFBFBF"/>'
             '<w:insideV w:val="single" w:sz="4" w:color="BFBFBF"/></w:tblBorders>'
             '<w:tblLayout w:type="fixed"/><w:tblLook w:val="04A0"/>'
           + (f'<w:tblPrChange w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" '
              f'w:date="{ctx.date()}"><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
              '</w:tblPrChange>' if ctx.k["revisions"] and rng.random() < 0.2 else "")
           + '</w:tblPr>',
           grid]
    if rows > 2 and cols > 2:
        # Never column 0: the first cell may gridSpan 2 there, which would leave the vertically
        # merged cells of one run with different widths (and the merge column covered by the span).
        vmerge_col = rng.randrange(2, cols)
    elif rows > 2 and cols == 2:
        vmerge_col = rng.randrange(cols)
    else:
        vmerge_col = -1
    for ri in range(rows):
        trpr = ""
        if ri == 0 and rng.random() < 0.5:
            trpr = "<w:trPr><w:tblHeader/></w:trPr>"
        elif ctx.k["revisions"] and rng.random() < 0.12:
            trpr = (f'<w:trPr><w:ins w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" '
                    f'w:date="{ctx.date()}"/></w:trPr>')
        cells = []
        ci = 0
        while ci < cols:
            span = 1
            if ci == 0 and cols >= 3 and rng.random() < 0.25:
                span = 2
            tcpr = [f'<w:tcW w:w="{1440 * span}" w:type="dxa"/>']
            if span > 1:
                tcpr.append(f'<w:gridSpan w:val="{span}"/>')
            if ci == vmerge_col:
                if ri == 0:
                    tcpr.append('<w:vMerge w:val="restart"/>')
                else:
                    tcpr.append('<w:vMerge/>')
            if rng.random() < 0.1:
                tcpr.append('<w:shd w:val="clear" w:color="auto" w:fill="DEEAF6"/>')
            tcpr.append('<w:vAlign w:val="center"/>')
            if ctx.k["revisions"] and rng.random() < 0.06:
                tcpr.append(f'<w:cellIns w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" w:date="{ctx.date()}"/>')
            elif ctx.k["revisions"] and rng.random() < 0.04:
                tcpr.append(f'<w:cellDel w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" w:date="{ctx.date()}"/>')
            if ctx.k["revisions"] and rng.random() < 0.05:
                tcpr.append(f'<w:tcPrChange w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" '
                            f'w:date="{ctx.date()}"><w:tcPr><w:tcW w:w="1440" w:type="dxa"/></w:tcPr>'
                            '</w:tcPrChange>')
            inner = []
            if depth > 0 and ri == 1 and ci == 0:
                inner.append(table_xml(ctx, 2, 2, depth - 1))
                inner.append(para(ctx, run(ctx, "After nested table.")))
            elif ctx.k["revisions"] and ctx.rng.random() < ctx.k["rev_density"] * 0.5:
                inner.append(para(ctx, revised_runs(ctx)))
            else:
                inner.append(para(ctx, run(ctx, f"Cell {ri},{ci}: " + ctx.sentence())))
            cells.append("<w:tc><w:tcPr>" + "".join(tcpr) + "</w:tcPr>" + "".join(inner) + "</w:tc>")
            ci += span
        out.append("<w:tr>" + trpr + "".join(cells) + "</w:tr>")
    out.append("</w:tbl>")
    return "".join(out)


# ---------------------------------------------------------------- planning
def plan_styles(ctx):
    k, rng = ctx.k, ctx.rng
    styles = []

    def add(sid, name, typ, **kw):
        s = {"id": sid, "name": name, "type": typ}
        s.update(kw)
        styles.append(s)
        return s

    add("Normal", "Normal", "paragraph", default=True, qFormat=True, uiPriority=0)
    add("DefaultParagraphFont", "Default Paragraph Font", "character", default=True, semiHidden=True)
    add("TableNormal", "Normal Table", "table", default=True, semiHidden=True)
    add("NoList", "No List", "numbering", default=True, semiHidden=True)
    ctx.table_style_id = "StressTable"
    add("StressTable", "Stress Table", "table", uiPriority=40,
        tblPr='<w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
              '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr>',
        tblStylePr='<w:tblStylePr w:type="firstRow"><w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="D9E2F3"/></w:tcPr></w:tblStylePr>')
    for i in (1, 2, 3):
        add(f"Heading{i}", f"heading {i}", "paragraph", basedOn="Normal", next="Normal",
            qFormat=True, uiPriority=9, outline=i - 1, keepNext=True)
        add(f"Heading{i}Char", f"Heading {i} Char", "character", link=f"Heading{i}")
        styles[-2]["link"] = f"Heading{i}Char"
    add("Title", "Title", "paragraph", basedOn="Normal", qFormat=True, uiPriority=10)
    add("Quote", "Quote", "paragraph", basedOn="Normal", qFormat=True, uiPriority=29)
    add("ListParagraph", "List Paragraph", "paragraph", basedOn="Normal", uiPriority=34)
    # basedOn chain of the requested depth
    prev = "Normal"
    for i in range(k["basedon_depth"]):
        sid = f"Chain{i:02d}"
        add(sid, f"Chain Style {i}", "paragraph", basedOn=prev, next="Normal", uiPriority=50)
        prev = sid
    chain_end = prev
    # wide fan of styles (most are orphans — defined, never used)
    n_wide = max(0, k["styles"] - len(styles))
    for i in range(n_wide):
        base = rng.choice(["Normal", chain_end, "ListParagraph", "Quote"])
        add(f"Stress{i:03d}", f"Stress Style {i}", "paragraph", basedOn=base,
            next="Normal", uiPriority=rng.randint(1, 99),
            aliases=f"S{i},Alt{i}" if i % 7 == 0 else None,
            semiHidden=(i % 11 == 0))
    ctx.style_plan = styles
    ctx.style_ids = [s["id"] for s in styles if s["type"] == "paragraph"
                     and s["id"] not in ("Normal",) and not s.get("semiHidden")]
    ctx.char_style_ids = [s["id"] for s in styles if s["type"] == "character"]


def render_styles(ctx):
    out = [XML_DECL, f'<w:styles xmlns:w="{NS["w"]}" xmlns:r="{NS["r"]}">']
    out.append('<w:docDefaults><w:rPrDefault><w:rPr>'
               '<w:rFonts w:ascii="Calibri" w:eastAsia="Calibri" w:hAnsi="Calibri" w:cs="Calibri"/>'
               '<w:lang w:val="en-US" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>'
               '<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr>'
               '</w:pPrDefault></w:docDefaults>')
    exc = "".join(
        f'<w:lsdException w:name="{esca(n)}" w:uiPriority="{i % 100}" '
        f'w:semiHidden="{0 if i % 3 else 1}" w:unhideWhenUsed="{i % 2}" w:qFormat="{1 if i % 5 == 0 else 0}"/>'
        for i, n in enumerate(LATENT_NAMES))
    if ctx.k["latent_styles"]:
        out.append(f'<w:latentStyles w:defLockedState="0" w:defUIPriority="99" w:defSemiHidden="1" '
                   f'w:defUnhideWhenUsed="1" w:defQFormat="0" w:count="{len(LATENT_NAMES)}">{exc}</w:latentStyles>')
    for s in ctx.style_plan:
        attrs = f' w:type="{s["type"]}" w:styleId="{s["id"]}"'
        if s.get("default"):
            attrs = f' w:type="{s["type"]}" w:default="1" w:styleId="{s["id"]}"'
        body = [f'<w:name w:val="{esca(s["name"])}"/>']
        if s.get("aliases"):
            body.append(f'<w:aliases w:val="{esca(s["aliases"])}"/>')
        if s.get("basedOn"):
            body.append(f'<w:basedOn w:val="{s["basedOn"]}"/>')
        if s.get("next"):
            body.append(f'<w:next w:val="{s["next"]}"/>')
        if s.get("link"):
            body.append(f'<w:link w:val="{s["link"]}"/>')
        if s.get("uiPriority") is not None:
            body.append(f'<w:uiPriority w:val="{s["uiPriority"]}"/>')
        if s.get("semiHidden"):
            body.append("<w:semiHidden/>")
            body.append("<w:unhideWhenUsed/>")
        if s.get("qFormat"):
            body.append("<w:qFormat/>")
        if s["type"] == "paragraph":
            pp = []
            if s.get("keepNext"):
                pp.append("<w:keepNext/>")
            pp.append(f'<w:spacing w:before="{ctx.rng.choice([0, 120, 240])}" w:after="80"/>')
            if s.get("outline") is not None:
                pp.append(f'<w:outlineLvl w:val="{s["outline"]}"/>')
            body.append("<w:pPr>" + "".join(pp) + "</w:pPr>")
            rp = ['<w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/>']
            if s["id"].startswith("Heading"):
                rp.append("<w:b/>")
                rp.append(f'<w:sz w:val="{40 - 4 * int(s["id"][-1])}"/>')
            body.append("<w:rPr>" + "".join(rp) + "</w:rPr>")
        if s.get("tblPr"):
            body.append(s["tblPr"])
        if s.get("tblStylePr"):
            body.append(s["tblStylePr"])
        out.append(f"<w:style{attrs}>" + "".join(body) + "</w:style>")
    out.append("</w:styles>")
    return "".join(out)


NUM_FMTS = ["decimal", "lowerLetter", "lowerRoman", "bullet", "upperLetter", "decimalZero"]


def plan_numbering(ctx):
    """abstractNum/num plan; the last `num_orphans` abstracts get no w:num."""
    k, rng = ctx.k, ctx.rng
    total = k["abstract_nums"]
    orphans = min(k["num_orphans"], total)
    abstracts, nums = [], []
    num_id = 0
    for ai in range(total):
        lvls = []
        for ilvl in range(9):
            fmt = NUM_FMTS[(ai + ilvl) % len(NUM_FMTS)]
            lvls.append({"ilvl": ilvl, "fmt": fmt,
                         "text": "" if fmt == "bullet" else f"%{ilvl + 1}.",
                         "pic": (ilvl == 0 and ai == 0 and fmt == "bullet")})
        abstracts.append({"id": ai, "lvls": lvls,
                          "nsid": "%08X" % rng.randrange(0, 0xFFFFFFFF)})
        if ai < total - orphans:
            for _ in range(rng.choice([1, 1, 2])):
                num_id += 1
                nums.append({"numId": num_id, "abs": ai,
                             "startOverride": rng.choice([None, None, 1, 4])})
    ctx.abstracts, ctx.nums = abstracts, nums
    ctx.num_ids = [n["numId"] for n in nums]
    ctx.has_pic_bullet = total > 0


def render_numbering(ctx):
    out = [XML_DECL, f'<w:numbering xmlns:w="{NS["w"]}" xmlns:r="{NS["r"]}" xmlns:v="{NS["v"]}">']
    if ctx.has_pic_bullet:
        out.append('<w:numPicBullet w:numPicBulletId="0"><w:pict>'
                   f'<v:shape style="width:9pt;height:9pt"><v:imagedata r:id="rId1"/></v:shape>'
                   '</w:pict></w:numPicBullet>')
    for a in ctx.abstracts:
        body = [f'<w:nsid w:val="{a["nsid"]}"/>', '<w:multiLevelType w:val="multilevel"/>']
        for lv in a["lvls"]:
            lb = [f'<w:start w:val="1"/><w:numFmt w:val="{lv["fmt"]}"/>']
            if lv["fmt"] != "bullet" and lv["ilvl"] > 0:
                lb.append(f'<w:lvlRestart w:val="{lv["ilvl"]}"/>')
            lb.append(f'<w:lvlText w:val="{esca(lv["text"])}"/>')
            if lv.get("pic"):
                lb.append('<w:lvlPicBulletId w:val="0"/>')
            lb.append('<w:lvlJc w:val="left"/>')
            lb.append(f'<w:pPr><w:ind w:left="{720 * (lv["ilvl"] + 1)}" w:hanging="360"/></w:pPr>')
            body.append(f'<w:lvl w:ilvl="{lv["ilvl"]}" w:tplc="0409000F" w:tentative="1">'
                        + "".join(lb) + "</w:lvl>")
        out.append(f'<w:abstractNum w:abstractNumId="{a["id"]}">'
                   + "".join(body) + "</w:abstractNum>")
    for n in ctx.nums:
        ovr = ""
        if n["startOverride"]:
            ovr = (f'<w:lvlOverride w:ilvl="0"><w:startOverride w:val="{n["startOverride"]}"/>'
                   '</w:lvlOverride>')
        out.append(f'<w:num w:numId="{n["numId"]}"><w:abstractNumId w:val="{n["abs"]}"/>'
                   + ovr + "</w:num>")
    out.append("</w:numbering>")
    return "".join(out)


def render_settings(ctx):
    k, rng = ctx.k, ctx.rng
    if ctx.rsid_pool is not None:  # settings lists the whole shared pool
        rsids = "".join(f'<w:rsid w:val="{v}"/>' for v in ctx.rsid_pool)
    else:
        rsids = "".join(f'<w:rsid w:val="{"%08X" % rng.randrange(0, 0xFFFFFFFF)}"/>'
                        for _ in range(k["rsids"]))
    out = [XML_DECL,
           f'<w:settings xmlns:w="{NS["w"]}" xmlns:r="{NS["r"]}" xmlns:m="{NS["m"]}" '
           f'xmlns:w14="{NS["w14"]}" xmlns:mc="{NS["mc"]}" mc:Ignorable="w14">']
    out.append('<w:zoom w:percent="100"/>')
    out.append('<w:proofState w:spelling="clean" w:grammar="clean"/>')
    if k["rev_density"] > 0 and k["revisions"]:
        out.append("<w:trackRevisions/>")
    out.append('<w:defaultTabStop w:val="720"/>')
    out.append("<w:evenAndOddHeaders/>")
    out.append('<w:drawingGridHorizontalSpacing w:val="120"/>')
    out.append('<w:drawingGridVerticalSpacing w:val="120"/>')
    out.append('<w:characterSpacingControl w:val="doNotCompress"/>')
    out.append('<w:compat><w:spaceForUL/><w:balanceSingleByteDoubleByteWidth/>'
               '<w:doNotLeaveBackslashAlone/><w:ulTrailSpace/><w:doNotExpandShiftReturn/>'
               '<w:adjustLineHeightInTable/>'
               '<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>'
               '<w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>'
               '<w:compatSetting w:name="enableOpenTypeFeatures" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>'
               '</w:compat>')
    out.append('<w:docVars><w:docVar w:name="StressSeed" w:val="seed"/>'
               '<w:docVar w:name="Generator" w:val="gen_stress"/></w:docVars>')
    out.append(f'<w:rsids><w:rsidRoot w:val="{"%08X" % rng.randrange(0, 0xFFFFFFFF)}"/>'
               + rsids + "</w:rsids>")
    out.append('<m:mathPr><m:mathFont m:val="Cambria Math"/><m:brkBin m:val="before"/>'
               '<m:brkBinSub m:val="--"/><m:smallFrac m:val="0"/><m:dispDef/>'
               '<m:lMargin m:val="0"/><m:rMargin m:val="0"/><m:defJc m:val="centerGroup"/>'
               '<m:wrapIndent m:val="1440"/><m:intLim m:val="subSup"/>'
               '<m:naryLim m:val="undOvr"/></m:mathPr>')
    if k["customxml"]:
        out.append(f'<w:attachedSchema w:val="{CUSTOMXML_SCHEMA_URI}"/>')
    out.append('<w:themeFontLang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>')
    out.append('<w:clrSchemeMapping w:bg1="light1" w:t1="dark1" w:bg2="light2" w:t2="dark2" '
               'w:accent1="accent1" w:accent2="accent2" w:accent3="accent3" w:accent4="accent4" '
               'w:accent5="accent5" w:accent6="accent6" w:hyperlink="hyperlink" '
               'w:followedHyperlink="followedHyperlink"/>')
    out.append('<w:decimalSymbol w:val="."/><w:listSeparator w:val=","/>')
    out.append("</w:settings>")
    return "".join(out)


def plan_comments(ctx):
    k, rng = ctx.k, ctx.rng
    threads = []
    cid = 0
    for _ in range(k["comment_threads"]):
        def mk(is_root):
            nonlocal cid
            c = {"cid": cid, "author": ctx.author(), "date": ctx.date(),
                 "paras": [], "durable": rng.randrange(1, 2**31)}
            cid += 1
            for _ in range(rng.randint(1, 2)):
                c["paras"].append((ctx.para_id("word/comments.xml"), ctx.text()))
            c["paraId"] = c["paras"][-1][0]
            return c
        root = mk(True)
        replies = [mk(False) for _ in range(rng.randint(0, k["comment_depth"]))]
        threads.append({"root": root, "replies": replies})
    ctx.comment_plan = threads
    ctx.all_comments = []
    for t in threads:
        ctx.all_comments.append(t["root"])
        ctx.all_comments.extend(t["replies"])
    ctx.pending_comment_roots = [t["root"]["cid"] for t in threads]


def render_comments(ctx):
    out = [XML_DECL, f'<w:comments xmlns:w="{NS["w"]}" xmlns:w14="{NS["w14"]}" '
           f'xmlns:mc="{NS["mc"]}" mc:Ignorable="w14">']
    for i, c in enumerate(ctx.all_comments):
        paras = []
        for j, (pid, txt) in enumerate(c["paras"]):
            inner = run(ctx, txt)
            if i == 1 and j == 0 and ctx.k["revisions"]:  # one comment carries a tracked insertion
                inner += (f'<w:ins w:id="{ctx.next_rev()}" w:author="{esca(c["author"])}" '
                          f'w:date="{c["date"]}"><w:r><w:t> (noted)</w:t></w:r></w:ins>')
            if i == 2 and j == 0:  # one comment carries a paired bookmark
                bid = ctx.next_bookmark()
                inner = (f'<w:bookmarkStart w:id="{bid}" w:name="commentBm{bid}"/>'
                         + inner + f'<w:bookmarkEnd w:id="{bid}"/>')
            paras.append(f'<w:p w14:paraId="{pid}" w14:textId="{ctx.para_id("word/comments.xml")}">'
                         + inner + "</w:p>")
        out.append(f'<w:comment w:id="{c["cid"]}" w:author="{esca(c["author"])}" '
                   f'w:date="{c["date"]}" w:initials="A{1 + (c["cid"] % 9)}">'
                   + "".join(paras) + "</w:comment>")
    out.append("</w:comments>")
    return "".join(out)


def render_comments_extended(ctx):
    out = [XML_DECL, f'<w15:commentsEx xmlns:w15="{NS["w15"]}">']
    for t in ctx.comment_plan:
        out.append(f'<w15:commentEx w15:paraId="{t["root"]["paraId"]}" w15:done="0"/>')
        for r in t["replies"]:
            out.append(f'<w15:commentEx w15:paraId="{r["paraId"]}" '
                       f'w15:paraIdParent="{t["root"]["paraId"]}" w15:done="0"/>')
    out.append("</w15:commentsEx>")
    return "".join(out)


def render_comments_ids(ctx):
    out = [XML_DECL, f'<w16cid:commentsIds xmlns:w16cid="{NS["w16cid"]}">']
    for c in ctx.all_comments:
        out.append(f'<w16cid:commentId w16cid:paraId="{c["paraId"]}" '
                   f'w16cid:durableId="{c["durable"]}"/>')
    out.append("</w16cid:commentsIds>")
    return "".join(out)


def render_people(ctx):
    out = [XML_DECL, f'<w15:people xmlns:w15="{NS["w15"]}">']
    for i, a in enumerate(ctx.authors):
        out.append(f'<w15:person w15:author="{esca(a)}">'
                   f'<w15:presenceInfo w15:providerId="AD" '
                   f'w15:userId="author{i + 1}@example.invalid"/></w15:person>')
    out.append("</w15:people>")
    return "".join(out)


def plan_notes(ctx):
    for kind in ("footnotes", "endnotes"):
        n = ctx.k[kind]
        notes = [(i + 2, ctx.text()) for i in range(n)]
        setattr(ctx, kind, notes)
        setattr(ctx, "pending_" + kind, [nid for nid, _ in notes])


def render_notes(ctx, kind):
    root = "w:footnotes" if kind == "footnotes" else "w:endnotes"
    one = "w:footnote" if kind == "footnotes" else "w:endnote"
    out = [XML_DECL, f'<{root} xmlns:w="{NS["w"]}">']
    out.append(f'<{one} w:type="separator" w:id="0"><w:p><w:r><w:separator/></w:r></w:p></{one}>')
    out.append(f'<{one} w:type="continuationSeparator" w:id="1"><w:p><w:r>'
               f'<w:continuationSeparator/></w:r></w:p></{one}>')
    for nid, txt in getattr(ctx, kind):
        extra = ""
        if nid == 2 and ctx.k["revisions"]:  # first real note carries a revision, for story coverage
            extra = (f'<w:ins w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" '
                     f'w:date="{ctx.date()}"><w:r><w:t> (rev.)</w:t></w:r></w:ins>')
        out.append(f'<{one} w:id="{nid}"><w:p>{run(ctx, txt)}{extra}</w:p></{one}>')
    out.append(f"</{root}>")
    return "".join(out)


# ---------------------------------------------------------------- media / sections
def plan_media(ctx):
    k, rng = ctx.k, ctx.rng
    for i in range(k["media_parts"]):
        w, h = (220, 220) if i else (384, 384)
        name = f"word/media/image{i + 1}.png"
        ctx.add_part(name, make_png(rng, w, h))
        rid = ctx.doc_rels.add(REL + "/image", f"media/image{i + 1}.png")
        ctx.media.append((name, rid))
    for i in range(k["media_unref"]):
        name = f"word/media/unref{i + 1}.png"
        ctx.add_part(name, make_png(rng, 96, 96))  # orphan part: no rel, legal
    # small bullet image owned by numbering.xml (only when a numbering part
    # exists that references it)
    if ctx.has_pic_bullet:
        ctx.add_part("word/media/bullet.png", make_png(rng, 16, 16))
    ctx.pending_media_refs = k["media_refs"]


def plan_sections(ctx):
    """Create header/footer parts for every section; store sectPr fragments."""
    k, rng = ctx.k, ctx.rng
    ctx.section_rels = []  # per section: dict variant->rid, kind->partname
    hn = fn = 0
    for si in range(k["sections"]):
        entry = {}
        for kind, tag in (("header", "w:hdr"), ("footer", "w:ftr")):
            for variant in (("default", "even", "first") if ctx.k["section_extras"]
                            else ("default",)):
                if kind == "header":
                    hn += 1
                    part = f"word/header{hn}.xml"
                else:
                    fn += 1
                    part = f"word/footer{fn}.xml"
                logo_rid = None
                if kind == "header" and variant == "default" and si == 0 and ctx.media:
                    # a picture inside a header resolves r:embed against the
                    # HEADER's own rels (OPC relationships are per-part)
                    hrels = Rels()
                    logo_rid = hrels.add(REL + "/image", "media/image1.png")
                    ctx.add_part(f"word/_rels/{part.split('/')[-1]}.rels", hrels.render())
                content = header_footer_content(ctx, part, kind, variant, si,
                                                logo_rid=logo_rid)
                ctx.add_part(part, content,
                             "application/vnd.openxmlformats-officedocument.wordprocessingml."
                             + kind + "+xml", override=True)
                rid = ctx.doc_rels.add(REL + "/" + kind, part.split("/")[-1])
                entry[(kind, variant)] = rid
        ctx.section_rels.append(entry)


def header_footer_content(ctx, part, kind, variant, si, logo_rid=None):
    tag = "w:hdr" if kind == "header" else "w:ftr"
    rng = ctx.rng
    paras = []
    if kind == "header":
        txt = f"Section {si + 1} {variant} header — " + ctx.sentence()
        if logo_rid is not None:
            paras.append(para(ctx, run(ctx, "Logo: ") +
                              f"<w:r>{picture_xml(ctx, logo_rid, 'logo.png')}</w:r>",
                              part=part))
        elif variant == "default" and si == 1 and ctx.k["tables"] > 0:
            paras.append(table_xml(ctx, 2, 3, 0))
            paras.append(para(ctx, run(ctx, txt), part=part))
        else:
            paras.append(para(ctx, run(ctx, txt), part=part))
    elif ctx.k["fields"] > 0:
        fld = field_complex(ctx, "PAGE", "1") + run(ctx, " of ", rich=False) \
            + field_simple(ctx, "NUMPAGES", "9")
        paras.append(para(ctx, run(ctx, f"Section {si + 1} {variant} footer — ", rich=False) + fld,
                          part=part))
    else:
        paras.append(para(ctx, run(ctx, f"Section {si + 1} {variant} footer — Page 1 of 9",
                                   rich=False), part=part))
    return (XML_DECL + f'<{tag} xmlns:w="{NS["w"]}" xmlns:r="{NS["r"]}" '
            f'xmlns:wp="{NS["wp"]}" xmlns:a="{NS["a"]}" xmlns:pic="{NS["pic"]}" '
            f'xmlns:w14="{NS["w14"]}" xmlns:mc="{NS["mc"]}" mc:Ignorable="w14">'
            + "".join(paras) + f"</{tag}>")


def sectpr_xml(ctx, si, last=False):
    entry = ctx.section_rels[si]
    out = ["<w:sectPr>"]
    for (kind, variant), rid in entry.items():
        out.append(f'<w:{kind}Reference w:type="{variant}" r:id="{rid}"/>')
    out.append(f'<w:type w:val="{"continuous" if si % 3 == 2 else "nextPage"}"/>')
    out.append('<w:pgSz w:w="12240" w:h="15840"/>')
    out.append('<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" '
               'w:header="720" w:footer="720" w:gutter="0"/>')
    if si == 1:
        out.append('<w:pgNumType w:fmt="upperRoman"/>')
    if si == 2:
        out.append('<w:cols w:space="720" w:num="2"/>')
    if ctx.k["section_extras"]:
        out.append("<w:titlePg/>")
    out.append('<w:docGrid w:linePitch="360"/>')
    if si % 2 == 0 and ctx.k["revisions"]:  # sectPrChange snapshot of the previous section setup
        out.append(f'<w:sectPrChange w:id="{ctx.next_rev()}" w:author="{esca(ctx.author())}" '
                   f'w:date="{ctx.date()}"><w:sectPr><w:pgSz w:w="12240" w:h="15840"/>'
                   '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>'
                   '</w:sectPr></w:sectPrChange>')
    return "".join(out) + "</w:sectPr>"


# ---------------------------------------------------------------- chart / misc parts
def render_chart(ctx):
    rng = ctx.rng
    cats = ["Alpha", "Beta", "Gamma", "Delta"]
    vals = [round(rng.uniform(1, 9), 1) for _ in cats]
    pts = "".join(f'<c:pt idx="{i}"><c:v>{esc(c)}</c:v></c:pt>' for i, c in enumerate(cats))
    vpts = "".join(f'<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>' for i, v in enumerate(vals))
    xml = (XML_DECL + f'<c:chartSpace xmlns:c="{NS["c"]}" xmlns:a="{NS["a"]}" xmlns:r="{NS["r"]}">'
           '<c:chart><c:autoTitleDeleted c:val="1"/><c:plotArea><c:layout/>'
           '<c:barChart><c:barDir c:val="col"/><c:grouping c:val="clustered"/>'
           '<c:varyColors c:val="0"/><c:ser><c:idx c:val="0"/><c:order c:val="0"/>'
           '<c:tx><c:strRef><c:strCache><c:ptCount c:val="1"/>'
           '<c:pt idx="0"><c:v>Series 1</c:v></c:pt></c:strCache></c:strRef></c:tx>'
           f'<c:cat><c:strRef><c:strCache><c:ptCount c:val="{len(cats)}"/>{pts}</c:strCache></c:strRef></c:cat>'
           f'<c:val><c:numRef><c:numCache><c:formatCode>General</c:formatCode>'
           f'<c:ptCount c:val="{len(vals)}"/>{vpts}</c:numCache></c:numRef></c:val></c:ser>'
           '<c:gapWidth c:val="150"/><c:axId c:val="111111111"/><c:axId c:val="222222222"/>'
           '</c:barChart>'
           '<c:catAx><c:axId c:val="111111111"/><c:scaling><c:orientation c:val="minMax"/></c:scaling>'
           '<c:delete c:val="0"/><c:axPos c:val="b"/><c:tickLblPos c:val="nextTo"/>'
           '<c:crossAx c:val="222222222"/><c:crosses c:val="autoZero"/><c:auto c:val="1"/>'
           '<c:lblAlgn c:val="ctr"/><c:lblOffset c:val="100"/></c:catAx>'
           '<c:valAx><c:axId c:val="222222222"/><c:scaling><c:orientation c:val="minMax"/></c:scaling>'
           '<c:delete c:val="0"/><c:axPos c:val="l"/><c:majorGridlines/>'
           '<c:tickLblPos c:val="nextTo"/><c:crossAx c:val="111111111"/>'
           '<c:crosses c:val="autoZero"/><c:crossBetween c:val="between"/></c:valAx>'
           '</c:plotArea><c:plotVisOnly c:val="1"/></c:chart>'
           '<c:externalData r:id="rId1"><c:autoUpdate c:val="0"/></c:externalData></c:chartSpace>')
    return xml, make_xlsx(cats, vals)


def chart_drawing(ctx, rid):
    d = ctx.next_docpr()
    return ('<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
            '<wp:extent cx="5486400" cy="3200400"/><wp:effectExtent l="0" t="0" r="0" b="0"/>'
            f'<wp:docPr id="{d}" name="Chart {d}"/>'
            '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">'
            f'<c:chart r:id="{rid}"/></a:graphicData></a:graphic></wp:inline></w:drawing>')


def render_glossary(ctx):
    rng = ctx.rng
    parts = []
    for i in range(3):
        guid = "{%08X-%04X-%04X-%04X-%012X}" % (
            rng.randrange(0, 0xFFFFFFFF), rng.randrange(0, 0xFFFF), rng.randrange(0, 0xFFFF),
            rng.randrange(0, 0xFFFF), rng.randrange(0, 0xFFFFFFFFFFFF))
        gallery = "autoTxt" if i % 2 == 0 else "quickParts"
        body = para(ctx, run(ctx, f"Building block {i}: " + ctx.sentence()),
                    part="word/glossary/document.xml")
        body += para(ctx, run(ctx, ctx.sentence()), part="word/glossary/document.xml")
        parts.append(
            '<w:docPart><w:docPartPr>'
            f'<w:name w:val="Stress Block {i}"/>'
            f'<w:category><w:name w:val="General"/><w:gallery w:val="{gallery}"/></w:category>'
            f'<w:types><w:type w:val="{gallery}"/></w:types>'
            '<w:behaviors><w:behavior w:val="content"/></w:behaviors>'
            f'<w:description w:val="Generated building block {i}"/>'
            f'<w:guid w:val="{guid}"/></w:docPartPr>'
            f'<w:docPartBody>{body}</w:docPartBody></w:docPart>')
    return (XML_DECL + f'<w:glossaryDocument xmlns:w="{NS["w"]}" xmlns:w14="{NS["w14"]}" '
            f'xmlns:mc="{NS["mc"]}" mc:Ignorable="w14"><w:docParts>'
            + "".join(parts) + "</w:docParts></w:glossaryDocument>")


FONTS = [("Calibri", "swiss", "020B0504020202020204"), ("Cambria", "roman", "020B0304020202020204"),
         ("Consolas", "modern", "020B0609020202020204"), ("Georgia", "roman", "020B0304020202020204"),
         ("Verdana", "swiss", "020B0604020202020204"), ("Symbol", "roman", "020B0504020202020204")]


def render_font_table(ctx):
    out = [XML_DECL, f'<w:fonts xmlns:w="{NS["w"]}" xmlns:r="{NS["r"]}">']
    for name, fam, panose in FONTS:
        out.append(f'<w:font w:name="{name}"><w:altName w:val="{name} Alt"/>'
                   f'<w:panose1 w:val="{panose}"/><w:charset w:val="00"/>'
                   f'<w:family w:val="{fam}"/><w:pitch w:val="variable"/>'
                   '<w:sig w:usb0="20007A87" w:usb1="80000000" w:usb2="00000008" '
                   'w:usb3="00000000" w:csb0="000001FF" w:csb1="00000000"/></w:font>')
    out.append("</w:fonts>")
    return "".join(out)


def render_docprops(ctx):
    core = (XML_DECL + '<cp:coreProperties '
            'xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" '
            'xmlns:dc="http://purl.org/dc/elements/1.1/" '
            'xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
            '<dc:title>Stress Document</dc:title><dc:creator>gen_stress</dc:creator>'
            '<cp:lastModifiedBy>gen_stress</cp:lastModifiedBy><cp:revision>3</cp:revision>'
            '<dcterms:created xsi:type="dcterms:W3CDTF">2026-11-06T12:00:00Z</dcterms:created>'
            '<dcterms:modified xsi:type="dcterms:W3CDTF">2026-11-06T12:30:00Z</dcterms:modified>'
            '</cp:coreProperties>')
    app = (XML_DECL + '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">'
           '<Application>gen_stress</Application><DocSecurity>0</DocSecurity>'
           '<ScaleCrop>false</ScaleCrop><Company>WordCraft</Company>'
           '<AppVersion>1.0000</AppVersion></Properties>')
    return core, app


# ---------------------------------------------------------------- body building
def comment_para(ctx, cid):
    anchor = (f'<w:commentRangeStart w:id="{cid}"/>'
              f'<w:r><w:t>{esc(ctx.sentence())}</w:t></w:r>'
              f'<w:commentRangeEnd w:id="{cid}"/>'
              f'<w:r><w:commentReference w:id="{cid}"/></w:r>')
    return para(ctx, run(ctx, ctx.sentence() + " ") + anchor + run(ctx, " " + ctx.sentence()))


def note_para(ctx, kind):
    pend = getattr(ctx, "pending_" + kind)
    nid = pend.pop(0)
    ref = "w:footnoteReference" if kind == "footnotes" else "w:endnoteReference"
    return para(ctx, run(ctx, ctx.sentence() + " ")
                + f'<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><{ref} w:id="{nid}"/></w:r>'
                + run(ctx, " " + ctx.sentence()))


def build_blocks(ctx):
    k, rng = ctx.k, ctx.rng
    blocks = []
    fields_left = k["fields"]
    math_left = k["math"]
    boxes_left = k["textboxes"]
    bm_left = k["bookmarks"]
    tables_left = k["tables"]
    n = k["paragraphs"]
    links_left = max(4, n // 25)  # cap: hyperlinks are a flavour, not the bulk

    def sprinkle_bm(content):
        nonlocal bm_left
        if bm_left > 0 and rng.random() < 0.3:
            bm_left -= 1
            bid = ctx.next_bookmark()
            name = f"bm{bid:04d}"
            ctx.bookmark_names.append(name)
            return (f'<w:bookmarkStart w:id="{bid}" w:name="{name}"/>'
                    + content + f'<w:bookmarkEnd w:id="{bid}"/>')
        return content

    if n >= 40 and k["fields"] > 0:  # a TOC field spanning several paragraphs (legal field layout)
        blocks.append(para(ctx, '<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
                           '<w:r><w:instrText xml:space="preserve"> TOC \\o "1-3" \\h \\z \\u </w:instrText></w:r>'
                           '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'))
        for _ in range(3):
            blocks.append(para(ctx, run(ctx, "Contents entry — " + ctx.sentence(3))))
        blocks.append(para(ctx, '<w:r><w:fldChar w:fldCharType="end"/></w:r>'))

    for i in range(n):
        if tables_left and i and i % max(1, n // max(1, k["tables"])) == 0:
            tables_left -= 1
            blocks.append(table_xml(ctx, k["table_rows"], k["table_cols"], k["table_depth"]))
        r = rng.random()
        if ctx.pending_comment_roots and r < 0.10:
            blocks.append(comment_para(ctx, ctx.pending_comment_roots.pop(0)))
        elif ctx.pending_footnotes and r < 0.16:
            blocks.append(note_para(ctx, "footnotes"))
        elif ctx.pending_endnotes and r < 0.19:
            blocks.append(note_para(ctx, "endnotes"))
        elif k["revisions"] and r < 0.19 + k["rev_density"]:
            content = revised_runs(ctx)
            if rng.random() < 0.25:  # pPrChange snapshot on some revised paragraphs
                a = ctx.author()
                content = ('<w:pPr><w:pPrChange w:id="%d" w:author="%s" w:date="%s">'
                           '<w:pPr><w:spacing w:after="0"/></w:pPr></w:pPrChange></w:pPr>'
                           % (ctx.next_rev(), esca(a), ctx.date())) + content
                blocks.append(f"<w:p>{content}</w:p>")
            else:
                blocks.append(para(ctx, sprinkle_bm(content)))
        elif fields_left and r < 0.55:
            fields_left -= 1
            blocks.append(para(ctx, sprinkle_bm(run(ctx, ctx.sentence() + " ") + field_run(ctx))))
        elif math_left and r < 0.62:
            math_left -= 1
            if rng.random() < 0.3:
                blocks.append(f"<w:p><m:oMathPara><m:oMath>{math_expr(ctx)}</m:oMath></m:oMathPara></w:p>")
            else:
                blocks.append(para(ctx, run(ctx, ctx.sentence(3) + " ")
                                   + f"<m:oMath>{math_expr(ctx)}</m:oMath>" + run(ctx, ".")))
        elif ctx.pending_media_refs and r < 0.70 and ctx.media:
            ctx.pending_media_refs -= 1
            _, rid = ctx.media[rng.randrange(len(ctx.media))]
            pic = anchored_picture_xml(ctx, rid) if rng.random() < 0.2 else picture_xml(ctx, rid, "image.png")
            blocks.append(para(ctx, f"<w:r>{pic}</w:r>", paraid_p=0.0))
        elif boxes_left and r < 0.75:
            boxes_left -= 1
            blocks.append(para(ctx, f"<w:r>{textbox_xml(ctx)}</w:r>", paraid_p=0.0))
        elif links_left and ctx.f_links and r < 0.82:
            links_left -= 1
            blocks.append(para(ctx, sprinkle_bm(run(ctx, ctx.sentence() + " ") + hyperlink_runs(ctx))))
        elif r < 0.90 and ctx.num_ids:
            numid = rng.choice(ctx.num_ids)
            blocks.append(para(ctx, run(ctx), num=(numid, rng.randrange(3))))
        elif ctx.style_ids:
            blocks.append(para(ctx, run(ctx), style=rng.choice(ctx.style_ids)))
        else:
            content = run(ctx)
            if ctx.f_proof and rng.random() < 0.2:  # proofing markers ride along
                content = '<w:proofErr w:type="spellStart"/>' + content + '<w:proofErr w:type="spellEnd"/>'
            if ctx.f_sdt and rng.random() < 0.06:  # inline SDT
                content += (f'<w:sdt><w:sdtPr><w:id w:val="{ctx.next_sdt()}"/>'
                            f'<w:tag w:val="stress"/><w:text/></w:sdtPr>'
                            f'<w:sdtContent>{run(ctx, ctx.sentence(2))}</w:sdtContent></w:sdt>')
            if ctx.f_smart and rng.random() < 0.04:  # smart tag flavour
                content += (f'<w:smartTag w:uri="urn:stress" w:element="city">{run(ctx, "Denver")}</w:smartTag>')
            blocks.append(para(ctx, sprinkle_bm(content)))

    # force-place whatever the dice did not reach
    while ctx.pending_comment_roots:
        blocks.append(comment_para(ctx, ctx.pending_comment_roots.pop(0)))
    while ctx.pending_footnotes:
        blocks.append(note_para(ctx, "footnotes"))
    while ctx.pending_endnotes:
        blocks.append(note_para(ctx, "endnotes"))
    while fields_left:
        fields_left -= 1
        blocks.append(para(ctx, field_run(ctx)))
    while math_left:
        math_left -= 1
        blocks.append(para(ctx, f"<m:oMath>{math_expr(ctx)}</m:oMath>"))
    while boxes_left:
        boxes_left -= 1
        blocks.append(para(ctx, f"<w:r>{textbox_xml(ctx)}</w:r>", paraid_p=0.0))
    while ctx.pending_media_refs and ctx.media:
        ctx.pending_media_refs -= 1
        _, rid = ctx.media[rng.randrange(len(ctx.media))]
        blocks.append(para(ctx, f"<w:r>{picture_xml(ctx, rid, 'image.png')}</w:r>", paraid_p=0.0))
    while bm_left:
        bm_left -= 1
        bid = ctx.next_bookmark()
        name = f"bm{bid:04d}"
        ctx.bookmark_names.append(name)
        blocks.append(para(ctx, f'<w:bookmarkStart w:id="{bid}" w:name="{name}"/>'
                           + run(ctx) + f'<w:bookmarkEnd w:id="{bid}"/>'))
    while tables_left:
        tables_left -= 1
        blocks.append(table_xml(ctx, k["table_rows"], k["table_cols"], k["table_depth"]))
    # bisect3: force-place flavour features that were EXPLICITLY enabled via
    # their sub-knob (tri-state True) but that the dice never placed — the
    # natural inline-SDT/smartTag/proofErr sites sit in a branch that styled
    # paragraphs pre-empt, so without this they can be impossible to emit.
    # Default builds (sub-knobs None) never enter here: output is unchanged.
    def _placed(marker):
        return any(marker in b for b in blocks)
    if k.get("proof_err") is True and not _placed("<w:proofErr"):
        for _ in range(3):
            blocks.append(para(ctx, '<w:proofErr w:type="spellStart"/>'
                               + run(ctx) + '<w:proofErr w:type="spellEnd"/>'))
    if k.get("inline_sdt") is True and not _placed('<w:tag w:val="stress"/>'):
        for _ in range(3):
            blocks.append(para(ctx, run(ctx, ctx.sentence() + " ")
                               + f'<w:sdt><w:sdtPr><w:id w:val="{ctx.next_sdt()}"/>'
                                 f'<w:tag w:val="stress"/><w:text/></w:sdtPr>'
                                 f'<w:sdtContent>{run(ctx, ctx.sentence(2))}'
                                 '</w:sdtContent></w:sdt>'))
    if k.get("smart_tags") is True and not _placed("<w:smartTag"):
        for _ in range(3):
            blocks.append(para(ctx, run(ctx, ctx.sentence() + " ")
                               + f'<w:smartTag w:uri="urn:stress" w:element="city">'
                                 f'{run(ctx, "Denver")}</w:smartTag>'))
    if k.get("hyperlinks") is True:
        # top up whichever hyperlink kind the dice did not place: one
        # external (r:id) and, when a bookmark target exists, one internal
        if "<w:hyperlink r:id=" not in "".join(blocks):
            rid = ctx.doc_rels.add(REL + "/hyperlink",
                                   "https://example.invalid/plus-link",
                                   external=True)
            blocks.append(para(ctx, f'<w:hyperlink r:id="{rid}" w:history="1">'
                               f'<w:r>{rpr(ctx)}<w:t>{esc(ctx.sentence(3))}</w:t></w:r>'
                               '</w:hyperlink>'))
        if "<w:hyperlink w:anchor=" not in "".join(blocks) and ctx.bookmark_names:
            bm = ctx.bookmark_names[0]
            blocks.append(para(ctx, f'<w:hyperlink w:anchor="{esca(bm)}" w:history="1">'
                               f'<w:r>{rpr(ctx)}<w:t>{esc(ctx.sentence(3))}</w:t></w:r>'
                               '</w:hyperlink>'))
    # block-level SDT (data-bound) + block customXml + chart + altChunk
    if k["block_sdt"]:
        db = (f'<w:dataBinding w:xpath="/stressRoot/item[1]" w:storeItemID="{CUSTOMXML_GUID}"/>'
              if k["customxml"] else "")
        blocks.append('<w:sdt><w:sdtPr><w:alias w:val="Stress Block"/>'
                      f'<w:tag w:val="block"/><w:id w:val="{ctx.next_sdt()}"/>'
                      + db +
                      '<w:richText/></w:sdtPr><w:sdtContent>'
                      + para(ctx, run(ctx, "Block SDT bound to the custom XML part."))
                      + "</w:sdtContent></w:sdt>")
    if k["customxml"] or k.get("body_customxml"):
        blocks.append('<w:customXml w:uri="" w:element="stressRoot">'
                      + para(ctx, run(ctx, "Wrapped in a block customXml element."))
                      + "</w:customXml>")
    if k["chart"]:
        blocks.append(para(ctx, f"<w:r>{chart_drawing(ctx, ctx.chart_rid)}</w:r>", paraid_p=0.0))
    if k["perm_ranges"]:
        bid = ctx.next_bookmark()
        blocks.append(para(ctx, f'<w:permStart w:id="{bid}" w:edGrp="everyone"/>'
                           + run(ctx, "Editable region under a permission range.")
                           + f'<w:permEnd w:id="{bid}"/>'))
    if k["altchunk"]:
        blocks.append(f'<w:altChunk r:id="{ctx.altchunk_rid}"/>')
    return blocks


DOC_ROOT_ATTRS = " ".join(
    f'xmlns:{p}="{u}"' for p, u in NS.items() if p not in ("ds", "w16cid", "mc")) \
    + f' xmlns:mc="{NS["mc"]}" mc:Ignorable="w14 wp14"'


def render_document(ctx, blocks):
    nsec = ctx.k["sections"]
    chunks = [[] for _ in range(nsec)]
    for i, b in enumerate(blocks):
        chunks[min(nsec - 1, i * nsec // max(1, len(blocks)))].append(b)
    body = []
    for si in range(nsec):
        body.extend(chunks[si])
        if si < nsec - 1:
            body.append(f"<w:p><w:pPr>{sectpr_xml(ctx, si)}</w:pPr></w:p>")
    body.append(sectpr_xml(ctx, nsec - 1, last=True))
    bg = '<w:background w:color="F7F7F7"/>' if ctx.k["background"] else ""
    return (XML_DECL + f"<w:document {DOC_ROOT_ATTRS}>{bg}<w:body>"
            + "".join(body) + "</w:body></w:document>")


def build_document(knobs, seed):
    ctx = Ctx(knobs, seed)
    # plan in dependency order ------------------------------------------------
    plan_styles(ctx)
    plan_numbering(ctx)
    plan_notes(ctx)
    plan_comments(ctx)
    plan_media(ctx)
    plan_sections(ctx)
    # fixed parts + rels ------------------------------------------------------
    ctx.doc_rels.add(REL + "/styles", "styles.xml")
    ctx.doc_rels.add(REL + "/numbering", "numbering.xml")
    ctx.doc_rels.add(REL + "/settings", "settings.xml")
    ctx.doc_rels.add(REL + "/fontTable", "fontTable.xml")
    ctx.doc_rels.add(REL + "/theme", "theme/theme1.xml")
    ctx.doc_rels.add(REL + "/footnotes", "footnotes.xml")
    ctx.doc_rels.add(REL + "/endnotes", "endnotes.xml")
    ctx.doc_rels.add(REL + "/comments", "comments.xml")
    ctx.doc_rels.add("http://schemas.microsoft.com/office/2011/relationships/commentsExtended",
                     "commentsExtended.xml")
    ctx.doc_rels.add("http://schemas.microsoft.com/office/2011/relationships/commentsIds",
                     "commentsIds.xml")
    ctx.doc_rels.add("http://schemas.microsoft.com/office/2011/relationships/people", "people.xml")
    if knobs["glossary"]:
        ctx.doc_rels.add(REL + "/glossaryDocument", "glossary/document.xml")
    if knobs["customxml"]:
        ctx.doc_rels.add(REL + "/customXml", "customXml/item1.xml")
    if knobs["chart"]:
        ctx.chart_rid = ctx.doc_rels.add(REL + "/chart", "charts/chart1.xml")
        chart_xml, xlsx = render_chart(ctx)
        ctx.add_part("word/charts/chart1.xml", chart_xml,
                     "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
                     override=True)
        ctx.add_part("word/embeddings/Microsoft_Excel_Workbook1.xlsx", xlsx,
                     "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                     override=True)
        crels = Rels()
        crels.add(REL + "/package", "../embeddings/Microsoft_Excel_Workbook1.xlsx")
        ctx.add_part("word/charts/_rels/chart1.xml.rels", crels.render())
    if knobs["altchunk"]:
        ctx.altchunk_rid = ctx.doc_rels.add(REL + "/aFChunk", "altChunk1.html")
        ctx.add_part("word/altChunk1.html",
                     "<html><body><p>Imported altChunk paragraph.</p></body></html>",
                     "text/html", override=True)
    # rendered parts -----------------------------------------------------------
    ctx.add_part("word/styles.xml", render_styles(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
                 override=True)
    ctx.add_part("word/numbering.xml", render_numbering(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
                 override=True)
    if ctx.has_pic_bullet:
        nrels = Rels()
        nrels.add(REL + "/image", "media/bullet.png")
        ctx.add_part("word/_rels/numbering.xml.rels", nrels.render())
    ctx.add_part("word/settings.xml", render_settings(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
                 override=True)
    ctx.add_part("word/fontTable.xml", render_font_table(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml",
                 override=True)
    ctx.add_part("word/theme/theme1.xml", THEME,
                 "application/vnd.openxmlformats-officedocument.theme+xml", override=True)
    ctx.add_part("word/footnotes.xml", render_notes(ctx, "footnotes"),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml",
                 override=True)
    ctx.add_part("word/endnotes.xml", render_notes(ctx, "endnotes"),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml",
                 override=True)
    ctx.add_part("word/comments.xml", render_comments(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
                 override=True)
    ctx.add_part("word/commentsExtended.xml", render_comments_extended(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml",
                 override=True)
    ctx.add_part("word/commentsIds.xml", render_comments_ids(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsIds+xml",
                 override=True)
    ctx.add_part("word/people.xml", render_people(ctx),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml",
                 override=True)
    if knobs["glossary"]:
        ctx.add_part("word/glossary/document.xml", render_glossary(ctx),
                     "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml",
                     override=True)
    if knobs["customxml"]:
        ctx.add_part("word/customXml/item1.xml",
                     '<stressRoot><item>alpha</item><item>beta</item></stressRoot>')
        ctx.add_part("word/customXml/itemProps1.xml",
                     XML_DECL + f'<ds:datastoreItem xmlns:ds="{NS["ds"]}" ds:itemID="{CUSTOMXML_GUID}">'
                     f'<ds:schemaRefs><ds:schemaRef ds:uri="{CUSTOMXML_SCHEMA_URI}"/>'
                     '</ds:schemaRefs></ds:datastoreItem>',
                     "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
                     override=True)
        irels = Rels()
        irels.add(REL + "/customXmlProps", "itemProps1.xml")
        ctx.add_part("word/customXml/_rels/item1.xml.rels", irels.render())
    core, app = render_docprops(ctx)
    ctx.add_part("docProps/core.xml", core,
                 "application/vnd.openxmlformats-package.core-properties+xml", override=True)
    ctx.add_part("docProps/app.xml", app,
                 "application/vnd.openxmlformats-officedocument.extended-properties+xml",
                 override=True)
    # body ---------------------------------------------------------------------
    blocks = build_blocks(ctx)
    ctx.add_part("word/document.xml", render_document(ctx, blocks),
                 "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
                 override=True)
    ctx.add_part("word/_rels/document.xml.rels", ctx.doc_rels.render())
    ctx.root_rels.add(REL + "/officeDocument", "word/document.xml")
    ctx.root_rels.add(PKG_REL + "/metadata/core-properties", "docProps/core.xml")
    ctx.root_rels.add(REL + "/extended-properties", "docProps/app.xml")
    ctx.add_part("_rels/.rels", ctx.root_rels.render())
    # content types --------------------------------------------------------------
    ct = [XML_DECL, f'<Types xmlns="{CT_NS}">']
    for ext, ctype in sorted(ctx.defaults.items()):
        ct.append(f'<Default Extension="{ext}" ContentType="{ctype}"/>')
    for part, ctype in sorted(ctx.overrides.items()):
        ct.append(f'<Override PartName="/{part}" ContentType="{ctype}"/>')
    ct.append("</Types>")
    ctx.parts["[Content_Types].xml"] = "".join(ct).encode("utf-8")
    return ctx.parts


def write_docx(parts, path):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for name in sorted(parts):
            zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(zi, parts[name])
    return os.path.getsize(path)


# ---------------------------------------------------------------- sanity check
R_NS = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"
MC_NS = "{http://schemas.openxmlformats.org/markup-compatibility/2006}"
XML_NS_URI = "http://www.w3.org/XML/1998/namespace"  # the xml prefix is predeclared


def audit_namespaces(label, data, result):
    """Per-part namespace audit (ECMA-376 Part 3 / MCE hygiene):
    (a) every prefix token named in an mc:Ignorable value must be declared in
        this part;
    (b) every namespace used by an element or attribute name must be declared
        in this part (a prefix used with no declaration at all cannot survive
        parsing, so the caller's parse check already covers the literal case).
    Findings are appended to result["ns_violations"]."""
    declared = {"xml": XML_NS_URI}
    used_uris = set()
    ignorable = []
    try:
        for ev, payload in ET.iterparse(io.BytesIO(data),
                                        events=("start-ns", "start")):
            if ev == "start-ns":
                declared.setdefault(payload[0], payload[1])
            else:
                for n in [payload.tag] + list(payload.attrib):
                    if n.startswith("{"):
                        used_uris.add(n[1:].split("}", 1)[0])
                ig = payload.get(MC_NS + "Ignorable")
                if ig:
                    ignorable.extend(ig.split())
    except ET.ParseError:
        return  # the caller already records the parse failure
    for tok in dict.fromkeys(ignorable):
        if tok not in declared:
            result["ns_violations"].append(
                f"{label}: mc:Ignorable names undeclared prefix '{tok}'")
    declared_uris = set(declared.values())
    for uri in sorted(used_uris):
        if uri not in declared_uris:
            result["ns_violations"].append(
                f"{label}: namespace used but never declared: {uri}")


def check_file(path):
    """Unzip, parse every XML part, resolve every r:id against its owner's rels."""
    z = zipfile.ZipFile(path)
    names = z.namelist()
    result = {"parts": len(names), "xml_parsed": 0, "refs": 0, "unresolved": [],
              "crc_bad": z.testzip(), "ct_missing": [], "ns_violations": [],
              "ct_pairing": []}
    rels_by_owner = {}
    for name in names:
        if name.endswith(".rels"):
            root = ET.fromstring(z.read(name))
            ids = {rel.get("Id") for rel in root}
            if name == "_rels/.rels":
                owner = ""
            else:
                d, _, f = name.rpartition("/_rels/")
                owner = (d + "/" + f[:-5]) if d else f[:-5]
            rels_by_owner[owner] = ids
    ct_root = ET.fromstring(z.read("[Content_Types].xml"))
    defaults = {d.get("Extension") for d in ct_root if d.tag.endswith("}Default")}
    overrides = {o.get("PartName") for o in ct_root if o.tag.endswith("}Override")}
    for name in names:
        ext = name.rsplit(".", 1)[-1] if "." in name else ""
        if ("/" + name) not in overrides and ext not in defaults and name != "[Content_Types].xml":
            result["ct_missing"].append(name)
        data = z.read(name)
        if data[:2] == b"PK":  # embedded package (a chart's .xlsx): audit inside it
            try:
                inner = zipfile.ZipFile(io.BytesIO(data))
                for m in inner.namelist():
                    if m.endswith(".xml") or m.endswith(".rels"):
                        idata = inner.read(m)
                        try:
                            ET.fromstring(idata)
                            result["xml_parsed"] += 1
                        except ET.ParseError as e:
                            result["unresolved"].append(
                                f"PARSE FAIL {name}!/{m}: {e}")
                            continue
                        audit_namespaces(f"{name}!/{m}", idata, result)
            except zipfile.BadZipFile:
                result["unresolved"].append(f"BAD EMBEDDED ZIP {name}")
            continue
        if not (name.endswith(".xml") or name.endswith(".rels")):
            continue
        try:
            root = ET.fromstring(data)
            result["xml_parsed"] += 1
        except ET.ParseError as e:
            result["unresolved"].append(f"PARSE FAIL {name}: {e}")
            continue
        audit_namespaces(name, data, result)
        if name.endswith(".rels"):
            continue
        owner_ids = rels_by_owner.get(name, set())
        for el in root.iter():
            for attr in ("id", "embed", "link"):
                v = el.get(R_NS + attr)
                if v is not None:
                    result["refs"] += 1
                    if v not in owner_ids:
                        result["unresolved"].append(f"{name}: r:{attr}={v} not in rels")
    # Pairing audit: for every part reached by a fixed-type relationship, the
    # declared content type must equal the type that relationship prescribes
    # (ECMA-376 Part 1 part catalogue). This audit exists because the generator
    # once declared the glossary part with the main document content type and
    # every consumer of the corpus (checker included) believed the manifest.
    ct_by_part = {o.get("PartName"): o.get("ContentType")
                  for o in ct_root if o.tag.endswith("}Override")}
    ct_defaults = {d.get("Extension"): d.get("ContentType")
                   for d in ct_root if d.tag.endswith("}Default")}
    rel_types = {}
    for name in names:
        if not name.endswith(".rels"):
            continue
        rroot = ET.fromstring(z.read(name))
        if name == "_rels/.rels":
            owner_dir = ""
        else:
            d, _, _f = name.rpartition("/_rels/")
            owner_dir = (d + "/") if d else ""
        for rel in rroot:
            if rel.get("TargetMode") == "External":
                continue
            tgt = rel.get("Target") or ""
            if tgt.startswith("/"):
                resolved = tgt[1:]
            else:
                segs = []
                for s in (owner_dir + tgt).split("/"):
                    if s in ("", "."):
                        continue
                    if s == "..":
                        if segs:
                            segs.pop()
                    else:
                        segs.append(s)
                resolved = "/".join(segs)
            rel_types.setdefault(resolved, set()).add(rel.get("Type") or "")
    wml = "application/vnd.openxmlformats-officedocument.wordprocessingml."
    expected_ct = {
        "/officeDocument": wml + "document.main+xml",
        "/glossaryDocument": wml + "document.glossary+xml",
        "/styles": wml + "styles+xml",
        "/numbering": wml + "numbering+xml",
        "/settings": wml + "settings+xml",
        "/fontTable": wml + "fontTable+xml",
        "/comments": wml + "comments+xml",
        "/commentsExtended": wml + "commentsExtended+xml",
        "/commentsIds": wml + "commentsIds+xml",
        "/people": wml + "people+xml",
        "/footnotes": wml + "footnotes+xml",
        "/endnotes": wml + "endnotes+xml",
        "/header": wml + "header+xml",
        "/footer": wml + "footer+xml",
        "/theme": "application/vnd.openxmlformats-officedocument.theme+xml",
        "/chart": "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
        "/customXmlProps": "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
        "/core-properties": "application/vnd.openxmlformats-package.core-properties+xml",
        "/extended-properties": "application/vnd.openxmlformats-officedocument.extended-properties+xml",
    }
    for part, types in sorted(rel_types.items()):
        if part not in names:
            continue
        eff = ct_by_part.get("/" + part)
        if eff is None:
            ext = part.rsplit(".", 1)[-1] if "." in part else ""
            eff = ct_defaults.get(ext)
        for t in sorted(types):
            for suffix, want in expected_ct.items():
                if t.endswith(suffix) and eff != want:
                    result["ct_pairing"].append(
                        f"{part}: relationship ...{suffix} prescribes {want}, "
                        f"manifest declares {eff}")
    result["ok"] = (result["crc_bad"] is None and not result["unresolved"]
                    and not result["ct_missing"] and not result["ns_violations"]
                    and not result["ct_pairing"])
    return result


# ---------------------------------------------------------------- presets / CLI
BASE_KNOBS = dict(
    paragraphs=120, rev_density=0.25, authors=3, comment_threads=8, comment_depth=2,
    abstract_nums=6, num_orphans=1, styles=40, basedon_depth=4, tables=3,
    table_rows=5, table_cols=4, table_depth=1, sections=2, media_parts=3,
    media_refs=8, media_unref=1, fields=10, textboxes=2, footnotes=6, endnotes=4,
    bookmarks=25, math=8, rsids=60, chart=True, glossary=True, altchunk=False,
    perm_ranges=True, background=False, revisions=True, latent_styles=True,
    section_extras=True, customxml=True, form_fields=True, block_sdt=True,
    flavour=True,
    # bisect3 additions — all inert at these defaults (proof of inertness:
    # corpus/minimal regeneration stays byte-identical). The four flavour
    # sub-knobs are tri-state: None means "follow the flavour master knob";
    # an explicit True additionally enables end-of-build force-placement.
    body_customxml=False, rsid_attrs=False,
    proof_err=None, inline_sdt=None, smart_tags=None, hyperlinks=None,
)

PRESETS = {
    "small-valid": dict(BASE_KNOBS,
                        paragraphs=40, rev_density=0.3, authors=2, comment_threads=3,
                        comment_depth=2, abstract_nums=3, num_orphans=1, styles=12,
                        basedon_depth=3, tables=1, table_rows=3, table_cols=3,
                        sections=2, media_parts=2, media_refs=3, fields=4,
                        textboxes=1, footnotes=3, endnotes=2, bookmarks=6, math=3,
                        rsids=20),
    "monster": dict(BASE_KNOBS,
                    paragraphs=2600, rev_density=0.55, authors=6, comment_threads=60,
                    comment_depth=5, abstract_nums=60, num_orphans=15, styles=220,
                    basedon_depth=12, tables=14, table_rows=12, table_cols=6,
                    table_depth=2, sections=5, media_parts=14, media_refs=60,
                    media_unref=4, fields=40, textboxes=8, footnotes=40, endnotes=25,
                    bookmarks=300, math=30, rsids=800, altchunk=True, background=True),
    "axis-revisions": dict(BASE_KNOBS,
                           paragraphs=900, rev_density=0.95, authors=10,
                           comment_threads=20, footnotes=15, endnotes=10),
    "axis-comments": dict(BASE_KNOBS,
                          paragraphs=500, comment_threads=220, comment_depth=8,
                          authors=8),
    "axis-numbering": dict(BASE_KNOBS,
                           paragraphs=1200, abstract_nums=300, num_orphans=60,
                           styles=80),
    "axis-styles": dict(BASE_KNOBS,
                        paragraphs=600, styles=600, basedon_depth=30),
    "axis-tables": dict(BASE_KNOBS,
                        paragraphs=300, tables=30, table_rows=40, table_cols=8,
                        table_depth=3),
    "axis-media": dict(BASE_KNOBS,
                       paragraphs=400, media_parts=40, media_refs=200, media_unref=10),
    "axis-sections": dict(BASE_KNOBS, paragraphs=600, sections=12),
    "axis-fields-math": dict(BASE_KNOBS,
                             paragraphs=600, fields=150, math=120, bookmarks=400),
}

KNOB_ARGS = [
    ("paragraphs", int), ("rev_density", float), ("authors", int),
    ("comment_threads", int), ("comment_depth", int), ("abstract_nums", int),
    ("num_orphans", int), ("styles", int), ("basedon_depth", int),
    ("tables", int), ("table_rows", int), ("table_cols", int),
    ("table_depth", int), ("sections", int), ("media_parts", int),
    ("media_refs", int), ("media_unref", int), ("fields", int),
    ("textboxes", int), ("footnotes", int), ("endnotes", int),
    ("bookmarks", int), ("math", int), ("rsids", int),
]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--preset", choices=sorted(PRESETS), help="start from a preset")
    ap.add_argument("--all", action="store_true", help="build the full corpus")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--outdir", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "corpus"))
    ap.add_argument("-o", "--out", help="single output file")
    for name, typ in KNOB_ARGS:
        ap.add_argument("--" + name.replace("_", "-"), type=typ, default=None)
    ap.add_argument("--chart", action="store_true", default=None)
    ap.add_argument("--no-chart", dest="chart", action="store_false")
    ap.add_argument("--glossary", action="store_true", default=None)
    ap.add_argument("--no-glossary", dest="glossary", action="store_false")
    ap.add_argument("--altchunk", action="store_true", default=None)
    ap.add_argument("--no-altchunk", dest="altchunk", action="store_false")
    for bname in ("revisions", "latent_styles", "section_extras", "customxml",
                  "form_fields", "block_sdt", "flavour", "perm_ranges",
                  "body_customxml", "rsid_attrs", "proof_err", "inline_sdt",
                  "smart_tags", "hyperlinks"):
        ap.add_argument("--" + bname.replace("_", "-"), dest=bname,
                        action="store_true", default=None)
        ap.add_argument("--no-" + bname.replace("_", "-"), dest=bname,
                        action="store_false")
    args = ap.parse_args(argv)

    def knobs_for(preset):
        k = dict(PRESETS[preset]) if preset else dict(BASE_KNOBS)
        for name, _ in KNOB_ARGS:
            v = getattr(args, name)
            if v is not None:
                k[name] = v
        for name in ("chart", "glossary", "altchunk", "revisions",
                     "latent_styles", "section_extras", "customxml",
                     "form_fields", "block_sdt", "flavour", "perm_ranges",
                     "body_customxml", "rsid_attrs", "proof_err", "inline_sdt",
                     "smart_tags", "hyperlinks"):
            v = getattr(args, name)
            if v is not None:
                k[name] = v
        return k

    jobs = []
    if args.all:
        order = ["monster", "axis-revisions", "axis-comments", "axis-numbering",
                 "axis-styles", "axis-tables", "axis-media", "axis-sections",
                 "axis-fields-math", "small-valid"]
        os.makedirs(args.outdir, exist_ok=True)
        for i, name in enumerate(order):
            jobs.append((name, knobs_for(name), args.seed + i,
                         os.path.join(args.outdir, name + ".docx")))
    else:
        name = args.preset or "custom"
        out = args.out or os.path.join(args.outdir, name + ".docx")
        os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
        jobs.append((name, knobs_for(args.preset), args.seed, out))

    failures = 0
    for name, knobs, seed, out in jobs:
        parts = build_document(knobs, seed)
        size = write_docx(parts, out)
        chk = check_file(out)
        status = "OK " if chk["ok"] else "FAIL"
        if not chk["ok"]:
            failures += 1
        print(f"{status} {name}: seed={seed} size={size} parts={chk['parts']} "
              f"xml_parsed={chk['xml_parsed']} r:id_refs={chk['refs']} "
              f"unresolved={len(chk['unresolved'])} ct_missing={chk['ct_missing']} "
              f"ns_violations={len(chk['ns_violations'])}")
        for u in chk["unresolved"][:10] + chk["ns_violations"][:10]:
            print("    ", u)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
