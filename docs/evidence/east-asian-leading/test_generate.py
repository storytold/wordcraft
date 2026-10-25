"""Check generated XML property order and required ECMA-376 page margins."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from xml.etree import ElementTree
from zipfile import ZipFile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
PPR_ORDER = ("snapToGrid", "spacing", "rPr")


class GeneratorOrder(unittest.TestCase):
    def test_generated_xml_contract(self):
        generator = Path(__file__).with_name("generate.py")
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "controls"
            subprocess.run([sys.executable, str(generator), str(output)], check=True, capture_output=True)
            documents = sorted(output.glob("*.docx"))
            self.assertEqual(len(documents), 3)
            for document in documents:
                with ZipFile(document) as archive:
                    root = ElementTree.fromstring(archive.read("word/document.xml"))
                margins = root.find(f".//{{{W}}}pgMar")
                self.assertIsNotNone(margins)
                for attribute in ("header", "footer", "gutter"):
                    self.assertEqual(margins.get(f"{{{W}}}{attribute}"), "0")
                properties = root.findall(f".//{{{W}}}pPr")
                self.assertTrue(properties)
                for index, props in enumerate(properties):
                    with self.subTest(document=document.name, paragraph=index):
                        order = tuple(child.tag.removeprefix(f"{{{W}}}") for child in props)
                        self.assertEqual(order, PPR_ORDER)


if __name__ == "__main__":
    unittest.main()
