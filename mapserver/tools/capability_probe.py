"""Capability probe: routes, semantics AND pixel content.

Builds its own catalog (cog world + @512 grid layer, vector places,
mbtiles test + test-vector, a live 3dtiles service with real files) and
probes: ops routes, tileMatrixSets (incl. @512 size suffix), WMS 1.1.1/1.3.0
depth + error semantics (PNG-only FORMAT whitelist is spec-correct),
WMTS caps XML, MVT depth, MBTiles vector gzip passthrough, pmtiles 404,
3dtiles traversal protection, HEAD, concurrency smoke.

Pixel-content assertions (distinct colors > 1) guard the "200 OK but blank
canvas" class of bugs: the renderer's non-identity per-pixel path and the
WMS CRS:84 axis order both regressed that way once.

Usage: python tools/capability_probe.py
"""
import http.client
import shutil
import subprocess
import sys
import time
import zlib
from pathlib import Path

ROOT = Path(r"D:\mycomputer\documents\gis-wt\thirdparty\rust-gisserver")
EXE = ROOT / "target" / "release" / "map-api.exe"
GOOD = Path("E:/tmp/rms-probe")
TDS = GOOD / "tds" / "probe"          # 3dtiles dataset root
HOST, PORT = "127.0.0.1", 18084


def req(path, headers=None, method="GET"):
    c = http.client.HTTPConnection(HOST, PORT, timeout=10)
    c.request(method, path, headers=headers or {})
    r = c.getresponse()
    body = r.read()
    hdrs = {k.lower(): v for k, v in r.getheaders()}
    c.close()
    return r.status, hdrs, body


def wait_up(proc, timeout_s=8):
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        try:
            c = http.client.HTTPConnection(HOST, PORT, timeout=2)
            c.request("GET", "/health")
            ok = c.getresponse().status == 200
            c.close()
            if ok:
                return True
        except OSError:
            pass
        if proc.poll() is not None:
            return False
        time.sleep(0.2)
    return False


def png_distinct(b):
    """Distinct sampled pixel colors of an RGBA/RGB PNG (1 == flat/blank)."""
    w = int.from_bytes(b[16:20], "big")
    h = int.from_bytes(b[20:24], "big")
    ct = b[25]
    ch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ct]
    idat = b""
    i = 8
    while i < len(b):
        ln = int.from_bytes(b[i:i + 4], "big")
        typ = b[i + 4:i + 8]
        if typ == b"IDAT":
            idat += b[i + 8:i + 8 + ln]
        i += 12 + ln
    raw = zlib.decompress(idat)
    stride = w * ch + 1
    colors = set()
    for yy in range(0, h, 5):
        row = raw[yy * stride:(yy + 1) * stride]
        for xx in range(0, w, 5):
            colors.add(bytes(row[1 + xx * ch:1 + (xx + 1) * ch]))
    return len(colors)


def build_catalog():
    if GOOD.exists():
        shutil.rmtree(GOOD, ignore_errors=True)
    for sub in ("cog", "vector", "mbtiles", "3dtiles"):
        (GOOD / sub).mkdir(parents=True, exist_ok=True)
    shutil.copy(ROOT / "config/cog/world.yaml", GOOD / "cog/world.yaml")
    # A second COG layer that uses the 512px size-suffixed preset grid.
    (GOOD / "cog/world512.yaml").write_text(
        "name: world512\ntitle: world @512 grid\nsource:\n  type: cog\n"
        "  path: fixtures/sample.tif\nsrc_crs: EPSG:4326\n"
        "grids: [WebMercatorQuad@512]\ntile_px: 256\n",
        encoding="utf-8",
    )
    shutil.copy(ROOT / "config/vector/places.yaml", GOOD / "vector/places.yaml")
    shutil.copy(ROOT / "config/mbtiles/test.yaml", GOOD / "mbtiles/test.yaml")
    shutil.copy(ROOT / "config/mbtiles/test-vector.yaml", GOOD / "mbtiles/test-vector.yaml")
    # A live 3D Tiles service with real (tiny) files on disk.
    (TDS / "Tile_0").mkdir(parents=True, exist_ok=True)
    (TDS / "tileset.json").write_text(
        '{"asset":{"version":"1.1"},"geometricError":100,'
        '"root":{"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},'
        '"geometricError":10,"content":{"uri":"Tile_0/tile.b3dm"}}}',
        encoding="utf-8",
    )
    (TDS / "Tile_0" / "tile.b3dm").write_bytes(b"b3dm" + b"\x00" * 60)
    (GOOD / "3dtiles" / "probe.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<TilesetService>\n'
        "  <Name>probe</Name>\n  <Title>probe service</Title>\n"
        f"  <Root>{TDS}</Root>\n  <Entry>tileset.json</Entry>\n"
        "</TilesetService>\n",
        encoding="utf-8",
    )
    (GOOD / "mapserver.yaml").write_text(
        "server:\n  host: 127.0.0.1\n  port: 18084\n  cache_mib: 64\n"
        "catalog:\n  root: E:/tmp/rms-probe\n  backend: files\n",
        encoding="utf-8",
    )


