#!/usr/bin/env python3
"""Extract OOXML content-model tables from the ECMA-376 XSDs.

Reads a schema directory (the ECMA-376 Part 1 Strict set by default),
loads the five files in SCOPE_FILES, expands every complex type's
content particle (sequence/choice/all, group references, complexContent
extension) into an ordered child list with min/max occurs, collects
EVERY element declaration (in these schemas most elements are declared
locally inside complex types and EG_ groups, not globally), and inverts
the models to give each element its parent contexts.

Stdlib only. Deterministic: JSON dumped with sort_keys=True; lists are
in schema document order or sorted, never dict-iteration order.

Complex types (and, internally, groups) are keyed NAMESPACE-QUALIFIED
(``w:CT_R`` vs ``m:CT_R``), derived from the file / target namespace the
type is defined in, consistent with the element keying.  An earlier
bare-name keying silently merged same-named types from different files:
in the Strict WML set six names collide (CT_R, CT_Style, CT_OnOff,
CT_String, CT_Text, CT_TwipsMeasure -- all defined in both wml.xsd and
shared-math.xsd), so e.g. the ``CT_R`` row was the MATH definition and
the ``w:style`` row was empty (the math CT_Style) instead of the WML
style model.  Element -> type references, group references and extension
bases are resolved through each referencing file's own xmlns prefix map
to the qualified key, and child element names are qualified by the file
whose particle declares them (so a math run's model can carry both
``m:rPr`` and ``w:rPr``, in order).

The output does not depend on the schema directory's path or name: the
meta block records the schema files' SHA-256 hashes, never locations.

Usage:
    python3 extract_content_model.py [--profile wml|dml] [schema_dir] [out.json]
The commands that produce the checked-in tables are in README.md in
this directory. The dml profile runs the same machinery over
dml-main.xsd + dml-chart.xsd (prefixes a:/c:); the wml profile's output
is unchanged by the extension.
"""
import hashlib
import io
import json
import os
import sys
import xml.etree.ElementTree as ET

# Files in scope (WML + the companion vocabularies the coverage matrix
# enumerates from Annex A). dml-main.xsd is deliberately out of scope
# (referenced vocabulary, per the matrix's scope boundary); references
# into it are recorded under meta.unresolved.
SCOPE_FILES = [
    "wml.xsd",
    "shared-math.xsd",
    "shared-bibliography.xsd",
    "dml-wordprocessingDrawing.xsd",
    "dml-picture.xsd",
]

FILE_PREFIX = {
    "wml.xsd": "w",
    "shared-math.xsd": "m",
    "shared-bibliography.xsd": "b",
    "dml-wordprocessingDrawing.xsd": "wp",
    "dml-picture.xsd": "pic",
}

# Extraction profiles: the WML set above (default) and the DrawingML set.
# Same machinery, different scope; selected with --profile.
PROFILES = {
    "wml": {"scope_files": SCOPE_FILES, "file_prefix": FILE_PREFIX},
    "dml": {
        "scope_files": ["dml-main.xsd", "dml-chart.xsd"],
        "file_prefix": {"dml-main.xsd": "a", "dml-chart.xsd": "c"},
    },
}

# Family labels for the dml profile: the chart vocabulary, the theme
# sub-vocabulary of dml-main, and the rest of DrawingML.
DML_THEME = set("theme themeElements clrScheme fontScheme fmtScheme "
                "majorFont minorFont objectDefaults extraClrSchemeLst "
                "custClrLst".split())


def dml_family(prefix, name):
    if prefix == "c":
        return "charts"
    if name in DML_THEME:
        return "theme"
    return "drawingml-main"

# Priority families for a structural checker (from the task brief).
# w: element name -> family label. Companion namespaces form their own
# families, except m: which is tagged "math".
PRIORITY = {}
def _fam(label, names):
    for n in names.split():
        PRIORITY[n] = label
_fam("document-structure",
     "document body p r tbl tr tc sectPr hdr ftr footnotes footnote "
     "endnotes endnote comments comment glossaryDocument docPartBody "
     "altChunk subDoc separator continuationSeparator")
_fam("property-containers",
     "pPr rPr tblPr tblPrEx trPr tcPr docDefaults rPrDefault pPrDefault style styles")
_fam("change-snapshots",
     "pPrChange rPrChange sectPrChange trPrChange tcPrChange tblPrChange "
     "tblPrExChange tblGridChange cellIns cellDel cellMerge")
