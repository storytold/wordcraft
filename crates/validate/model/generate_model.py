#!/usr/bin/env python3
"""Generate ``crates/validate/src/model.rs`` from the checked-in content-model JSON.

The chain is XSD -> JSON (``extract_content_model.py``) -> Rust (this script).
The Rust file is checked in; this script is the only thing that writes it.
See ``README.md`` in this directory for the full regeneration chain.

Inputs (all in ``--model-dir``):

- ``content_model_v2.json``                    WML tables, Strict
- ``content_model_dml_v2.json``                DML tables, Strict
- ``content_model_v2_transitional.json``       WML tables, Transitional
- ``content_model_dml_v2_transitional.json``   DML tables, Transitional

plus the Strict schema set in ``--xsd-dir``: ``wml.xsd`` gives the
``ST_NumberFormat`` enumeration wholesale, and the ``xsd:group ref`` sites of
the extractor's in-scope ``*.xsd`` files there confirm the group contexts that no table type flattens
(see ``check_unflattened_group``).

What it emits:

- ``TYPES``: every Strict complex type of both profiles, merged into one
  table keyed by qualified name and sorted by key.
- ``TRANSITIONAL_TYPES``: only the types whose children differ between the
  Strict and Transitional runs (the dual-check veto models).
- ``ELEMENTS``: every Strict element row of both profiles, sorted by name,
  each with its type variants; each variant's parent contexts are expanded
  to parent TYPE keys (group contexts and global declarations resolved).
- ``NUM_FMT_VALUES``: the ``ST_NumberFormat`` enumeration in XSD order.

The script fails (exit status 1, message on stderr) whenever an expected
fact about the tables stops being true, rather than emitting a drifted
table. Stdlib only. Deterministic: the output depends only on the input
bytes, never on dict or set iteration order, time, or paths.

Usage:
    python3 generate_model.py --model-dir . --xsd-dir <strict-xsd-dir> --out ../src/model.rs
"""
import argparse
import hashlib
import json
import os
import re
import sys
import xml.etree.ElementTree as ET

XSD_NS = "{http://www.w3.org/2001/XMLSchema}"

STRICT_WML = "content_model_v2.json"
STRICT_DML = "content_model_dml_v2.json"
TRANS_WML = "content_model_v2_transitional.json"
TRANS_DML = "content_model_dml_v2_transitional.json"

# Expected table sizes (EXTRACTION.md v2, EXTRACTION-DML.md v2 section).
EXPECTED = {
    STRICT_WML: {"element_names": 794, "complex_types": 380},
    STRICT_DML: {"element_names": 541, "complex_types": 367},
}

# Types whose children differ between Strict and Transitional
# (EXTRACTION.md v2: 19 qualified WML rows; EXTRACTION-DML.md: 1 DML row).
EXPECTED_TRANSITIONAL_WML = 19
EXPECTED_TRANSITIONAL_DML = ["c:CT_PrintSettings"]

# The bare-name collision families that v2 keying separates; each must
# exist as its own qualified row on both sides.
COLLISION_FAMILIES = [
    ("w:CT_R", "m:CT_R"),
    ("w:CT_Style", "m:CT_Style"),
    ("w:CT_OnOff", "m:CT_OnOff"),
    ("w:CT_String", "m:CT_String"),
    ("w:CT_Text", "m:CT_Text"),
    ("w:CT_TwipsMeasure", "m:CT_TwipsMeasure"),
    ("a:CT_Boolean", "c:CT_Boolean"),
]

# Part roots the grammar scope uses (BRIEF-pr2 3.2). Each must be a row
# with exactly one type variant, so root model selection needs no parent.
PART_ROOTS = [
    "w:document", "w:glossaryDocument", "w:comments", "w:footnotes",
    "w:endnotes", "w:hdr", "w:ftr", "w:numbering", "w:styles",
    "w:settings", "w:fonts", "w:webSettings", "b:Sources",
    "c:chartSpace", "a:theme",
]

# The extractor's in-scope schema files and their key prefixes, both profiles
# (extract_content_model.py: FILE_PREFIX and PROFILES["dml"]).
SCOPE_PREFIX = {
    "wml.xsd": "w",
    "shared-math.xsd": "m",
    "shared-bibliography.xsd": "b",
    "dml-wordprocessingDrawing.xsd": "wp",
    "dml-picture.xsd": "pic",
    "dml-main.xsd": "a",
    "dml-chart.xsd": "c",
}

