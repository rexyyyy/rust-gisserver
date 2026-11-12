#!/usr/bin/env python3
"""Verify MBTiles tile pixel colors (fixture encodes R=zoom, G=column, B=TMS row)."""
import struct
import sys
import zlib
from pathlib import Path


def read_png_pixel(path: Path) -> tuple:
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", f"{path}: not a PNG"
    pos = 8
    idat = b""
    w = h = None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        tag = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        if tag == b"IHDR":
            w, h = struct.unpack(">II", chunk[:8])
        elif tag == b"IDAT":
            idat += chunk
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = 1 + w * 3  # filter byte + RGB
    first = raw[:stride]
    assert first[0] == 0, f"{path}: unexpected filter {first[0]}"
    return (first[1], first[2], first[3], w, h)


def check(label: str, path: Path, expect: tuple):
    r, g, b, w, h = read_png_pixel(path)
    got = (r, g, b)
    ok = got == expect
    mark = "PASS" if ok else "FAIL"
    print(f"[{mark}] {label}: pixel={got} expect={expect} ({w}x{h})")
    return ok


def main():
    d = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp")
    results = []
    # z0: single tile, XYZ y=0 == TMS row 0 -> (0, 0, 0)
    results.append(check("mbtiles z0/0/0 (XYZ y=0 -> TMS row 0)", d / "mb_z0.png", (0, 0, 0)))
    # z1 x=1 XYZ y=0 -> TMS row = 1 -> (60, 60, 60)
    results.append(check("mbtiles z1/1/0 (XYZ y=0 -> TMS row 1)", d / "mb_z1a.png", (60, 60, 60)))
    # z1 x=1 XYZ y=1 -> TMS row = 0 -> (60, 60, 0)
    results.append(check("mbtiles z1/1/1 (XYZ y=1 -> TMS row 0)", d / "mb_z1b.png", (60, 60, 0)))
    # XYZ front-end on the same MBTiles layer must equal the /mbtiles route
    results.append(check("xyz front-end z1/1/0 == mbtiles route", d / "mb_xyz.png", (60, 60, 60)))
    # TMS front-end: y=0 is bottom-up -> grid row 1 -> XYZ y=1 -> TMS row 0
    results.append(check("tms front-end z1/1/0 (y=0 bottom-up -> TMS row 0)", d / "mb_tms.png", (60, 60, 0)))
    print("ALL PASS" if all(results) else "SOME FAILED")
    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