def main():
    build_catalog()
    proc = subprocess.Popen(
        [str(EXE), "serve", "--config", str(GOOD / "mapserver.yaml")],
        stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT,
        cwd=str(ROOT),
    )
    if not wait_up(proc):
        print("server did not start")
        return 1

    results = []

    def probe(name, path, expect, note="", headers=None, method="GET", contains=None,
              not_contains=None):
        s, h, b = req(path, headers, method)
        ok = s == expect
        detail = ""
        if contains:
            has = contains.encode() in b
            ok = ok and has
            detail = f" contains'{contains[:24]}'={has}"
        if not_contains:
            leak = not_contains.encode() in b
            ok = ok and not leak
            detail += f" leak'{not_contains[:16]}'={leak}"
        ct = h.get("content-type", "")
        if ok:
            results.append(f"OK   {name:26s} -> {s} [{ct[:38]}]{detail} {note}")
        else:
            results.append(f"FAIL {name:26s} -> {s} (expect {expect}){detail} "
                           f"body={b[:100]!r}")

    # ---- ops ----
    probe("ready", "/ready", 200)
    probe("api/config", "/api/config", 200, contains="cache_mib")
    probe("viewer HTML", "/viewer", 200, contains="leaflet")
    probe("cesium HTML", "/cesium", 200, contains="cesium")
    probe("HEAD health", "/health", 200, method="HEAD")
    probe("HEAD tile", "/tms/1.0.0/world/2/1/1.png", 200, method="HEAD")

    # ---- tile matrix sets ----
    probe("tileMatrixSets preset", "/tileMatrixSets/WebMercatorQuad", 200)
    probe("tileMatrixSets @512", "/tileMatrixSets/WebMercatorQuad@512", 200,
          note="(size suffix resolves when a layer uses it)")
    probe("tileMatrixSets unknown", "/tileMatrixSets/NopeQuad", 404)
    probe("XYZ tile on @512 grid", "/tiles/world512/WebMercatorQuad@512/2/1/1.png", 200)

    # ---- WMS depth ----
    probe("WMS 1.1.1 caps", "/wms?service=WMS&request=GetCapabilities&version=1.1.1", 200,
          contains="WMS_Capabilities")
    probe("WMS bad request (exc)", "/wms?service=WMS&request=Bogus", 400,
          contains="ServiceException")
    probe("WMS unknown layer", "/wms?service=WMS&request=GetMap&version=1.3.0&layers=nope"
          "&crs=EPSG:4326&bbox=100,36,108,40&width=64&height=64&format=image/png", 400,
          contains="LayerNotDefined")
    probe("WMS jpeg -> InvalidFormat", "/wms?service=WMS&request=GetMap&version=1.1.1"
          "&layers=world&srs=EPSG:4326&bbox=100,36,108,40&width=64&height=64"
          "&format=image/jpeg", 400, contains="InvalidFormat",
          note="(PNG-only encoder; caps honestly advertises image/png)")
    probe("WMS GetFeatureInfo", "/wms?service=WMS&request=GetFeatureInfo&version=1.1.1"
          "&query_layers=world&srs=EPSG:4326&bbox=100,36,108,40&width=64&height=64"
          "&x=32&y=32&info_format=text/plain", 200, contains="value =",
          note="(raster sampling path)")
    probe("WMS GetFeatureInfo bad format", "/wms?service=WMS&request=GetFeatureInfo&version=1.1.1"
          "&query_layers=world&srs=EPSG:4326&bbox=100,36,108,40&width=64&height=64"
          "&x=32&y=32&info_format=text/xml", 400, contains="InvalidFormat",
          note="(unsupported INFO_FORMAT rejected per WMS spec)")

    # ---- WMTS depth ----
    s, h, b = req("/wmts/1.0.0/WMTSCapabilities.xml")
    results.append(f"{'OK  ' if s == 200 and b'<Capabilities' in b else 'FAIL'} "
                   f"{'WMTS caps XML':26s} -> {s} ({len(b)}B)")

    # ---- MVT depth ----
    probe("MVT style.json", "/mvt/places/style.json", 200)
    probe("MVT tms variant", "/mvt/places/WorldCRS84Quad/2/1/1.pbf", 200)
    probe("MVT unknown layer", "/mvt/nope/2/1/1.pbf", 404)

    # ---- pixel content: tiles + GetMap must render REAL pixels, not a
    # blank canvas (regression guard: non-identity render path + CRS:84
    # axis order both once returned "200 OK, 1 distinct color").
    # world layer extent: 100..108 E, 36..40 N. z3 tile over the centroid
    # (104 E, 38 N): col 6, row 3 (TMS row = 7 - 3 = 4).
    import math
    mx0 = 100.0 * 20037508.3427892 / 180.0
    mx1 = 108.0 * 20037508.3427892 / 180.0
    my0 = math.log(math.tan(math.pi / 4 + math.radians(36.0) / 2)) * 6378137.0
    my1 = math.log(math.tan(math.pi / 4 + math.radians(40.0) / 2)) * 6378137.0
    merc_bbox = "%.2f,%.2f,%.2f,%.2f" % (mx0, my0, mx1, my1)

    def probe_content(name, path, min_colors, headers=None):
        s, h, b = req(path, headers)
        ok = s == 200 and b[:8] == b"\x89PNG\r\n\x1a\n"
        n = png_distinct(b) if ok else -1
        good = ok and n >= min_colors
        results.append(
            f"{'OK  ' if good else 'FAIL'} {name:26s} -> {s} distinct_colors={n} "
            f"(expect >={min_colors})")
    probe_content("XYZ tile content", "/tiles/world/3/6/3.png", 4)
    probe_content("TMS tile content", "/tms/1.0.0/world/3/6/4.png", 4)
    probe_content("WMTS REST tile content", "/wmts/rest/world/default/WebMercatorQuad/3/3/6.png", 4)
    probe_content("WMTS KVP tile content", "/wmts?request=GetTile&version=1.0.0&layer=world"
                  "&style=default&tilematrixset=WebMercatorQuad&tilematrix=3&tilerow=3"
                  "&tilecol=6&format=image/png", 4)
    probe_content("GetMap CRS:84 content", "/wms?service=WMS&request=GetMap&version=1.3.0"
                  "&layers=world&crs=CRS:84&bbox=100,36,108,40&width=256&height=256"
                  "&format=image/png", 4)
    probe_content("GetMap 3857 content", "/wms?service=WMS&request=GetMap&version=1.3.0"
                  "&layers=world&crs=EPSG:3857&bbox=%s&width=256&height=256"
                  "&format=image/png" % merc_bbox, 4)
    probe_content("GetMap 4326 content", "/wms?service=WMS&request=GetMap&version=1.3.0"
                  "&layers=world&crs=EPSG:4326&bbox=36,100,40,108&width=256&height=256"
                  "&format=image/png", 4)

    # ---- MBTiles depth ----
    probe("MBTiles vector info", "/mbtiles/test-vector", 200)
    s, h, b = req("/mbtiles/test-vector/1/1/0", {"Accept-Encoding": "gzip"})
    gz = h.get("content-encoding") == "gzip"
    results.append(f"{'OK  ' if s == 200 and gz else 'FAIL'} {'MBTiles vector gzip':26s} -> {s} "
                   f"content-encoding={h.get('content-encoding')} (expect gzip passthrough)")

    # ---- archives ----
    probe("pmtiles unknown archive", "/pmtiles/cities", 404, contains="archive")
    probe("cog unknown layer", "/cog/nope", 404)

    # ---- 3dtiles ----
    s, h, b = req("/3dtiles")
    n = b.count(b'"name"')
    results.append(f"{'OK  ' if s == 200 and n == 1 else 'FAIL'} {'3dtiles list':26s} -> {s} "
                   f"({n} services configured)")
    probe("3dtiles entry", "/3dtiles/probe/tileset.json", 200, contains="geometricError")
    probe("3dtiles tile file", "/3dtiles/probe/Tile_0/tile.b3dm", 200)
    probe("3dtiles range", "/3dtiles/probe/Tile_0/tile.b3dm", 206,
          headers={"Range": "bytes=0-3"}, contains="b3dm")
    probe("3dtiles entry alias", "/3dtiles/probe", 200, contains="geometricError")
    probe("3dtiles unknown svc", "/3dtiles/nope/tileset.json", 404)
    probe("3dtiles missing file", "/3dtiles/probe/nope.b3dm", 404)
    probe("3dtiles traversal raw", "/3dtiles/probe/../../mapserver.yaml", 400,
          not_contains="cache_mib", note="(rejected, no config leak)")
    probe("3dtiles traversal enc", "/3dtiles/probe/..%2f..%2fmapserver.yaml", 400,
          not_contains="cache_mib")

    # ---- concurrency smoke: parallel tiles, mixed protocols ----
    import concurrent.futures as cf
    paths = (["/tms/1.0.0/world/3/%d/%d.png" % (i % 8, (i * 3) % 8) for i in range(24)]
             + ["/mvt/places/3/%d/%d.pbf" % (i % 8, (i * 3) % 8) for i in range(24)]
             + ["/wms?service=WMS&request=GetCapabilities&version=1.3.0"] * 8)
    t0 = time.time()
    with cf.ThreadPoolExecutor(16) as ex:
        codes = list(ex.map(lambda p: req(p)[0], paths))
    dt = time.time() - t0
    bad = [c for c in codes if c != 200]
    results.append(f"{'OK  ' if not bad else 'FAIL'} {'concurrent 56 req':26s} -> "
                   f"{len(codes) - len(bad)}/{len(codes)} x200 in {dt:.2f}s "
                   f"({len(codes) / dt:.0f} req/s)")

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    print("\n".join(results))
    fails = [r for r in results if r.startswith("FAIL")]
    print(f"\nPROBE RESULT: {'ALL PASS' if not fails else str(len(fails)) + ' FAILURES'}")
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