COMPOSITORS = {"sequence": "Sequence", "choice": "Choice", "all": "All"}

# Names and keys are emitted inside Rust string literals: allow only the
# characters the tables actually use, so no escaping is ever needed.
SAFE = re.compile(r"^[A-Za-z0-9_:.<>\-]+$")

U16_MAX = 0xFFFF


class GenError(Exception):
    """A fact the generator relies on is not true of its inputs."""


def check(cond, message):
    """Raise GenError with `message` unless `cond` holds."""
    if not cond:
        raise GenError(message)


def sha256(path):
    """Hex SHA-256 of the file at `path`."""
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def load(model_dir, name):
    """Load one table JSON and check it is a v2 (qualified-key) run."""
    path = os.path.join(model_dir, name)
    with open(path, encoding="utf-8") as fh:
        data = json.load(fh)
    keying = data.get("meta", {}).get("type_keying")
    check(keying == "namespace-qualified (v2)",
          f"{name}: meta.type_keying is {keying!r}, expected the v2 qualified keying")
    for k in ("elements", "types"):
        check(isinstance(data.get(k), dict), f"{name}: missing '{k}' table")
    return data


def check_counts(name, data):
    """Assert a Strict table has its recorded element and type counts."""
    want = EXPECTED[name]
    got = {"element_names": len(data["elements"]), "complex_types": len(data["types"])}
    check(got == want, f"{name}: counts {got}, expected {want}")


def safe(s, what):
    """Return `s` after checking it can sit in a Rust literal unescaped."""
    check(isinstance(s, str) and SAFE.match(s) is not None, f"unexpected characters in {what}: {s!r}")
    return s


def merge(name_a, table_a, name_b, table_b, what):
    """Merge two keyed tables; a shared key must carry identical data."""
    out = dict(table_a)
    for key, row in table_b.items():
        if key in out:
            check(out[key] == row, f"{what} {key} is in {name_a} and {name_b} with different content")
        out[key] = row
    return out


def child_signature(children):
    """Canonical comparable form of a type's child list."""
    return json.dumps(children, sort_keys=True)


def transitional_overrides(strict, trans, name):
    """Qualified keys of Strict types whose Transitional children differ."""
    out = []
    for key in sorted(strict):
        t = trans.get(key)
        check(t is not None, f"{name}: Strict type {key} is missing from the Transitional run")
        if child_signature(strict[key]["children"]) != child_signature(t["children"]):
            out.append(key)
    return out


def num_fmt_values(xsd_dir):
    """The ST_NumberFormat enumeration values from wml.xsd, in XSD order."""
    path = os.path.join(xsd_dir, "wml.xsd")
    root = ET.parse(path).getroot()
    for st in root.iter(f"{XSD_NS}simpleType"):
        if st.get("name") == "ST_NumberFormat":
            values = [e.get("value") for e in st.iter(f"{XSD_NS}enumeration")]
            check(values and all(values), "ST_NumberFormat has an empty or missing enumeration value")
            check(len(set(values)) == len(values), "ST_NumberFormat has a duplicate value")
            return [safe(v, "ST_NumberFormat value") for v in values], sha256(path)
    raise GenError(f"no ST_NumberFormat simple type in {path}")


def group_bare(context):
    """Bare group name of a qualified group context (`w:EG_X` -> `EG_X`)."""
    return context.split(":", 1)[1] if ":" in context else context


def group_refs(xsd_dir):
    """Map bare group name -> sorted qualified keys of the complex types that reference it.

    Scans the extractor's in-scope schema files (`SCOPE_PREFIX`, both
    profiles; the same file -> prefix mapping, so keys match the tables) for
    ``xsd:group ref`` particles and records the nearest enclosing named
    ``xsd:complexType``. References from inside another group are recorded
    under ``(group)``, so they are never mistaken for a type's content.
    """
    refs = {}
    for fname, prefix in sorted(SCOPE_PREFIX.items()):
        path = os.path.join(xsd_dir, fname)
        check(os.path.isfile(path), f"in-scope schema {fname} is missing from {xsd_dir}")
        root = ET.parse(path).getroot()
        stack = [(root, None)]
        while stack:
            node, owner = stack.pop()
            if node.tag == f"{XSD_NS}complexType" and node.get("name"):
                owner = f"{prefix}:{node.get('name')}"
            elif node.tag == f"{XSD_NS}group" and node.get("name"):
                owner = "(group)"
            elif node.tag == f"{XSD_NS}group" and node.get("ref"):
                refs.setdefault(group_bare(node.get("ref")), set()).add(owner or "(none)")
            stack.extend((child, owner) for child in node)
    return {g: sorted(owners) for g, owners in refs.items()}


