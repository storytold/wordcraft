#!/usr/bin/env python3
"""Derive the WordCraft Nightly icon set from the committed stable renders.

The stable artwork is untouched: only the background contribution changes from the app's blue
to the deeper Nightly background. Every pixel is decomposed in the stable drawing's three-colour
palette (ink, paper, background) and rebuilt with the Nightly background, so the foreground lines,
the paper, the alpha and the macOS padding stay exactly as they were drawn. There is no letter,
crescent or other badge; the app and the Launcher name the channel in text.

Sources (all committed, all rendered from assets/app-icon/wordcraft.svg):
  * assets/app-icon/wordcraft.ico                    nine full-bleed frames, 16-256 px
  * assets/app-icon/wordcraft.icns                   padded macOS frames, 32-1024 px
  * assets/app-icon/hicolor/256x256/...png           the 256 px full-bleed frame
  * assets/app-icon/wordcraft-macos-512.png          the padded macOS master

Outputs:
  * assets/app-icon/wordcraft-nightly.png            full-bleed 256 px (Windows/Linux tar)
  * assets/app-icon/wordcraft-nightly.ico            nine full-bleed frames, 16-256 px
  * assets/app-icon/wordcraft-nightly-1024.png       padded macOS 1024 px master
  * assets/app-icon/wordcraft-nightly.icns           eleven padded macOS frames, 16-1024 px

Usage: python3 packaging/nightly-icons.py [--check]
`--check` re-derives into memory and compares the committed outputs' SHA-256 instead of writing.
"""
import argparse
import hashlib
import io
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "assets" / "app-icon"

# The stable drawing's three colours (assets/app-icon/README.md) and the Nightly background.
INK = (0x0B, 0x0B, 0x0C)
PAPER = (0xEF, 0xE9, 0xDC)
BACKGROUND = (0x3B, 0x5B, 0xDB)
NIGHTLY_BACKGROUND = (0x23, 0x2C, 0x67)

ICO_FRAMES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
# Padded macOS slots by pixel size: PNG slots the committed icns already holds, and the three
# small ones derived from the padded 512 px master.
ICNS_FROM_MASTER = {16: "icp4", 32: "icp5", 64: "icp6"}
# The padded frames the committed icns already holds, by slot and pixel size (16@2x, 32@2x, ...).
ICNS_EXISTING = {"ic11": 32, "ic12": 64, "ic07": 128, "ic08": 256, "ic13": 256, "ic09": 512, "ic14": 512, "ic10": 1024}


