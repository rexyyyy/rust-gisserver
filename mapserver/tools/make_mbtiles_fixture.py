#!/usr/bin/env python3
"""Build a small MBTiles fixture for r-mapserver regression tests.

Creates fixtures/test.mbtiles: a PNG archive over z0..z3 in WebMercatorQuad
scheme (tiles stored in TMS rows, bottom-up, per the MBTiles spec).

Also builds fixtures/test-vector.mbtiles: a vector archive (format=pbf) whose
tiles are gzip-compressed payloads (the common tippecanoe/OpenMapTiles layout),
so the gzip pass-through path is exercised too.

Run:  python make_mbtiles_fixture.py
"""
import gzip
import sqlite3
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent  # project root


def png_bytes(r: int, g: int, b: int, w: int = 256, h: int = 256) -> bytes:
    """Minimal solid-color PNG (no PIL dependency)."""
    raw = b"".join(b"\x00" + bytes([r, g, b]) * w for _ in range(h))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)  # 8-bit truecolor
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )


def build(path: Path, fmt: str, tile_for, meta_extra: dict):
    if path.exists():
        path.unlink()
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE metadata (name text, value text);
        CREATE TABLE tiles (
            zoom_level integer,
            tile_column integer,
            tile_row integer,
            tile_data blob,
            PRIMARY KEY (zoom_level, tile_column, tile_row)
        );
        CREATE UNIQUE INDEX tile_index on tiles (zoom_level, tile_column, tile_row);
        """
    )
    meta = {
        "name": path.stem,
        "format": fmt,
        "bounds": "-180.0,-85.0,180.0,85.0",
        "minzoom": "0",
        "maxzoom": "3",
        "type": "overlay" if fmt == "png" else "baselayer",
    }
    meta.update(meta_extra)
    conn.executemany(
        "INSERT INTO metadata VALUES (?, ?)", sorted(meta.items())
    )
    # z0..z3, every tile, TMS rows (row 0 at the BOTTOM).
    for z in range(4):
        n = 1 << z
        for x in range(n):
            for tms_row in range(n):
                data = tile_for(z, x, tms_row)
                conn.execute(
                    "INSERT INTO tiles VALUES (?, ?, ?, ?)",
                    (z, x, tms_row, sqlite3.Binary(data)),
                )
    conn.commit()
    conn.close()
    n_tiles = sum((1 << z) ** 2 for z in range(4))
    print(f"wrote {path} ({path.stat().st_size} bytes, {n_tiles} tiles)")


def main():
    # Raster archive: color encodes the tile address so a flipped/misread tile
    # is obvious: R = zoom, G = column, B = TMS row.
    build(
        ROOT / "fixtures" / "test.mbtiles",
        "png",
        lambda z, x, r: png_bytes(z * 60 % 256, x * 60 % 256, r * 60 % 256),
        {"description": "r-mapserver MBTiles regression fixture (PNG)"},
    )

    # Vector archive: gzip-compressed fake pbf payload (gzip magic 1f 8b).
    def vec_tile(z, x, r):
        payload = f"fake-pbf z{z} x{x} row{r}".encode()
        return gzip.compress(payload)

    build(
        ROOT / "fixtures" / "test-vector.mbtiles",
        "pbf",
        vec_tile,
        {"description": "r-mapserver MBTiles regression fixture (gzip pbf)"},
    )


if __name__ == "__main__":
    main()
