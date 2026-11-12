"""Draws the app icon and the sidebar logo into crates/tt-app/assets.

    python crates/tt-app/tools/gen_icon.py

Standard library only. Both are a rounded square with a 135 degree blue -> violet
gradient (#3b82f6 -> #8b5cf6). `app.ico` (16, 24, 32, 48, 64, 256 px, PNG entries) adds a
white clock (ring and two hands); `logo.png` (36 px, shown at 18 px) is the plain square:
Slint's software renderer ignores the border radius of gradient rectangles, so the
sidebar shows this image instead. Linux also gets `app-256.png` (the 256 px `app.ico`
image, for the .desktop entry and the package) and `app-64.rgba` (the same drawing at 64 px
as raw RGBA, row by row, no header: the window icon embedded in the executable).
macOS gets `app.icns` (PNG entries, 16 to 1024 px): the same drawing on the macOS icon grid,
the rounded square filling 824/1024 of the canvas, centered, with transparent margins.
Drawn with 4x4 supersampling.
"""

import math
import struct
import zlib
from pathlib import Path

SIZES = (16, 24, 32, 48, 64, 256)
BLUE = (0x3B, 0x82, 0xF6)
VIOLET = (0x8B, 0x5C, 0xF6)
SUB = 4  # supersampling per axis
ASSETS = Path(__file__).resolve().parent.parent / "assets"
LOGO_SIZE = 36
MAC_INSET = 100 / 1024  # margin per side: the square is 824/1024 of the canvas
# icns entry type -> PNG size. several types share a size, so each size is drawn once.
ICNS_TYPES = (
    ("icp4", 16),
    ("icp5", 32),
    ("ic11", 32),
    ("ic12", 64),
    ("ic07", 128),
    ("ic13", 256),
    ("ic08", 256),
    ("ic14", 512),
    ("ic09", 512),
    ("ic10", 1024),
)


def rounded_square(x, y, radius):
    """Whether (x, y) in the unit square lies inside the rounded square."""
    cx = min(max(x, radius), 1 - radius)
    cy = min(max(y, radius), 1 - radius)
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius**2


def segment_distance(px, py, ax, ay, bx, by):
    abx, aby = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * abx + (py - ay) * aby) / (abx**2 + aby**2)))
    return math.hypot(px - (ax + t * abx), py - (ay + t * aby))


def clock(x, y, size):
    """Whether (x, y) is on the white clock mark. Strokes get thicker at small sizes."""
    stroke = max(0.085, 1.6 / size)
    d = math.hypot(x - 0.5, y - 0.5)
    ring = abs(d - 0.29) <= stroke / 2
    # Hands: to 12 o'clock and to about 2 o'clock (rounded ends).
    hour = segment_distance(x, y, 0.5, 0.5, 0.5, 0.31) <= stroke / 2
    minute = segment_distance(x, y, 0.5, 0.5, 0.64, 0.56) <= stroke / 2
    return ring or hour or minute


def pixels(size, with_clock=True, inset=0.0):
    """RGBA rows (bytes, no PNG filter byte) of the icon at `size` px.

    `inset` is the transparent margin per side, as a fraction of the canvas.
    """
    radius = 0.28
    scale = 1 - 2 * inset
    art = size * scale
    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SUB):
                for sx in range(SUB):
                    x = ((px + (sx + 0.5) / SUB) / size - inset) / scale
                    y = ((py + (sy + 0.5) / SUB) / size - inset) / scale
                    if not rounded_square(x, y, radius):
                        continue
                    if with_clock and clock(x, y, art):
                        color = (255, 255, 255)
                    else:
                        t = (x + y) / 2  # 135 degrees: top-left -> bottom-right
                        color = tuple(b + (v - b) * t for b, v in zip(BLUE, VIOLET))
                    for i in range(3):
                        acc[i] += color[i]
                    acc[3] += 1
            n = acc[3]
            if n:
                row += bytes(round(acc[i] / n) for i in range(3))
                row.append(round(255 * n / SUB**2))
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def render(size, with_clock=True, inset=0.0):
    rows = pixels(size, with_clock, inset)
    return png(size, b"".join(b"\0" + row for row in rows))  # PNG filter: none


def png(size, raw):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(images):
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, data in images:
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    return out + b"".join(data for _, data in images)


def icns(images):
    """images: (type, png bytes) pairs. Entries: type, length with header, PNG."""
    body = b"".join(
        kind.encode("ascii") + struct.pack(">I", 8 + len(data)) + data for kind, data in images
    )
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main():
    mac = {size: render(size, inset=MAC_INSET) for size in sorted({s for _, s in ICNS_TYPES})}
    outputs = {
        "app.ico": ico([(s, render(s)) for s in SIZES]),
        "logo.png": render(LOGO_SIZE, with_clock=False),
        "app-256.png": render(256),
        "app-64.rgba": b"".join(pixels(64)),
        "app.icns": icns([(kind, mac[size]) for kind, size in ICNS_TYPES]),
    }
    for name, data in outputs.items():
        path = ASSETS / name
        path.write_bytes(data)
        print(f"wrote {path} ({len(data)} bytes)")


if __name__ == "__main__":
    main()
