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

if __name__ == "__main__":
    unittest.main()