_fam("textboxes-drawings", "txbxContent pict drawing object wgp wsp graphicFrame")
_fam("fields", "fldChar fldSimple instrText ffData")
_fam("hyperlinks", "hyperlink")
_fam("bookmarks-permissions", "bookmarkStart bookmarkEnd permStart permEnd proofErr")
_fam("comments", "commentRangeStart commentRangeEnd commentReference")
_fam("notes", "footnoteReference endnoteReference footnoteRef endnoteRef annotationRef")
_fam("headers-footers", "headerReference footerReference")
_fam("numbering", "numbering abstractNum num numPr lvl lvlOverride numPicBullet abstractNumId numId ilvl")
_fam("styles", "latentStyles basedOn next link styleId")


def local(tag):
    return tag.rsplit("}", 1)[-1]


def occurs(node):
    mn = node.get("minOccurs", "1")
    mx = node.get("maxOccurs", "1")
    return (int(mn), "unbounded" if mx == "unbounded" else int(mx))


def combine(outer, inner):
    """XSD particle occurs multiplication (outer particle x inner item)."""
    mn = outer[0] * inner[0]
    mx = "unbounded" if outer[1] == "unbounded" or inner[1] == "unbounded" else outer[1] * inner[1]
    return (mn, mx)


class Schema:
    def __init__(self, schema_dir, profile="wml"):
        self.dir = schema_dir
        cfg = PROFILES[profile]
        self.scope_files = cfg["scope_files"]
        self.file_prefix = cfg["file_prefix"]
        self.types = {}        # QUALIFIED type name -> {"node", "file", "bare"}
        self.groups = {}       # QUALIFIED group name -> {"node", "file", "bare"}
        self.decls = []        # every element declaration, document order
        self.file_hashes = {}
        self.file_ns = {}      # fname -> {xmlns prefix: namespace URI}
        self.uri_to_prefix = {}  # targetNamespace URI -> canonical prefix
        self.unresolved = set()
        self._load()

    def qualify(self, qname, fname):
        """Resolve a QName as written in file `fname` to a qualified key.

        A prefixed name is resolved through that file's own xmlns map
        to its namespace URI, then to the canonical prefix for that URI
        (the defining file's prefix in this profile); if the URI is not
        one of the in-scope target namespaces, the written prefix is
        kept (e.g. ``s:ST_String``, ``a:CT_ShapeProperties`` in the wml
        profile).  An unprefixed name belongs to the file's own target
        namespace, i.e. the file's canonical prefix.
        """
        if ":" in qname:
            pfx, loc = qname.split(":", 1)
            uri = self.file_ns.get(fname, {}).get(pfx)
            canon = self.uri_to_prefix.get(uri, pfx) if uri else pfx
            return f"{canon}:{loc}"
        return f"{self.file_prefix[fname]}:{qname}"

    def _load(self):
        for fname in self.scope_files:
            path = os.path.join(self.dir, fname)
            if not os.path.exists(path):
                self.unresolved.add(f"missing file {fname}")
                continue
            with open(path, "rb") as fh:
                raw = fh.read()
            self.file_hashes[fname] = hashlib.sha256(raw).hexdigest()
            # Collect this file's xmlns prefix map (root declarations
            # are authoritative for the QName spellings used in it).
            nsmap = {}
            for _ev, ns in ET.iterparse(io.BytesIO(raw), events=("start-ns",)):
                nsmap.setdefault(ns[0], ns[1])
            self.file_ns[fname] = nsmap
            root = ET.fromstring(raw)
            prefix = self.file_prefix[fname]
            tns = root.get("targetNamespace")
            if tns:
                self.uri_to_prefix.setdefault(tns, prefix)
            for node in root.iter():
                kind = local(node.tag)
                if kind == "complexType" and node.get("name"):
                    qname = f"{prefix}:{node.get('name')}"
                    self.types[qname] = {"node": node, "file": fname,
                                         "bare": node.get("name")}
                elif kind == "group" and node.get("name"):
                    qname = f"{prefix}:{node.get('name')}"
                    self.groups[qname] = {"node": node, "file": fname,
                                          "bare": node.get("name")}
            self._collect_decls(root, prefix, fname, context=None)

    def _collect_decls(self, node, prefix, fname, context):
        kind = local(node.tag)
        if kind in ("complexType", "group") and node.get("name"):
            context = f"{prefix}:{node.get('name')}"
        if kind == "element" and (node.get("name") or node.get("ref")):
            anon = None
            for child in node:
                if local(child.tag) == "complexType":
                    anon = child
            raw_type = node.get("type")
            self.decls.append({
                "prefix": prefix,
                "name": node.get("name"),
                "ref": node.get("ref"),
                "type": raw_type,
                "type_q": self.qualify(raw_type, fname) if raw_type else None,
                "anon": anon,
                "context": context,
                "file": fname,
            })
        for child in node:
            self._collect_decls(child, prefix, fname, context)

    # ---- particle expansion ------------------------------------------------
    # Every expansion carries the file whose particle is being expanded
    # (`file_ctx`): a bare element name belongs to that file's target
    # namespace, a prefixed name/ref resolves through that file's xmlns
    # map, and a group reference switches the context to the group's
    # defining file for its own children.
    def expand_particle(self, node, file_ctx, outer=(1, 1), choice_id=None, counter=None, depth=0):
        if counter is None:
            counter = [0]
        items = []
        if node is None or depth > 60:
            return items
        kind = local(node.tag)
        occ = combine(outer, occurs(node))
        if kind == "element":
            ref = node.get("ref")
            if ref:
                qname = self.qualify(ref, file_ctx)
            elif node.get("name"):
                qname = f"{self.file_prefix[file_ctx]}:{node.get('name')}"
            else:
                qname = None
            if qname:
                items.append({"name": qname, "min": occ[0], "max": occ[1],
                              "compositor": None, "choice_group": choice_id,
                              "via_group": None})
            return items
        if kind == "group":
            ref = node.get("ref") or ""
            gqname = self.qualify(ref, file_ctx)
            grp = self.groups.get(gqname)
            if grp is None:
                self.unresolved.add(f"group {ref}")
                return items
            gname = grp["bare"]
            sub = None
            for child in grp["node"]:
                if local(child.tag) in ("sequence", "choice", "all"):
                    sub = child
            for it in self.expand_particle(sub, grp["file"], occ, choice_id, counter, depth + 1):
                if it["via_group"] is None:
                    it["via_group"] = gname
                items.append(it)
            return items
        if kind == "any":
            items.append({"name": "<any>", "min": occ[0], "max": occ[1],
                          "compositor": None, "choice_group": choice_id,
                          "via_group": None, "namespace": node.get("namespace")})
            return items
        if kind in ("sequence", "choice", "all"):
            cid = choice_id
            if kind == "choice":
                counter[0] += 1
                cid = f"choice{counter[0]}"
            for child in node:
                for it in self.expand_particle(child, file_ctx, occ, cid, counter, depth + 1):
                    if it["compositor"] is None:
                        it["compositor"] = kind
                    items.append(it)
            return items
        return items

    def expand_compositor_children(self, node, file_ctx):
        """Expand the compositor children of a complexType-like node."""
        items, kind = [], "empty"
        for child in node:
            if local(child.tag) in ("sequence", "choice", "all"):
                items = self.expand_particle(child, file_ctx)
                kind = local(child.tag)
        return items, kind

    def type_model(self, type_qname, seen=frozenset()):
        """(items, kind) for a QUALIFIED complex type; extensions merge base first."""
        if type_qname in seen:
            return [], "recursive"
        entry = self.types.get(type_qname)
        if entry is None:
            self.unresolved.add(f"type {type_qname}")
            return [], "unresolved"
        node = entry["node"]
        file_ctx = entry["file"]
        items, kind = [], "empty"
        for child in node:
            ck = local(child.tag)
            if ck in ("sequence", "choice", "all"):
                items, kind = self.expand_particle(child, file_ctx), ck
            elif ck == "complexContent":
                for cc in child:
                    if local(cc.tag) == "extension":
                        base_raw = cc.get("base") or ""
                        if base_raw:
                            base = self.qualify(base_raw, file_ctx)
                            b_items, _ = self.type_model(base, seen | {type_qname})
                            items.extend(b_items)
                        sub_items, sub_kind = self.expand_compositor_children(cc, file_ctx)
                        items.extend(sub_items)
                        if sub_items:
                            kind = sub_kind
                    elif local(cc.tag) == "restriction":
                        sub_items, sub_kind = self.expand_compositor_children(cc, file_ctx)
                        items, kind = sub_items, sub_kind
            elif ck == "simpleContent":
                return [], "simple-content"
        if node.get("mixed") == "true":
            kind = "mixed"
        return items, kind

    def anon_model(self, anon_node, file_ctx):
        return self.expand_compositor_children(anon_node, file_ctx)