def check_unflattened_group(ctx, types, refs):
    """Accept a group context that no type in the tables flattens, or fail.

    The extractor flattens a group into a type only when the group sits inside
    the type's ``sequence``/``choice``. Two shapes leave a group with no slot
    in any type: the group is never referenced by a complex type (DML
    ``EG_Media``, ``EG_TextGeometry``), or the group is the type's whole
    content model as a bare top-level particle, which the extractor records
    as an ``empty`` type (``w:CT_RubyContent`` over ``EG_RubyContent``,
    ``a:CT_EffectContainer`` over ``EG_Effect``). In both shapes the context
    names no parent type that the tables hold a slot for, so it contributes
    no parents. Anything else (a referencing type with children, or an
    unknown type) is a real inconsistency and fails.

    Returns the sorted qualified keys of the empty types whose content is the
    group (the tables do not carry their content).
    """
    bare = group_bare(ctx)
    flattened = sorted(t for t, row in types.items() for c in row["children"] if c.get("via_group") == bare)
    check(not flattened, f"group context {ctx}: group is flattened into {flattened} but not for this element")
    opaque = []
    for owner in refs.get(bare, []):
        check(owner != "(none)", f"group context {ctx}: referenced outside any complex type or group")
        if owner == "(group)":
            # Referenced from another group: that group's own contexts carry it.
            continue
        row = types.get(owner)
        check(row is not None, f"group context {ctx}: referencing type {owner} has no row in the tables")
        check(row.get("model_kind") == "empty" and row["children"] == [],
              f"group context {ctx}: referencing type {owner} is not an empty row, yet holds no slot from the group")
        opaque.append(owner)
    return sorted(set(opaque))


def expand_contexts(elements, types, refs):
    """Expand each element variant's contexts to parent type keys.

    A context that is a type key stays as it is. A group context expands to
    every type whose flattened child list has a slot for this element that
    came from that group (`via_group`). An empty expansion is accepted only
    when no type flattens the group at all and every type referencing it in
    the XSD is an empty row (see `check_unflattened_group`); otherwise it
    fails. A global declaration expands to every type with a slot for this
    element that no other variant of the element claims. A (parent type,
    element) pair claimed by two variants is ambiguous: it is removed from
    every variant, so the consumer falls back to union membership there.

    Returns ({element: {variant: sorted parent types}}, sorted ambiguous pairs,
    {unflattened group context: sorted empty types whose content it is}).
    """
    # (element name) -> list of (type key, via_group) for every slot
    slots = {}
    for tkey in sorted(types):
        for c in types[tkey]["children"]:
            slots.setdefault(c["name"], []).append((tkey, c.get("via_group")))

    expanded = {}
    ambiguous = []
    unflattened = {}
    for ename in sorted(elements):
        row = elements[ename]
        contexts = row["variant_contexts"]
        claimed = {}
        globals_ = []
        for variant in sorted(contexts):
            got = set()
            for ctx in contexts[variant]:
                if ctx == "(global)":
                    globals_.append(variant)
                elif ctx in types:
                    got.add(ctx)
                else:
                    bare = group_bare(ctx)
                    hits = {t for t, via in slots.get(ename, []) if via == bare}
                    if not hits:
                        unflattened[ctx] = check_unflattened_group(ctx, types, refs)
                    got |= hits
            claimed[variant] = got
        explicit = set().union(*claimed.values()) if claimed else set()
        for variant in globals_:
            claimed[variant] |= {t for t, _ in slots.get(ename, []) if t not in explicit}
        owners = {}
        for variant, parents in claimed.items():
            for p in parents:
                owners.setdefault(p, []).append(variant)
        clash = sorted(p for p, vs in owners.items() if len(vs) > 1)
        for p in clash:
            ambiguous.append((p, ename))
            for variant in claimed:
                claimed[variant].discard(p)
        expanded[ename] = {v: sorted(ps) for v, ps in claimed.items()}
    return expanded, sorted(ambiguous), dict(sorted(unflattened.items()))


def rust_max(value):
    """Rust `Option<u16>` literal for an occurs maximum (None = unbounded)."""
    if value == "unbounded":
        return "None"
    check(isinstance(value, int) and 0 <= value <= U16_MAX, f"maxOccurs {value!r} out of range")
    return f"Some({value})"


