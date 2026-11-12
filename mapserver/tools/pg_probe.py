#!/usr/bin/env python3
"""PostGIS dual-CRS dynamic vector tile probe.

Run against a live map-api started with `config/mapserver.yaml` (the
`config/postgis/wuhua.yaml` layer must be present and the PostGIS database
reachable).  Checks both coordinate interfaces:

* EPSG:3857 (WebMercatorQuad)  -- /mvt/{layer}/{z}/{x}/{y}.pbf
* EPSG:4543 (source projection)-- /mvt/{layer}/4543/{z}/{x}/{y}.pbf
plus TileJSON per interface, caching behaviour and error paths.
"""

import json
import math
import sys
import time
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8080"
LAYER = "wuhua_buildings"
SRID = 4543
HALF = 20037508.342789244
# Wuhua building centroid in EPSG:4543 map units / EPSG:4326 degrees.
PX, PY = 562011.49, 2773995.81
LON, LAT = 102.66, 25.22

passed = failed = 0


def check(name, ok, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {name}")
    else:
        failed += 1
        print(f"  FAIL  {name}  {detail}")


# A system proxy would intercept localhost traffic (adding 100-300 ms per
# request and occasionally 502s); build an opener that never proxies.
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def get(path):
    req = urllib.request.Request(BASE + path)
    t0 = time.time()
    try:
        r = OPENER.open(req, timeout=60)
        return r.status, r.read(), time.time() - t0
    except urllib.error.HTTPError as e:
        return e.code, e.read(), time.time() - t0
    except Exception as e:  # noqa: BLE001
        return 0, str(e).encode(), time.time() - t0


def webmercator_tile(lon, lat, z):
    n = 2**z
    return int((lon + 180) / 360 * n), int(
        (1 - math.log(math.tan(math.radians(lat)) + 1 / math.cos(math.radians(lat))) / math.pi)
        / 2 * n
    )


def projected_tile(px, py, z):
    ts = 2 * HALF / 2**z
    return int((px + HALF) / ts), int((HALF - py) / ts)


def probe_tilejson():
    print("== TileJSON ==")
    s, b, _ = get(f"/mvt/{LAYER}/tilejson.json")
    d = json.loads(b) if s == 200 else {}
    check("tilejson 3857 status", s == 200, f"HTTP {s}")
    check("tilejson 3857 bounds in 4326", 100 < d.get("bounds", [9e9])[0] < 105, str(d.get("bounds"))[:60])
    check("tilejson 3857 vector_layers", d.get("vector_layers", [{}])[0].get("id") == LAYER, "")
    check("tilejson 3857 no crs member", "crs" not in d, str(d.get("crs")))

    s, b, _ = get(f"/mvt/{LAYER}/{SRID}/tilejson.json")
    d = json.loads(b) if s == 200 else {}
    check(f"tilejson {SRID} status", s == 200, f"HTTP {s}")
    check("tilejson crs member", d.get("crs") == f"EPSG:{SRID}", str(d.get("crs")))
    b4 = d.get("bounds", [0, 0, 0, 0])
    check("tilejson bounds in map units", 500000 < b4[0] < 600000 and 2700000 < b4[1] < 2900000, str(b4))
    fields = d.get("vector_layers", [{}])[0].get("fields", {})
    check("tilejson field types", fields.get("height") == "Number", str(fields))
    check("tilejson 4543 tiles template", f"/{SRID}/{{z}}/{{x}}/{{y}}.pbf" in d.get("tiles", [""])[0], str(d.get("tiles")))

    s, _, _ = get(f"/mvt/{LAYER}/9999/tilejson.json")
    check("tilejson unknown grid 400", s == 400, f"HTTP {s}")


def probe_tiles():
    print("== Tiles ==")
    # Every interface/zoom: status, non-empty, cache hit fast.
    for z in (10, 12, 14, 16, 18):
        gx, gy = projected_tile(PX, PY, z)
        s1, b1, t1 = get(f"/mvt/{LAYER}/{SRID}/{z}/{gx}/{gy}.pbf")
        check(f"{SRID} z{z} 200 non-empty", s1 == 200 and len(b1) > 0, f"HTTP {s1} {len(b1)}B")
        s2, _, t2 = get(f"/mvt/{LAYER}/{SRID}/{z}/{gx}/{gy}.pbf")
        check(f"{SRID} z{z} cache hit <50ms", s2 == 200 and t2 < 0.05, f"{t2:.3f}s")

    for z in (12, 14, 16):
        x, y = webmercator_tile(LON, LAT, z)
        s1, b1, _ = get(f"/mvt/{LAYER}/{z}/{x}/{y}.pbf")
        check(f"3857 z{z} 200 non-empty", s1 == 200 and len(b1) > 0, f"HTTP {s1} {len(b1)}B")

    # Explicit 3857 alias works identically.
    x, y = webmercator_tile(LON, LAT, 14)
    s, _, _ = get(f"/mvt/{LAYER}/3857/14/{x}/{y}.pbf")
    check("3857 explicit alias 200", s == 200, f"HTTP {s}")

    # Both tiles are valid vector tiles: leading layer message field
    # (tag 3, wire type 2 -> byte 0x1a) and a layer name follows.
    gx, gy = projected_tile(PX, PY, 14)
    _, b1, _ = get(f"/mvt/{LAYER}/{SRID}/{gx and 14}/{gx}/{gy}.pbf")
    check("MVT magic (proj)", b1[:1] in (b"\x1a", b"\x12"), b1[:8].hex())
    x, y = webmercator_tile(LON, LAT, 14)
    _, b2, _ = get(f"/mvt/{LAYER}/14/{x}/{y}.pbf")
    check("MVT magic (webmerc)", b2[:1] in (b"\x1a", b"\x12"), b2[:8].hex())

    # Error paths.
    s, _, _ = get(f"/mvt/{LAYER}/{SRID}/1/1/1.pbf")  # ocean, far from Kunming
    check("empty tile 404", s == 404, f"HTTP {s}")
    s, _, _ = get(f"/mvt/{LAYER}/9999/1/1/1.pbf")
    check("unknown grid 400", s == 400, f"HTTP {s}")
    s, _, _ = get("/mvt/no_such_layer/1/1/1.pbf")
    check("unknown layer 404", s == 404, f"HTTP {s}")


SQL_LAYER = "wuhua_tall"  # P3-5 demo: postgis `from` view layer


def probe_p3():
    print("== P3: simplify / sortby / sql-view layer ==")
    # P3-1 simplify: both interfaces below under_zoom=12 still serve.
    for z in (9, 10, 11):
        gx, gy = projected_tile(PX, PY, z)
        s, b, _ = get(f"/mvt/{LAYER}/{SRID}/{z}/{gx}/{gy}.pbf")
        check(f"simplify proj z{z} 200 non-empty", s == 200 and len(b) > 0, f"HTTP {s} {len(b)}B")
        x, y = webmercator_tile(LON, LAT, z)
        s, b, _ = get(f"/mvt/{LAYER}/{z}/{x}/{y}.pbf")
        check(f"simplify merc z{z} 200 non-empty", s == 200 and len(b) > 0, f"HTTP {s} {len(b)}B")
    # At/above the threshold the plain path must still work.
    gx, gy = projected_tile(PX, PY, 12)
    s, b, _ = get(f"/mvt/{LAYER}/{SRID}/12/{gx}/{gy}.pbf")
    check("no-simplify z12 200 non-empty", s == 200 and len(b) > 0, f"HTTP {s}")

    # P3-5 sql/view layer: TileJSON + tiles.
    s, b, _ = get(f"/mvt/{SQL_LAYER}/tilejson.json")
    d = json.loads(b) if s == 200 else {}
    check("sqlview tilejson 200", s == 200, f"HTTP {s}")
    check("sqlview tilejson fields", d.get("vector_layers", [{}])[0].get("fields", {}).get("height") == "Number", str(d.get("vector_layers")))
    x, y = webmercator_tile(LON, LAT, 14)
    s, b, _ = get(f"/mvt/{SQL_LAYER}/14/{x}/{y}.pbf")
    check("sqlview merc z14 200", s == 200, f"HTTP {s} {len(b)}B")
    gx, gy = projected_tile(PX, PY, 14)
    s, b, _ = get(f"/mvt/{SQL_LAYER}/{SRID}/14/{gx}/{gy}.pbf")
    check("sqlview proj z14 200", s == 200, f"HTTP {s}")

    # OAF on the sql/view layer.
    s, b, _ = get(f"/ogc/features/v1/collections/{SQL_LAYER}/items?limit=3")
    d = json.loads(b) if s == 200 else {}
    check("sqlview items 200", s == 200, f"HTTP {s}")
    check("sqlview items has features", len(d.get("features", [])) > 0, str(d)[:80])

    # P3-2 sortby on the base layer (OAF).
    base_items = f"/ogc/features/v1/collections/{LAYER}/items"
    s, b, _ = get(f"{base_items}?limit=5&sortby=height")
    d = json.loads(b) if s == 200 else {}
    hs = [f["properties"].get("height") for f in d.get("features", [])]
    hs = [h for h in hs if h is not None]
    check("sortby asc 200", s == 200, f"HTTP {s}")
    check("sortby asc ordered", hs == sorted(hs), str(hs))
    s, b, _ = get(f"{base_items}?limit=5&sortby=-height")
    d = json.loads(b) if s == 200 else {}
    hs = [f["properties"].get("height") for f in d.get("features", [])]
    hs = [h for h in hs if h is not None]
    check("sortby desc 200", s == 200, f"HTTP {s}")
    check("sortby desc ordered", hs == sorted(hs, reverse=True), str(hs))
    s, _, _ = get(f"{base_items}?limit=5&sortby=no_such_col")
    check("sortby invalid 400", s == 400, f"HTTP {s}")


def main():
    print(f"PostGIS dual-CRS probe against {BASE} (layer {LAYER})")
    probe_tilejson()
    probe_tiles()
    probe_p3()
    print(f"\n{passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
