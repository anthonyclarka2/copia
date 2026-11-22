#!/usr/bin/env python3
"""Draws Copia's app icon (three vertical bars) with no dependencies.

Writes the PNG / ICO files Tauri's bundler expects into icons/, and on macOS
also icon.icns (via iconutil). Run from the repository root:

    python3 scripts/make_icons.py
"""
import os
import shutil
import struct
import subprocess
import sys
import zlib

# The popover's dark theme: aubergine ground, purple / amber / red bars.
BG = (25, 16, 28)
TRACK = (51, 38, 53)
BARS = [((210, 140, 225), 0.82), ((238, 187, 88), 0.46), ((246, 110, 96), 0.20)]


def rounded(x, y, x0, y0, x1, y1, r):
    """True if pixel centre (x, y) is inside the rounded rectangle."""
    if not (x0 <= x < x1 and y0 <= y < y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def draw(size):
    s = size / 64.0
    rows = []
    for py in range(size):
        row = bytearray([0])  # PNG filter byte
        for px in range(size):
            x, y = (px + 0.5) / s, (py + 0.5) / s
            rgba = (0, 0, 0, 0)
            if rounded(x, y, 2, 2, 62, 62, 14):
                rgba = BG + (255,)
                for i, (colour, level) in enumerate(BARS):
                    x0 = 13 + i * 14
                    if rounded(x, y, x0, 12, x0 + 10, 52, 3):
                        top = 52 - 40 * level
                        rgba = (colour if y >= top else TRACK) + (255,)
            row += bytes(rgba)
        rows.append(bytes(row))
    return b"".join(rows)


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))


def png(size):
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(draw(size), 9))
        + chunk(b"IEND", b"")
    )


def ico(images):
    """ICO container holding PNG-compressed images: [(size, png_bytes)]."""
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, data in images:
        out += struct.pack(
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
    return out + b"".join(data for _, data in images)


def main():
    os.makedirs("icons", exist_ok=True)
    cache = {}

    def get(size):
        if size not in cache:
            cache[size] = png(size)
        return cache[size]

    for name, size in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ]:
        with open(os.path.join("icons", name), "wb") as f:
            f.write(get(size))

    with open("icons/icon.ico", "wb") as f:
        f.write(ico([(s, get(s)) for s in (16, 32, 48, 256)]))

    if sys.platform == "darwin" and shutil.which("iconutil"):
        iconset = "icons/icon.iconset"
        os.makedirs(iconset, exist_ok=True)
        for pts in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                suffix = "" if scale == 1 else "@2x"
                with open(f"{iconset}/icon_{pts}x{pts}{suffix}.png", "wb") as f:
                    f.write(get(pts * scale))
        subprocess.run(
            ["iconutil", "-c", "icns", iconset, "-o", "icons/icon.icns"], check=True
        )
        shutil.rmtree(iconset)


if __name__ == "__main__":
    main()