def rust_children(children, what):
    """Rust lines for one type's child slots."""
    lines = []
    check(len(children) <= U16_MAX, f"{what}: too many children for a u16 position")
    for pos, c in enumerate(children):
        name = safe(c["name"], f"{what} child name")
        mn = c["min"]
        check(isinstance(mn, int) and 0 <= mn <= U16_MAX, f"{what}: minOccurs {mn!r} out of range")
        comp = COMPOSITORS.get(c.get("compositor"))
        check(comp is not None, f"{what}: child {name} has compositor {c.get('compositor')!r}")
        group = c.get("choice_group")
        group_lit = "None" if group is None else f'Some("{safe(group, what + " choice group")}")'
        lines.append(f'        ch("{name}", {mn}, {rust_max(c["max"])}, {comp}, {group_lit}, {pos}),')
    return lines


def used_compositors(type_rows):
    """Sorted Rust variant names of every compositor the given type rows use."""
    used = {COMPOSITORS.get(c.get("compositor")) for row in type_rows for c in row["children"]}
    used.discard(None)
    check(used, "no compositor is used by any type")
    return sorted(used)


def use_line(names):
    """The `use Compositor::...;` line, in the form rustfmt leaves unchanged."""
    if len(names) == 1:
        return f"use Compositor::{names[0]};"
    return "use Compositor::{" + ", ".join(names) + "};"


def rust_types(name, types):
    """Rust source for one `&[TypeModel]` static."""
    out = ["#[rustfmt::skip]", f"pub(crate) static {name}: &[TypeModel] = &["]
    for key in sorted(types):
        children = types[key]["children"]
        safe(key, "type key")
        if not children:
            out.append(f'    ty("{key}", &[]),')
            continue
        out.append(f'    ty("{key}", &[')
        out.extend(rust_children(children, key))
        out.append("    ]),")
    out.append("];")
    return out


def rust_elements(elements, expanded):
    """Rust source for the `ELEMENTS` static."""
    out = ["#[rustfmt::skip]", "pub(crate) static ELEMENTS: &[ElementModel] = &["]
    for ename in sorted(elements):
        row = elements[ename]
        canonical = safe(row["canonical_type"], f"{ename} canonical type")
        variants = expanded[ename]
        check(canonical in variants, f"{ename}: canonical type {canonical} is not one of its variants")
        out.append(f'    el("{safe(ename, "element name")}", "{canonical}", &[')
        for variant in sorted(variants):
            ctx = ", ".join(f'"{safe(p, "context type")}"' for p in variants[variant])
            out.append(f'        va("{safe(variant, "variant type")}", &[{ctx}]),')
        out.append("    ]),")
    out.append("];")
    return out


