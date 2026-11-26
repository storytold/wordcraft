#!/usr/bin/env python3
"""LF-output entry point for the byte-pinned content-model extractor.

The extraction algorithm and input validation live in extract_content_model.py.
This entry point uses that shared builder and explicitly writes LF on every platform.
It accepts the same positional schema directory/output path and optional --profile argument.
"""

import json
import os
import sys

from extract_content_model import build


def main():
    """Build with the pinned extractor and serialize its result with explicit LF newlines."""
    here = os.path.dirname(os.path.abspath(__file__))
    args = sys.argv[1:]
    profile = "wml"
    if "--profile" in args:
        i = args.index("--profile")
        profile = args[i + 1]
        del args[i:i + 2]
    schema_dir = args[0] if args else os.path.join(here, "src", "strict")
    default_out = "content_model_dml.json" if profile == "dml" else "content_model.json"
    out_path = args[1] if len(args) > 1 else os.path.join(here, default_out)
    data = build(schema_dir, profile)
    with open(out_path, "w", encoding="utf-8", newline="\n") as output:
        json.dump(data, output, indent=1, sort_keys=True)
        output.write("\n")
    print(json.dumps(data["meta"]["counts"], indent=1))
    print("unresolved:", len(data["meta"]["unresolved"]))
    for unresolved in data["meta"]["unresolved"][:25]:
        print("  ", unresolved)


if __name__ == "__main__":
    main()
