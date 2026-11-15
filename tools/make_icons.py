#!/usr/bin/env python3
"""Generate the minimal MyJanitor icon set for the Tauri build.

Standard library only (zlib + struct). Writes RGBA PNGs and a multi-image ICO
(PNG-compressed entries) into src-tauri/icons/. The drawing is a placeholder
mark: a rounded dark tile with an orange accent bar, matching the palette in
ui/styles/tokens.css (#000000 ink, #FD5108 accent). Re-run after changing it:

    python3 tools/make_icons.py
"""
import os
import struct
import zlib

INK = (0x1A, 0x1A, 0x1A, 0xFF)
ACCENT = (0xFD, 0x51, 0x08, 0xFF)
CLEAR = (0, 0, 0, 0)


def pixel(x, y, n):
    """Colour of pixel (x, y) in an n x n icon."""
    r = n * 0.18  # corner radius
    # rounded-rectangle mask
    cx = min(max(x + 0.5, r), n - r)
    cy = min(max(y + 0.5, r), n - r)
    if (x + 0.5 - cx) ** 2 + (y + 0.5 - cy) ** 2 > r * r:
        return CLEAR
    # accent bar: a broom-like diagonal stripe in the lower half
    u, v = (x + 0.5) / n, (y + 0.5) / n
    if 0.22 < u < 0.78 and 0.62 < v < 0.76:
        return ACCENT
    if 0.44 < u < 0.56 and 0.20 < v < 0.62:
        return ACCENT
    return INK


def png(n):
    rows = bytearray()
    for y in range(n):
        rows.append(0)  # filter type: none
        for x in range(n):
            rows.extend(pixel(x, y, n))

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b""))


def ico(sizes):
    images = [png(s) for s in sizes]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    for s, data in zip(sizes, images):
        dim = 0 if s >= 256 else s
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    return header + entries + b"".join(images)


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons")
    os.makedirs(out, exist_ok=True)
    files = {
        "32x32.png": png(32),
        "128x128.png": png(128),
        "128x128@2x.png": png(256),
        "icon.png": png(512),
        "icon.ico": ico([16, 24, 32, 48, 64, 256]),
    }
    for name, data in files.items():
        with open(os.path.join(out, name), "wb") as f:
            f.write(data)
        print(f"wrote {name} ({len(data)} bytes)")


if __name__ == "__main__":
    main()