def read_png(data: bytes):
    """Decode an 8-bit RGB/RGBA, non-interlaced PNG into (width, height, RGBA bytes)."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    offset, header, compressed = 8, None, bytearray()
    while offset < len(data):
        length, kind = struct.unpack_from(">I4s", data, offset)
        body = data[offset + 8:offset + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            compressed += body
        elif kind == b"IEND":
            break
        offset += 12 + length
    width, height, depth, colour, _, _, interlace = header
    if depth != 8 or colour not in (2, 6) or interlace != 0:
        raise ValueError("expected an 8-bit non-interlaced RGB/RGBA PNG")
    channels = 3 if colour == 2 else 4
    stride = width * channels
    raw = zlib.decompress(bytes(compressed))
    if len(raw) != height * (stride + 1):
        raise ValueError("unexpected PNG payload length")
    pixels = bytearray(width * height * 4)
    previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        filter_kind = raw[start]
        line = bytearray(raw[start + 1:start + 1 + stride])
        if filter_kind == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif filter_kind == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_kind == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_kind == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = previous[i]
                c = previous[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 0xFF
        elif filter_kind != 0:
            raise ValueError("unknown PNG filter")
        previous = line
        out = y * width * 4
        if channels == 4:
            pixels[out:out + width * 4] = line
        else:
            for x in range(width):
                pixels[out + x * 4:out + x * 4 + 3] = line[x * 3:x * 3 + 3]
                pixels[out + x * 4 + 3] = 255
    return width, height, pixels


def write_png(width: int, height: int, pixels: bytearray) -> bytes:
    stride = width * 4
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw += pixels[y * stride:(y + 1) * stride]

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def _inverse(matrix):
    (a, b, c), (d, e, f), (g, h, i) = matrix
    determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    if abs(determinant) < 1e-9:
        raise ValueError("the palette's colours are not independent")
    return (
        ((e * i - f * h) / determinant, (c * h - b * i) / determinant, (b * f - c * e) / determinant),
        ((f * g - d * i) / determinant, (a * i - c * g) / determinant, (c * d - a * f) / determinant),
        ((d * h - e * g) / determinant, (b * g - a * h) / determinant, (a * e - b * d) / determinant),
    )


def _weights():
    """Each palette colour as a weight (ink, paper, background) per channel."""
    matrix = ((INK[0], PAPER[0], BACKGROUND[0]), (INK[1], PAPER[1], BACKGROUND[1]), (INK[2], PAPER[2], BACKGROUND[2]))
    return _inverse(matrix)


def recolour(pixels: bytearray) -> bytearray:
    """Rebuild every pixel with the Nightly background, keeping its ink and paper weights."""
    inverse = _weights()
    out = bytearray(len(pixels))
    for i in range(0, len(pixels), 4):
        alpha = pixels[i + 3]
        if alpha == 0:
            continue
        red, green, blue = pixels[i], pixels[i + 1], pixels[i + 2]
        # The three palette weights that reproduce this colour exactly.
        w_ink = inverse[0][0] * red + inverse[0][1] * green + inverse[0][2] * blue
        w_paper = inverse[1][0] * red + inverse[1][1] * green + inverse[1][2] * blue
        w_background = inverse[2][0] * red + inverse[2][1] * green + inverse[2][2] * blue
        # A pixel between two edges can fall outside the triangle: clamp and renormalise, so the
        # result is still a mixture of the three colours and never a new tint.
        weights = [max(0.0, w_ink), max(0.0, w_paper), max(0.0, w_background)]
        total = sum(weights)
        if total <= 0:
            out[i:i + 4] = bytes((*NIGHTLY_BACKGROUND, alpha))
            continue
        weights = [w / total for w in weights]
        mixed = [weights[0] * INK[j] + weights[1] * PAPER[j] + weights[2] * NIGHTLY_BACKGROUND[j] for j in range(3)]
        out[i:i + 3] = bytes(min(255, max(0, round(v))) for v in mixed)
        out[i + 3] = alpha
    return out


def downsample(width: int, height: int, pixels: bytearray, size: int) -> bytearray:
    """Box-filter `pixels` to `size` x `size`: only used to shrink the padded master, never to grow a frame."""
    if size > width or size > height:
        raise ValueError("a frame is never upscaled")
    factor = width // size
    if factor * size != width or height != width:
        raise ValueError("the master must divide evenly into each slot")
    out = bytearray(size * size * 4)
    area = factor * factor
    for y in range(size):
        for x in range(size):
            total = [0, 0, 0, 0]
            for dy in range(factor):
                row = ((y * factor + dy) * width + x * factor) * 4
                for dx in range(factor):
                    at = row + dx * 4
                    total[0] += pixels[at]
                    total[1] += pixels[at + 1]
                    total[2] += pixels[at + 2]
                    total[3] += pixels[at + 3]
            at = (y * size + x) * 4
            alpha = total[3]
            if alpha == 0:
                continue
            out[at] = round(total[0] / area)
            out[at + 1] = round(total[1] / area)
            out[at + 2] = round(total[2] / area)
            out[at + 3] = round(alpha / area)
    return out


def read_ico(data: bytes):
    """Every PNG frame of an .ico as {size: png bytes}."""
    if data[:4] != b"\x00\x00\x01\x00":
        raise ValueError("not an .ico")
    count = struct.unpack_from("<H", data, 4)[0]
    frames = {}
    for index in range(count):
        width, height, _, _, _, _, size, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16 * index)
        body = data[offset:offset + size]
        if body[:8] != b"\x89PNG\r\n\x1a\n":
            raise ValueError("only PNG-compressed .ico frames are expected")
        size_px = width or 256
        if height and height != width:
            raise ValueError("square .ico frames are expected")
        frames[size_px or 256] = body
    return frames


def write_ico(frames) -> bytes:
    header = struct.pack("<HHH", 0, 1, len(frames))
    entries, bodies, offset = bytearray(), bytearray(), 6 + 16 * len(frames)
    for size in sorted(frames):
        body = frames[size]
        width = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", width, width, 0, 0, 1, 32, len(body), offset)
        bodies += body
        offset += len(body)
    return header + bytes(entries) + bytes(bodies)


def read_icns(data: bytes):
    """The PNG slots of an .icns as {four-letter type: png bytes}, in file order."""
    if data[:4] != b"icns":
        raise ValueError("not an .icns")
    offset, slots = 8, {}
    while offset < len(data):
        kind, length = struct.unpack_from(">4sI", data, offset)
        body = data[offset + 8:offset + length]
        if body[:8] == b"\x89PNG\r\n\x1a\n":
            slots[kind.decode("ascii")] = body
        offset += length
    return slots


def write_icns(slots) -> bytes:
    order = ["icp4", "icp5", "icp6", "ic07", "ic08", "ic09", "ic10", "ic11", "ic12", "ic13", "ic14"]
    body = bytearray()
    for kind in order:
        payload = slots[kind]
        body += kind.encode("ascii") + struct.pack(">I", len(payload) + 8) + payload
    return b"icns" + struct.pack(">I", len(body) + 8) + bytes(body)


def derive():
    """Build every Nightly artifact in memory: {relative path: bytes}."""
    outputs = {}
    ico = read_ico((ICONS / "wordcraft.ico").read_bytes())
    nightly_frames = {}
    for size in ICO_FRAMES:
        width, height, pixels = read_png(ico[size])
        if width != size:
            raise ValueError(f"the {size} px .ico frame is {width} px")
        nightly_frames[size] = write_png(width, height, recolour(pixels))
    outputs["wordcraft-nightly.ico"] = write_ico(nightly_frames)

    two_fifty_six = read_png((ICONS / "hicolor/256x256/apps/ai.storyteller.wordcraft.png").read_bytes())
    outputs["wordcraft-nightly.png"] = write_png(two_fifty_six[0], two_fifty_six[1], recolour(two_fifty_six[2]))

    master = read_png((ICONS / "wordcraft-macos-512.png").read_bytes())
    recoloured_master = recolour(master[2])
    existing = read_icns((ICONS / "wordcraft.icns").read_bytes())
    slots = {}
    for size, kind in ICNS_FROM_MASTER.items():
        slots[kind] = write_png(size, size, downsample(master[0], master[1], recoloured_master, size))
    for kind, size in ICNS_EXISTING.items():
        width, height, pixels = read_png(existing[kind])
        if width != size or height != size:
            raise ValueError(f"the {kind} slot is {width}x{height}, not {size}x{size}")
        slots[kind] = write_png(width, height, recolour(pixels))
        if size == 1024:
            outputs["wordcraft-nightly-1024.png"] = slots[kind]
    outputs["wordcraft-nightly.icns"] = write_icns(slots)
    return outputs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare the committed outputs instead of writing")
    args = parser.parse_args()
    outputs = derive()
    failed = False
    for name, content in sorted(outputs.items()):
        path = ICONS / name
        digest = hashlib.sha256(content).hexdigest()
        if args.check:
            current = hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else "missing"
            status = "ok" if current == digest else "differs"
            failed |= current != digest
            print(f"{status:8} {name} {digest}")
        else:
            path.write_bytes(content)
            print(f"wrote    {name} {digest} ({len(content)} bytes)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