PRELUDE = '''\
/// How many times a child may occur in its slot: a finite bound or `unbounded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaxOccurs {
    /// At most this many occurrences.
    Limited(u16),
    /// No upper bound (`maxOccurs="unbounded"`).
    Unbounded,
}

/// The XSD compositor a child slot sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compositor {
    /// `xsd:sequence`: slots in a fixed order.
    Sequence,
    /// `xsd:choice`: one of the slots sharing a `choice_group`.
    Choice,
    /// `xsd:all`: each slot at most once, any order.
    All,
}

/// One child slot in a type's flattened content model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildSpec {
    /// Prefixed element name, e.g. `w:tblPr`; `<any>` for an `xsd:any` wildcard.
    pub name: &'static str,
    /// Minimum occurrences, with the enclosing particles' minOccurs multiplied in.
    pub min: u16,
    /// Maximum occurrences, with the enclosing particles' maxOccurs multiplied in.
    pub max: MaxOccurs,
    /// The compositor this slot sits in.
    pub compositor: Compositor,
    /// The choice group id shared by the members of one `xsd:choice`, if any.
    pub choice_group: Option<&'static str>,
    /// Index of this slot in the type's flattened child list.
    pub position: u16,
}

/// The content model of one complex type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TypeModel {
    /// Namespace-qualified type key, e.g. `w:CT_Tc`, `a:CT_FontCollection`.
    pub key: &'static str,
    /// The type's child slots in flattened schema order.
    pub children: &'static [ChildSpec],
}

/// One type an element name is declared with, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Variant {
    /// The type key of this declaration (may name a simple or out-of-scope type
    /// that has no [`TypeModel`]; such an element is unmodelled).
    pub type_key: &'static str,
    /// Parent type keys this variant is selected under (group contexts expanded).
    pub contexts: &'static [&'static str],
}

/// An element name and every type it is declared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ElementModel {
    /// Prefixed element name, e.g. `w:sectPr`.
    pub name: &'static str,
    /// The type key used in the most declaration contexts.
    pub canonical: &'static str,
    /// Every declared type, sorted by type key.
    pub variants: &'static [Variant],
}

/// Builds one child slot; a `max` of `None` means unbounded. Keeps table rows short.
const fn ch(name: &'static str, min: u16, max: Option<u16>, compositor: Compositor, group: Option<&'static str>, position: u16) -> ChildSpec {
    let max = match max {
        Some(n) => MaxOccurs::Limited(n),
        None => MaxOccurs::Unbounded,
    };
    ChildSpec { name, min, max, compositor, choice_group: group, position }
}

/// Builds one type row.
const fn ty(key: &'static str, children: &'static [ChildSpec]) -> TypeModel {
    TypeModel { key, children }
}

/// Builds one element variant.
const fn va(type_key: &'static str, contexts: &'static [&'static str]) -> Variant {
    Variant { type_key, contexts }
}

/// Builds one element row.
const fn el(name: &'static str, canonical: &'static str, variants: &'static [Variant]) -> ElementModel {
    ElementModel { name, canonical, variants }
}

/// The Strict model of a type, by qualified key (binary search over [`TYPES`]).
pub(crate) fn type_model(key: &str) -> Option<&'static TypeModel> {
    TYPES.binary_search_by(|t| t.key.cmp(key)).ok().and_then(|i| TYPES.get(i))
}

/// The Transitional model of a type, only where it differs from the Strict one.
pub(crate) fn transitional_type_model(key: &str) -> Option<&'static TypeModel> {
    TRANSITIONAL_TYPES.binary_search_by(|t| t.key.cmp(key)).ok().and_then(|i| TRANSITIONAL_TYPES.get(i))
}

/// The row for a prefixed element name (binary search over [`ELEMENTS`]).
pub(crate) fn element_model(prefixed_name: &str) -> Option<&'static ElementModel> {
    ELEMENTS.binary_search_by(|e| e.name.cmp(prefixed_name)).ok().and_then(|i| ELEMENTS.get(i))
}
'''


