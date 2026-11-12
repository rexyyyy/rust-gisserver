# -*- coding: utf-8 -*-
"""P2 stress probe: 100k-row PostGIS table through r-mapserver.

Steps:
  1. create/drop table `stress_100k` (100k points, EPSG:4543) in the gis DB
  2. write a temporary catalog (mapserver.yaml + postgis/stress.yaml) with the
     foyer DISK cache enabled, serving on port 8081
  3. start map-api against it, then probe:
       - OGC API - Features items: numberMatched == 100000, bbox filter, paging
       - WFS GetFeature: bbox + OGC Filter pushdown (PropertyIsGreaterThan)
       - MVT dynamic tiles, both CRS interfaces (3857 / 4543), foyer disk hit
       - WMS GetMap PNG render
  4. report timings, drop the table, stop the server
"""
import io
import json
import math
import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.request

import psycopg2

OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))

PG_DSN = "dbname=gis user=postgres password=123456 host=localhost port=5434"
BASE = "http://127.0.0.1:8081"
ROOT = os.path.join(os.path.dirname(__file__), "..", ".tmp-stress")
CFG_DIR = os.path.join(ROOT, "config")
TABLE = "stress_100k"
N_ROWS = 100_000

passed = 0
failed = 0

def check(name, ok, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"  [PASS] {name} {detail}")
    else:
        failed += 1
        print(f"  [FAIL] {name} {detail}")

def http(path, timeout=60):
    t0 = time.perf_counter()
    try:
        with OPENER.open(BASE + path, timeout=timeout) as r:
            body = r.read()
            ct = r.headers.get("content-type", "")
        return (time.perf_counter() - t0) * 1000.0, body, ct
    except urllib.error.HTTPError as e:
        body = e.read()
        print(f"    [HTTP {e.code}] {path[:120]} -> {body[:300].decode('utf-8', 'replace')}")
        raise

# --- geometry helpers (mirror the server's grid math) ---
PROJ_HALF = 20037508.342789244
# data extent in EPSG:4543 (CM 102E, false easting 500000)
DATA_X0, DATA_X1 = 500000.0, 556000.0
DATA_Y0, DATA_Y1 = 2760000.0, 2820000.0
# equivalent 4326 bbox (lon ~102.0..102.55, lat ~24.9..25.45)
BBOX_4326 = "102.00,25.00,102.30,25.30"

def wm_tile(z, lon, lat):
    n = 2 ** z
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(lat)
    y = int((1.0 - math.asinh(math.tan(lat_rad)) / math.pi) / 2.0 * n)
    return x, y

