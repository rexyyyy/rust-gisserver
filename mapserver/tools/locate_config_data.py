#!/usr/bin/env python3
"""Locate the real data paths referenced (mis-spelled) by rust-gisserver configs."""
import os
from pathlib import Path

ROOT = Path(os.environ.get("LOCATE_ROOT", Path.home() / "Documents"))
TARGETS = [
    "wh2013_small_4326.tif",
    "recon_footprint_4326.geojson",
    "recon_3dtiles",
    "kmczc_3dtiles",
    "mcy_3dtiles",
]


def main():
    for t in TARGETS:
        hits = []
        for dirpath, dirnames, filenames in os.walk(ROOT):
            dirnames[:] = [
                d for d in dirnames
                if d not in {"target", "node_modules", ".git"}
            ]
            depth = dirpath[len(str(ROOT)) :].count(os.sep)
            if depth > 5:
                dirnames[:] = []
                continue
            for n in list(dirnames) + filenames:
                if t.lower() in n.lower():
                    hits.append(os.path.join(dirpath, n))
            if len(hits) >= 3:
                break
        print(f"{t}:")
        for h in hits[:3]:
            print(f"    {h}")
        if not hits:
            print("    (not found under documents, depth<=5)")


if __name__ == "__main__":
    main()