def generate(model_dir, xsd_dir):
    """Build the full text of model.rs from the inputs."""
    sw, sd = load(model_dir, STRICT_WML), load(model_dir, STRICT_DML)
    tw, td = load(model_dir, TRANS_WML), load(model_dir, TRANS_DML)
    check_counts(STRICT_WML, sw)
    check_counts(STRICT_DML, sd)

    types = merge(STRICT_WML, sw["types"], STRICT_DML, sd["types"], "type")
    elements = merge(STRICT_WML, sw["elements"], STRICT_DML, sd["elements"], "element")
    check(len(types) == 380 + 367, f"merged type table has {len(types)} rows, expected 747")
    check(len(elements) == 794 + 541, f"merged element table has {len(elements)} rows, expected 1335")
    for a, b in COLLISION_FAMILIES:
        check(a in types and b in types, f"collision family {a} / {b} is not two separate rows")
    for k in list(types) + list(elements):
        check(k.isascii(), f"non-ASCII key {k!r} (sort order must match Rust byte order)")

    over_wml = transitional_overrides(sw["types"], tw["types"], TRANS_WML)
    over_dml = transitional_overrides(sd["types"], td["types"], TRANS_DML)
    check(len(over_wml) == EXPECTED_TRANSITIONAL_WML,
          f"{len(over_wml)} WML types differ in Transitional, expected {EXPECTED_TRANSITIONAL_WML}: {over_wml}")
    check(over_dml == EXPECTED_TRANSITIONAL_DML,
          f"DML Transitional differences {over_dml}, expected {EXPECTED_TRANSITIONAL_DML}")
    trans_types = {k: tw["types"][k] for k in over_wml}
    trans_types.update({k: td["types"][k] for k in over_dml})

    for root in PART_ROOTS:
        row = elements.get(root)
        check(row is not None, f"part root {root} has no element row")
        check(len(row["variant_contexts"]) == 1, f"part root {root} has more than one type variant")

    expanded, ambiguous, unflattened = expand_contexts(elements, types, group_refs(xsd_dir))
    numfmt, wml_xsd_hash = num_fmt_values(xsd_dir)

    hashes = [(n, sha256(os.path.join(model_dir, n))) for n in (STRICT_WML, STRICT_DML, TRANS_WML, TRANS_DML)]
    hashes.append(("wml.xsd (Strict)", wml_xsd_hash))
    multi = sum(1 for r in elements.values() if len(r["variant_contexts"]) > 1)

    head = [
        "//! ECMA-376 content-model tables for the grammar checks.",
        "//!",
        "//! GENERATED FILE: do not edit by hand. Regenerate from `crates/validate/model/` with",
        "//!",
        "//! ```text",
        "//! python3 generate_model.py --model-dir . --xsd-dir <strict-xsd-dir> --out ../src/model.rs",
        "//! ```",
        "//!",
        "//! The chain from the ECMA-376 schema archives to this file is documented in",
        "//! `crates/validate/model/README.md`. Inputs (SHA-256):",
        "//!",
    ]
    head += [f"//! - `{n}`: `{h}`" for n, h in hashes]
    head += [
        "//!",
        f"//! Counts: {len(types)} Strict types ({len(sw['types'])} WML + {len(sd['types'])} DML); "
        f"{len(trans_types)} Transitional overrides ({len(over_wml)} WML + {len(over_dml)} DML);",
        f"//! {len(elements)} elements ({len(sw['elements'])} WML + {len(sd['elements'])} DML), "
        f"{multi} with more than one type variant; {len(ambiguous)} ambiguous (parent type, element)",
        f"//! pairs left to fallback selection; {len(numfmt)} `ST_NumberFormat` values.",
        "//!",
        "//! Group contexts that no type flattens contribute no parent types. Where such a group is a",
        "//! type's whole content, the extractor records that type as empty: these tables do not carry",
        "//! its content, so a consumer must not read its empty child list as \"no children allowed\".",
    ]
    for ctx, opaque in unflattened.items():
        detail = ", ".join(f"`{k}`" for k in opaque) if opaque else "referenced by no in-scope complex type"
        head.append(f"//! - `{ctx}`: {detail}")
    head += [
        "",
        # Import only the compositors the tables use: an unused import fails clippy -D warnings.
        use_line(used_compositors(list(types.values()) + list(trans_types.values()))),
        "",
    ]
    body = PRELUDE.split("\n")
    out = head + body
    out.append("/// Every Strict complex type of the WML and DML profiles, sorted by key.")
    out += rust_types("TYPES", types)
    out.append("")
    out.append("/// Transitional models of the types whose children differ from Strict, sorted by key.")
    out += rust_types("TRANSITIONAL_TYPES", trans_types)
    out.append("")
    out.append("/// Every Strict element row of the WML and DML profiles, sorted by name.")
    out += rust_elements(elements, expanded)
    out.append("")
    out.append(f"/// The `ST_NumberFormat` enumeration of `wml.xsd` (Strict), all {len(numfmt)} values in XSD order.")
    out.append("#[rustfmt::skip]")
    out.append("pub(crate) static NUM_FMT_VALUES: &[&str] = &[")
    out += [f'    "{v}",' for v in numfmt]
    out.append("];")
    text = "\n".join(out).rstrip("\n") + "\n"
    return text, ambiguous, unflattened


def main():
    """Parse arguments, generate, write the output, report the summary."""
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--model-dir", required=True, help="directory holding the four table JSON files")
    ap.add_argument("--xsd-dir", required=True, help="the ECMA-376 Strict schema directory (holds wml.xsd)")
    ap.add_argument("--out", required=True, help="path of the model.rs to write")
    args = ap.parse_args()
    try:
        text, ambiguous, unflattened = generate(args.model_dir, args.xsd_dir)
    except GenError as e:
        print(f"generate_model.py: {e}", file=sys.stderr)
        return 1
    with open(args.out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(text)
    print(f"wrote {args.out} ({len(text.encode('utf-8'))} bytes, sha256 {hashlib.sha256(text.encode('utf-8')).hexdigest()})")
    print(f"ambiguous (parent type, element) pairs left to fallback: {len(ambiguous)}")
    for p, e in ambiguous:
        print(f"  {p} > {e}")
    print(f"group contexts no type flattens (no parents contributed): {len(unflattened)}")
    for ctx, opaque in unflattened.items():
        print(f"  {ctx}: empty content types {opaque}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