def proj_tile(z, x_m, y_m):
    span = 2.0 * PROJ_HALF / (2 ** z)
    xt = int((x_m + PROJ_HALF) // span)
    yt = int((PROJ_HALF - y_m) // span)  # XYZ row order: row 0 = top
    return xt, yt

def build_table(cur):
    cur.execute(f"DROP TABLE IF EXISTS {TABLE}")
    cur.execute(f"""
        CREATE TABLE {TABLE} (
            gid bigserial PRIMARY KEY,
            val integer NOT NULL,
            name text NOT NULL,
            geom geometry(Point, 4543) NOT NULL
        )
    """)
    # deterministic pseudo-random spread (~56km x ~60km box); literal '%'
    # must be doubled for psycopg2 paramstyle
    cur.execute(f"""
        INSERT INTO {TABLE} (val, name, geom)
        SELECT
            (i * 7919) %% 1000 + 1,
            'f' || i,
            ST_SetSRID(ST_MakePoint(
                500000 + ((i::bigint * 104729) %% 56000)::float8,
                2760000 + ((i::bigint * 15485863) %% 60000)::float8
            ), 4543)
        FROM generate_series(1, %s) AS i
    """, (N_ROWS,))
    cur.execute(f"ANALYZE {TABLE}")

def write_config():
    os.makedirs(os.path.join(CFG_DIR, "postgis"), exist_ok=True)
    io.open(os.path.join(CFG_DIR, "mapserver.yaml"), "w", encoding="utf-8").write(f"""
server:
  host: 127.0.0.1
  port: 8081
  cache_mib: 64
  cache_ttl_secs: 300
  cache_stats_secs: 0
  disk_cache:
    dir: {os.path.join(ROOT, 'cache').replace(chr(92), '/')}
    capacity_mib: 256
catalog:
  root: {CFG_DIR.replace(chr(92), '/')}
""")
    io.open(os.path.join(CFG_DIR, "postgis", "stress.yaml"), "w", encoding="utf-8").write(f"""
name: stress_100k
title: 100k stress table
source:
  type: postgis
  dsn: postgres://postgres:123456@localhost:5434/gis
  table: {TABLE}
  srid: 4543
  geometry_column: geom
  id_column: gid
  properties: [val, name]
  maxzoom: 22
  limit: 5000
src_crs: EPSG:4543
grids: [WebMercatorQuad]
""")

def main():
    conn = psycopg2.connect(PG_DSN)
    conn.autocommit = True
    cur = conn.cursor()
    print(f"[1] building {N_ROWS}-row table {TABLE} ...")
    t0 = time.perf_counter()
    build_table(cur)
    print(f"    built in {time.perf_counter()-t0:.1f}s")
    cur.execute(f"SELECT count(*) FROM {TABLE}")
    n = cur.fetchone()[0]
    check("table row count", n == N_ROWS, f"({n})")

    print("[2] writing temp catalog ...")
    write_config()

    server = None
    try:
        print("[3] starting map-api :8081 ...")
        exe = os.path.join(os.path.dirname(__file__), "..",
                           "target", "debug", "map-api.exe")
        server = subprocess.Popen(
            [exe, "serve", "--config", os.path.join(CFG_DIR, "mapserver.yaml")],
            cwd=os.path.join(os.path.dirname(__file__), ".."),
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
        )
        # wait for readiness
        ready = False
        for _ in range(120):
            time.sleep(0.5)
            try:
                _, body, _ = http("/health", timeout=2)
                ready = True
                break
            except Exception:
                pass
        check("server ready", ready)
        if not ready:
            return

        # --- OAF items: full count ---
        ms, body, _ = http("/ogc/features/v1/collections/stress_100k/items?limit=100")
        doc = json.loads(body)
        check("oaf numberMatched == 100000", doc.get("numberMatched") == N_ROWS,
              f"(matched={doc.get('numberMatched')}, returned={doc.get('numberReturned')}, {ms:.0f}ms)")
        check("oaf returned == 100", doc.get("numberReturned") == 100)
        check("oaf features carry id", all("id" in f for f in doc.get("features", [])))

        # --- OAF bbox filter ---
        ms, body, _ = http(f"/ogc/features/v1/collections/stress_100k/items?limit=50&bbox={BBOX_4326}")
        doc = json.loads(body)
        check("oaf bbox filter works", doc.get("numberMatched", 0) > 0
              and doc.get("numberReturned", 0) <= 50,
              f"(matched={doc.get('numberMatched')}, {ms:.0f}ms)")

        # --- OAF filter pushdown ---
        flt_oaf = ("<Filter><PropertyIsGreaterThan><PropertyName>val</PropertyName>"
                   "<Literal>990</Literal></PropertyIsGreaterThan></Filter>")
        from urllib.parse import quote
        ms, body, _ = http("/ogc/features/v1/collections/stress_100k/items?limit=50&filter=" + quote(flt_oaf))
        doc = json.loads(body)
        feats = doc.get("features", [])
        check("oaf filter pushdown (val > 990)",
              len(feats) > 0 and all(f["properties"]["val"] > 990 for f in feats),
              f"(returned={len(feats)}, matched={doc.get('numberMatched')}, {ms:.0f}ms)")

        # --- OAF paging ---
        ms, body, _ = http("/ogc/features/v1/collections/stress_100k/items?limit=100&offset=99950")
        doc = json.loads(body)
        check("oaf offset paging", doc.get("numberReturned") == 50
              and doc.get("features", [{}])[0].get("id") == 99951,
              f"(first id={doc.get('features', [{}])[0].get('id')})")

        # --- WFS GetFeature bbox + Filter pushdown ---
        # WFS 2.0 + default srsName EPSG:4326: bbox is northing-first (lat,lon).
        latlon = "25.00,102.00,25.30,102.30"
        ms, body, _ = http("/wfs?service=WFS&version=2.0.0&request=GetFeature"
                           "&typeNames=stress_100k&count=200&outputFormat=application/json"
                           f"&bbox={latlon}")
        doc = json.loads(body)
        matched = doc.get("numberMatched", doc.get("totalFeatures", 0))
        check("wfs bbox query", 0 < matched < N_ROWS, f"(matched={matched}, {ms:.0f}ms)")

        flt = ("<Filter><PropertyIsGreaterThan><PropertyName>val</PropertyName>"
               "<Literal>990</Literal></PropertyIsGreaterThan></Filter>")
        from urllib.parse import quote
        ms, body, _ = http("/wfs?service=WFS&version=2.0.0&request=GetFeature"
                           "&typeNames=stress_100k&count=200&outputFormat=application/json"
                           "&filter=" + quote(flt))
        doc = json.loads(body)
        feats = doc.get("features", [])
        check("wfs filter pushdown (val > 990)",
              len(feats) > 0 and all(f["properties"]["val"] > 990 for f in feats),
              f"(returned={len(feats)}, matched={doc.get('numberMatched')}, {ms:.0f}ms)")

        flt_bad = "<Filter><BogusOperator/></Filter>"
        try:
            http("/wfs?service=WFS&version=2.0.0&request=GetFeature"
                 "&typeNames=stress_100k&outputFormat=application/json&filter=" + quote(flt_bad))
            check("wfs bad filter rejected", False, "(got 200)")
        except urllib.error.HTTPError as e:
            check("wfs bad filter rejected", e.code == 400, f"(HTTP {e.code})")

        # --- WFS featureId ---
        ms, body, _ = http("/wfs?service=WFS&version=2.0.0&request=GetFeature"
                           "&typeNames=stress_100k&outputFormat=application/json&featureId=42")
        doc = json.loads(body)
        fs = doc.get("features", [])
        check("wfs featureId=42", len(fs) == 1 and fs[0].get("id") == 42)

        # --- MVT both CRS interfaces, cold + foyer cache hit ---
        # web-mercator tile computed from the data extent (mid point)
        wx, wy = wm_tile(14, 102.2, 25.2)
        ms, body, _ = http(f"/mvt/stress_100k/14/{wx}/{wy}.pbf")
        check("mvt 3857 z14 served", len(body) > 100, f"(tile {wx}/{wy}, {len(body)}B, {ms:.0f}ms)")
        ms2, body2, _ = http(f"/mvt/stress_100k/14/{wx}/{wy}.pbf")
        check("mvt 3857 cache hit fast", ms2 < 50, f"({ms2:.1f}ms, {len(body2)}B)")

        # projected (EPSG:4543) grid tile computed from the data extent
        px, py = proj_tile(14, (DATA_X0 + DATA_X1) / 2, (DATA_Y0 + DATA_Y1) / 2)
        ms, body, _ = http(f"/mvt/stress_100k/4543/14/{px}/{py}.pbf")
        check("mvt 4543 z14 served", len(body) > 100, f"(tile {px}/{py}, {len(body)}B, {ms:.0f}ms)")
        ms2, body2, _ = http(f"/mvt/stress_100k/4543/14/{px}/{py}.pbf")
        check("mvt 4543 cache hit fast", ms2 < 50, f"({ms2:.1f}ms, {len(body2)}B)")

        # --- WMS GetMap ---
        ms, body, ct = http("/wms?service=WMS&version=1.3.0&request=GetMap"
                            "&layers=stress_100k&crs=EPSG:4326&bbox=24.90,102.00,25.45,102.55"
                            "&width=512&height=512&format=image/png")
        check("wms getmap png", body[:8] == b"\x89PNG\r\n\x1a\n", f"({len(body)}B, {ms:.0f}ms)")

        # --- capabilities advertise the pg layer ---
        ms, body, _ = http("/wfs?service=WFS&request=GetCapabilities")
        check("wfs caps advertise stress_100k", b"stress_100k" in body)
        ms, body, _ = http("/ogc/features/v1/collections")
        check("oaf collections advertise stress_100k", b"stress_100k" in body)

        # --- cache admin reports foyer disk tier ---
        ms, body, _ = http("/api/cache")
        doc = json.loads(body)
        check("cache stats expose disk tier", doc.get("disk") is not None)
    finally:
        if server:
            server.send_signal(signal.CTRL_BREAK_EVENT)
            try:
                server.wait(timeout=10)
            except Exception:
                server.kill()
        cur.execute(f"DROP TABLE IF EXISTS {TABLE}")
        conn.close()
        shutil.rmtree(ROOT, ignore_errors=True)

    print(f"\n===== pg_stress: {passed} passed, {failed} failed =====")
    sys.exit(1 if failed else 0)

if __name__ == "__main__":
    import urllib.error
    main()
