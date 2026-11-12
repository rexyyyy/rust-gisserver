import io

p = r"d:\mycomputor\documents\gis-wt\desktop\src\config.js"
src = io.open(p, encoding="utf-8").read()
marker = "export const THREED_TILES_SERVICES = [\n"
entry = (
    "  {\n"
    "    id: 'kmczc-reconv',\n"
    "    name: 'kmczc 重转 (EPSG:2358 投影/米 / rust-gisserver 发布)',\n"
    "    url: 'http://127.0.0.1:8080/3dtiles/kmczc-reconv/tileset.json',\n"
    "  },\n"
)
if "id: 'kmczc-reconv'" in src:
    print("already present, no change")
elif marker in src:
    src = src.replace(marker, marker + entry, 1)
    io.open(p, "w", encoding="utf-8", newline="\n").write(src)
    print("inserted kmczc-reconv entry")
else:
    print("MARKER NOT FOUND")
