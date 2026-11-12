import io

p = r"d:\mycomputor\documents\gis-wt\desktop\src\config.js"
src = io.open(p, encoding="utf-8").read()
marker = "export const THREED_TILES_SERVICES = [\n"
entry = (
    "  {\n"
    "    id: 'osgb4326',\n"
    "    name: 'OSGB4326 倾斜摄影 (EPSG:4326 昆明 / r-manager 发布)',\n"
    "    url: 'http://127.0.0.1:8080/3dtiles/osgb4326/tileset.json',\n"
    "  },\n"
)
if "osgb4326_3dtiles" in src or "id: 'osgb4326'" in src:
    print("already present, no change")
elif marker in src:
    src = src.replace(marker, marker + entry, 1)
    io.open(p, "w", encoding="utf-8", newline="\n").write(src)
    print("inserted osgb4326 entry into THREED_TILES_SERVICES")
else:
    print("MARKER NOT FOUND")
