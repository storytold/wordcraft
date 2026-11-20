"""Summarize OWPML element/attribute names from Hancom's Apache-2.0 hwpx-owpml-model.

Usage: python docs/hwp/tools/extract_owpml_names.py <hwpx-owpml-model>/OWPML/Class > names.md
"""
import pathlib
import re
import sys

# Namespace macro family -> XML prefix (OWPML/Base/NamespacePrefix.h)
PFX = {"CORE": "hc", "HEAD": "hh", "HISTORY": "hhs", "APPLICATION": "ha", "PARALIST": "hp", "OPF": "opf",
       "OCF": "ocf", "ODF": "odf", "HPF": "hpf", "RDF": "rdf", "ODFCONFIG": "config"}


def main(root: pathlib.Path) -> str:
    classes = {}
    for f in sorted(root.rglob("*.cpp")):
        src = f.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r"(void|bool)\s+(C\w+)::(InitMap|ReadAttribute)\s*\([^)]*\)\s*\{", src):
            cls, kind = m.group(2), m.group(3)
            nxt = re.search(r"\n\s*(?:\w[\w\*<>:,\s]*?)\s+C\w+::\w+\s*\(", src[m.end():])
            body = src[m.end(): m.end() + (nxt.start() if nxt else len(src))]
            c = classes.setdefault(cls, {"file": f.relative_to(root).as_posix(), "children": {}, "attrs": []})
            if kind == "InitMap":
                for a in re.finditer(r"OWPML_(\w+?)_ADD_REDIRECT_FUNC\d?\(\s*(\w+)\s*,\s*(\w+)\s*,", body):
                    ns, elem, child = a.groups()
                    c["children"][f"{PFX.get(ns, ns.lower())}:{elem}"] = child
            else:
                for a in re.finditer(r'GetAttribute\(\s*att\s*,\s*L"([^"]+)"', body):
                    if a.group(1) not in c["attrs"]:
                        c["attrs"].append(a.group(1))
    out = []
    for cls in sorted(classes):
        c = classes[cls]
        if not c["children"] and not c["attrs"]:
            continue
        out.append(f"### {cls}  ({c['file']})")
        if c["attrs"]:
            out.append("- attrs: " + ", ".join(c["attrs"]))
        out.extend(f"- child `{e}` -> {k}" for e, k in c["children"].items())
        out.append("")
    return "\n".join(out)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    print(main(pathlib.Path(sys.argv[1])))
