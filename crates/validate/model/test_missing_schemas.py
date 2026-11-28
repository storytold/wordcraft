"""Missing required schemas must fail before either CLI touches output."""
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCOPES = {
    "wml": ["wml.xsd", "shared-math.xsd", "shared-bibliography.xsd",
            "dml-wordprocessingDrawing.xsd", "dml-picture.xsd"],
    "dml": ["dml-main.xsd", "dml-chart.xsd"],
}
SCHEMA = '<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"/>'

class MissingSchemaTests(unittest.TestCase):
    def test_missing_inputs_preserve_output(self):
        for entry in ["extract_content_model.py", "extract_model_lf.py"]:
            for profile, files in SCOPES.items():
                for missing in [None, *files]:
                    for existing in [False, True]:
                        with self.subTest(entry=entry, profile=profile, missing=missing, existing=existing):
                            with tempfile.TemporaryDirectory() as tmp:
                                root = pathlib.Path(tmp)
                                source = root / "schemas"
                                if missing is not None:
                                    source.mkdir()
                                    for name in files:
                                        if name != missing:
                                            (source / name).write_text(SCHEMA)
                                output = root / "model.json"
                                if existing:
                                    output.write_bytes(b"preserve existing output")
                                result = subprocess.run(
                                    [sys.executable, str(HERE / entry), str(source),
                                     str(output), "--profile", profile],
                                    capture_output=True, text=True, check=False)
                                self.assertNotEqual(result.returncode, 0, result.stdout)
                                self.assertIn(missing or files[0], result.stderr)
                                if existing:
                                    self.assertEqual(output.read_bytes(), b"preserve existing output")
                                else:
                                    self.assertFalse(output.exists())

    def test_complete_inputs_still_extract(self):
        for entry in ["extract_content_model.py", "extract_model_lf.py"]:
            for profile, files in SCOPES.items():
                with self.subTest(entry=entry, profile=profile), tempfile.TemporaryDirectory() as tmp:
                    root = pathlib.Path(tmp)
                    for name in files:
                        (root / name).write_text(SCHEMA)
                    result = subprocess.run(
                        [sys.executable, str(HERE / entry), str(root),
                         str(root / "out.json"), "--profile", profile],
                        capture_output=True, text=True, check=False)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertTrue((root / "out.json").is_file())

    def test_extension_choice_group_ids_do_not_collide_with_base(self):
        sys.path.insert(0, str(HERE))
        import extract_content_model as ecm

        wml_xsd = """\
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
           xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <xs:complexType name="CT_Base">
    <xs:choice>
      <xs:element name="baseA" type="xs:string"/>
      <xs:element name="baseB" type="xs:string"/>
    </xs:choice>
  </xs:complexType>
  <xs:complexType name="CT_Ext">
    <xs:complexContent>
      <xs:extension base="w:CT_Base">
        <xs:choice>
          <xs:element name="extX" type="xs:string"/>
          <xs:element name="extY" type="xs:string"/>
        </xs:choice>
      </xs:extension>
    </xs:complexContent>
  </xs:complexType>
</xs:schema>
"""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name in SCOPES["wml"]:
                (root / name).write_text(wml_xsd if name == "wml.xsd" else SCHEMA)
            built = ecm.build(str(root), profile="wml")
        children = built["types"]["w:CT_Ext"]["children"]
        by_name = {c["name"]: c["choice_group"] for c in children}
        self.assertEqual(by_name["w:baseA"], by_name["w:baseB"])
        self.assertEqual(by_name["w:extX"], by_name["w:extY"])
        self.assertIsNotNone(by_name["w:baseA"])
        self.assertIsNotNone(by_name["w:extX"])
        self.assertNotEqual(by_name["w:baseA"], by_name["w:extX"])

    def test_generator_rejects_empty_named_owner_and_partial_group_coverage(self):
        sys.path.insert(0, str(HERE))
        import generate_model as gm

        # 1. Stale empty row for a named direct owner must fail check_unflattened_group.
        types_empty_owner = {"w:CT_RubyContent": {"model_kind": "empty", "children": []}}
        refs_ruby = {"EG_RubyContent": ["w:CT_RubyContent"]}
        with self.assertRaises(gm.GenError):
            gm.check_unflattened_group("w:EG_RubyContent", types_empty_owner, refs_ruby)

        # Unreferenced group with no named complex-type owner is still accepted.
        self.assertEqual(gm.check_unflattened_group("a:EG_Media", {}, {}), [])

        # 2. Partial per-owner coverage (one owner flattened, another missing) must fail expand_contexts.
        elements = {
            "a:noFill": {
                "variant_contexts": {"a:CT_NoFillProperties": ["a:EG_FillProperties"]},
            }
        }
        refs_multi = {"EG_FillProperties": ["a:CT_FillEffect", "a:CT_TextUnderlineFillGroupWrapper"]}
        types_partial = {
            "a:CT_FillEffect": {
                "model_kind": "sequence",
                "children": [{"name": "a:noFill", "via_group": "EG_FillProperties"}],
            },
            "a:CT_TextUnderlineFillGroupWrapper": {"model_kind": "empty", "children": []},
        }
        with self.assertRaises(gm.GenError):
            gm.expand_contexts(elements, types_partial, refs_multi)

        # When every direct owner supplies its slot, expand_contexts succeeds and lists all owners.
        types_complete = {
            "a:CT_FillEffect": {
                "model_kind": "sequence",
                "children": [{"name": "a:noFill", "via_group": "EG_FillProperties"}],
            },
            "a:CT_TextUnderlineFillGroupWrapper": {
                "model_kind": "group",
                "children": [{"name": "a:noFill", "via_group": "EG_FillProperties"}],
            },
        }
        expanded, ambiguous, unflattened = gm.expand_contexts(elements, types_complete, refs_multi)
        self.assertEqual(
            expanded["a:noFill"]["a:CT_NoFillProperties"],
            ["a:CT_FillEffect", "a:CT_TextUnderlineFillGroupWrapper"],
        )
        self.assertEqual(ambiguous, [])
        self.assertEqual(unflattened, {})

if __name__ == "__main__":
    unittest.main()