def build(schema_dir, profile="wml"):
    schema = Schema(schema_dir, profile)

    # --- types layer (keyed namespace-qualified, e.g. w:CT_R / m:CT_R) -------
    types_out = {}
    for tname in sorted(schema.types):
        items, kind = schema.type_model(tname)
        types_out[tname] = {
            "file": schema.types[tname]["file"],
            "model_kind": kind,
            "children": items,
        }

    # --- elements layer (from every declaration) -----------------------------
    by_key = {}
    for d in schema.decls:
        if d["ref"] and not d["name"]:
            continue  # pure reference, not a declaration
        key = f"{d['prefix']}:{d['name']}"
        by_key.setdefault(key, []).append(d)

    elements_out = {}
    parents = {}
    for key in sorted(by_key):
        decls = by_key[key]
        prefix = decls[0]["prefix"]
        variants = {}
        contexts = {}
        for d in decls:
            if d["type"]:
                vname = d["type_q"]
            elif d["anon"] is not None:
                vname = "(anonymous)"
            else:
                vname = "(simple/unspecified)"
            contexts.setdefault(vname, set()).add(d["context"] or "(global)")
            if vname not in variants:
                if d["type"]:
                    items, kind = schema.type_model(vname)
                elif d["anon"] is not None:
                    items, kind = schema.anon_model(d["anon"], d["file"])
                else:
                    items, kind = [], "simple"
                # Child names are qualified at expansion time by the
                # file whose particle declares them, so a variant's
                # children already carry their true namespaces (the
                # v1 code re-prefixed every child with one prefix,
                # which is exactly what merged the two CT_R models).
                variants[vname] = {
                    "type": vname, "file": d["file"], "model_kind": kind,
                    "children": items,
                }
        # canonical model: the variant used in the most contexts
        canonical = max(variants.values(), key=lambda v: len(contexts[v["type"]]))
        if profile == "dml":
            fam = dml_family(prefix, key.split(":", 1)[1])
        else:
            fam = PRIORITY.get(key.split(":", 1)[1]) if prefix == "w" else ("math" if prefix == "m" else f"{prefix}-vocabulary")
        elements_out[key] = {
            "element": key,
            "family": fam,
            "declared_in": sorted({d["file"] for d in decls}),
            "type_variants": sorted(variants),
            "ambiguous_type": len(variants) > 1,
            "variant_contexts": {v: sorted(c) for v, c in sorted(contexts.items())},
            "canonical_type": canonical["type"],
            "model_kind": canonical["model_kind"],
            "parents": [],
            "children": canonical["children"],
            "models_by_variant": {v["type"]: v["children"] for v in variants.values()} if len(variants) > 1 else None,
        }
        for v in variants.values():
            for it in v["children"]:
                if it["name"] != "<any>":
                    parents.setdefault(it["name"], set()).add(key)
    for key, row in elements_out.items():
        row["parents"] = sorted(parents.get(key, set()))
        if row["models_by_variant"] is None:
            del row["models_by_variant"]

    return {
        "meta": {
            "generator": "extract_content_model.py",
            "type_keying": "namespace-qualified (v2)",
            "schema_set": "ecma-376",
            "scope_files": schema.scope_files,
            "file_sha256": schema.file_hashes,
            "counts": {
                "element_names": len(elements_out),
                "complex_types": len(types_out),
                "groups": len(schema.groups),
                "ambiguous_type_elements": sum(1 for r in elements_out.values() if r["ambiguous_type"]),
                "elements_with_children": sum(1 for r in elements_out.values() if r["children"]),
                "elements_with_parents": sum(1 for r in elements_out.values() if r["parents"]),
            },
            "unresolved": sorted(schema.unresolved),
        },
        "elements": elements_out,
        "types": types_out,
    }


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    args = sys.argv[1:]
    profile = "wml"
    if "--profile" in args:
        i = args.index("--profile")
        profile = args[i + 1]
        del args[i:i + 2]
    schema_dir = args[0] if len(args) > 0 else os.path.join(here, "src", "strict")
    default_out = "content_model_dml.json" if profile == "dml" else "content_model.json"
    out_path = args[1] if len(args) > 1 else os.path.join(here, default_out)
    data = build(schema_dir, profile)
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=1, sort_keys=True)
        fh.write("\n")
    print(json.dumps(data["meta"]["counts"], indent=1))
    print("unresolved:", len(data["meta"]["unresolved"]))
    for u in data["meta"]["unresolved"][:25]:
        print("  ", u)


if __name__ == "__main__":
    main()
