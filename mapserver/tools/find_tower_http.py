"""Show http-body versions in Cargo.lock and tower-http's constraint."""
from pathlib import Path

lock = Path(r"D:\mycomputer\documents\gis-wt\thirdparty\r-mapserver\Cargo.lock")
lines = lock.read_text(encoding="utf-8", errors="replace").splitlines()
cur = None
for i, line in enumerate(lines):
    s = line.strip()
    if s == 'name = "http-body"':
        # print this and the next few lines
        print("\n".join(lines[i : i + 3]))
    if s == 'name = "tower-http"':
        print("\n".join(lines[i : i + 12]))
        print("-" * 40)
