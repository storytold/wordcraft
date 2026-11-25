#!/usr/bin/env python3
"""The committed Nightly icons are exactly what the committed derivation produces."""
import hashlib
import importlib.util
from pathlib import Path
import json
import struct
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
ICONS = ROOT / "assets" / "app-icon"
SPEC = importlib.util.spec_from_file_location("word_nightly_icons", HERE / "nightly-icons.py")
ICON_POLICY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ICON_POLICY)


class NightlyIcons(unittest.TestCase):
    def test_committed_outputs_are_what_the_derivation_produces(self):
        outputs = ICON_POLICY.derive()
        self.assertEqual(set(outputs), {
            "wordcraft-nightly.png", "wordcraft-nightly.ico",
            "wordcraft-nightly-1024.png", "wordcraft-nightly.icns",
        })
        for name, content in outputs.items():
            self.assertEqual(hashlib.sha256((ICONS / name).read_bytes()).hexdigest(),
                             hashlib.sha256(content).hexdigest(), f"{name} is stale: run packaging/nightly-icons.py")

    def test_provenance_records_the_same_hashes_and_the_committed_generator(self):
        provenance = json.loads((ICONS / "nightly-provenance.json").read_text())
        for relative, digest in provenance["outputs"].items():
            path = ROOT / relative
            self.assertTrue(path.is_file(), relative)
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest, relative)
        for row in provenance["inputs"]:
            self.assertEqual(hashlib.sha256((ROOT / row["path"]).read_bytes()).hexdigest(), row["sha256"], row["path"])
        generator = ROOT / provenance["method"]["generator"]
        self.assertEqual(hashlib.sha256(generator.read_bytes()).hexdigest(), provenance["method"]["generator_sha256"])
        self.assertNotEqual(provenance["method"]["nightly_background"], provenance["method"]["original_palette"]["background"],
                            "the Nightly field must differ from the app blue")

    def test_the_containers_hold_the_frames_the_platforms_expect(self):
        ico = (ICONS / "wordcraft-nightly.ico").read_bytes()
        count = struct.unpack_from("<H", ico, 4)[0]
        sizes = []
        for index in range(count):
            width, height, _, _, _, bpp, size, offset = struct.unpack_from("<BBBBHHII", ico, 6 + 16 * index)
            self.assertEqual(width, height)
            self.assertEqual(bpp, 32)
            self.assertEqual(ico[offset:offset + 8], b"\x89PNG\r\n\x1a\n")
            sizes.append(width or 256)
        self.assertEqual(sorted(sizes), list(ICON_POLICY.ICO_FRAMES))

        icns = (ICONS / "wordcraft-nightly.icns").read_bytes()
        self.assertEqual(icns[:4], b"icns")
        self.assertEqual(struct.unpack_from(">I", icns, 4)[0], len(icns))
        slots, offset = [], 8
        while offset < len(icns):
            kind, length = struct.unpack_from(">4sI", icns, offset)
            self.assertEqual(icns[offset + 8:offset + 16], b"\x89PNG\r\n\x1a\n", kind)
            slots.append(kind.decode("ascii"))
            offset += length
        expected = list(ICON_POLICY.ICNS_FROM_MASTER.values()) + list(ICON_POLICY.ICNS_EXISTING)
        self.assertEqual(len(expected), len(set(expected)), "each macOS slot is written once")
        self.assertEqual(sorted(slots), sorted(expected))
        self.assertEqual(offset, len(icns))

    def test_a_frame_is_never_upscaled_from_the_committed_master(self):
        master = ICON_POLICY.read_png((ICONS / "wordcraft-macos-512.png").read_bytes())
        for size in ICON_POLICY.ICNS_FROM_MASTER:
            self.assertLessEqual(size, master[0])
        for slot, size in ICON_POLICY.ICNS_EXISTING.items():
            width, height, _ = ICON_POLICY.read_png(ICON_POLICY.read_icns((ICONS / "wordcraft.icns").read_bytes())[slot])
            self.assertEqual((width, height), (size, size), f"the {slot} frame must already be {size} px")


if __name__ == "__main__":
    unittest.main()
