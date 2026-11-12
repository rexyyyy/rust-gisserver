"""Post-fix verification: startup pre-flight, compression policy, protocol regression."""
import http.client
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(r"D:\mycomputer\documents\gis-wt\thirdparty\rust-gisserver")
EXE = ROOT / "target" / "release" / "map-api.exe"
GOOD = Path("E:/tmp/rms-verify")
BAD = Path("E:/tmp/rms-badcfg")
ALLBAD = Path("E:/tmp/rms-allbad")
HOST, PORT = "127.0.0.1", 18083


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


def prepare_good():
    if GOOD.exists():
        shutil.rmtree(GOOD, ignore_errors=True)
    for sub in ("cog", "vector", "mbtiles"):
        (GOOD / sub).mkdir(parents=True, exist_ok=True)
    shutil.copy(ROOT / "config/cog/world.yaml", GOOD / "cog/world.yaml")
    shutil.copy(ROOT / "config/vector/places.yaml", GOOD / "vector/places.yaml")
    shutil.copy(ROOT / "config/mbtiles/test.yaml", GOOD / "mbtiles/test.yaml")
    (GOOD / "mapserver.yaml").write_text(
        "server:\n  host: 127.0.0.1\n  port: 18083\n  cache_mib: 64\n"
        "catalog:\n  root: E:/tmp/rms-verify\n  backend: files\n",
        encoding="utf-8",
    )


def prepare_bad():
    if BAD.exists():
        shutil.rmtree(BAD, ignore_errors=True)
    for sub in ("cog", "vector"):
        (BAD / sub).mkdir(parents=True, exist_ok=True)
    # Two broken paths (typo'd drive letter + plain missing file) plus one
    # good layer: policy since commit 3b42de5 (2026-10-23) is skip-and-warn --
    # every broken layer is listed, the good layers still serve.
    shutil.copy(ROOT / "config/cog/world.yaml", BAD / "cog/world.yaml")
    (BAD / "cog/broken1.yaml").write_text(
        "name: broken1\nsource:\n  type: cog\n"
        "  path: D:\\mycomputor\\documents\\gis-wt\\nope\\x.tif\n",
        encoding="utf-8",
    )
    (BAD / "vector/broken2.yaml").write_text(
        "name: broken2\nsource:\n  type: vector\n  path: E:/tmp/does_not_exist.geojson\n",
        encoding="utf-8",
    )
    (BAD / "mapserver.yaml").write_text(
        "server:\n  host: 127.0.0.1\n  port: 18083\n  cache_mib: 64\n"
        "catalog:\n  root: E:/tmp/rms-badcfg\n  backend: files\n",
        encoding="utf-8",
    )


def prepare_all_bad():
    if ALLBAD.exists():
        shutil.rmtree(ALLBAD, ignore_errors=True)
    for sub in ("cog",):
        (ALLBAD / sub).mkdir(parents=True, exist_ok=True)
    (ALLBAD / "cog/broken1.yaml").write_text(
        "name: broken1\nsource:\n  type: cog\n"
        "  path: D:\\mycomputor\\documents\\gis-wt\\nope\\x.tif\n",
        encoding="utf-8",
    )
    (ALLBAD / "cog/broken2.yaml").write_text(
        "name: broken2\nsource:\n  type: cog\n  path: E:/tmp/also_missing.tif\n",
        encoding="utf-8",
    )
    (ALLBAD / "mapserver.yaml").write_text(
        "server:\n  host: 127.0.0.1\n  port: 18083\n  cache_mib: 64\n"
        "catalog:\n  root: E:/tmp/rms-allbad\n  backend: files\n",
        encoding="utf-8",
    )


def main():
    ok = True

    # ---- 1a) startup pre-flight: partial catalog must LIST every broken
    # layer, skip them, and still serve the good ones (policy 3b42de5).
    prepare_bad()
    proc = subprocess.Popen(
        [str(EXE), "serve", "--config", str(BAD / "mapserver.yaml")],
        # 子进程日志是 UTF-8；中文 Windows 上 text=True 会按本地 GBK 解码而抛
        # UnicodeDecodeError（out 变成 None），必须显式指定编码。
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding="utf-8", errors="replace",
    )
    try:
        out, _ = proc.communicate(timeout=15)
    except subprocess.TimeoutExpired:
        proc.kill()
        out, _ = proc.communicate()
    print("=== 1a) startup pre-flight (partial catalog) ===")
    print(out.strip()[-700:])
    lists_both = "broken1" in out and "broken2" in out
    says_missing = "data file not found" in out
    keeps_serving = "service continues" in out
    print(f"lists both layers: {lists_both} | 'data file not found': {says_missing} | "
          f"keeps serving: {keeps_serving}")
    ok &= lists_both and says_missing and keeps_serving

    # ---- 1b) all-broken catalog must still fail fast (nothing to serve) ----
    prepare_all_bad()
    proc = subprocess.Popen(
        [str(EXE), "serve", "--config", str(ALLBAD / "mapserver.yaml")],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding="utf-8", errors="replace",
    )
    try:
        out, _ = proc.communicate(timeout=15)
        allbad_failed = proc.returncode != 0
    except subprocess.TimeoutExpired:
        proc.kill()
        out, _ = proc.communicate()
        allbad_failed = False
    print("\n=== 1b) startup pre-flight (all-broken catalog) ===")
    print(out.strip()[-500:])
    allbad_lists = "broken1" in out and "broken2" in out
    print(f"exit!=0: {allbad_failed} | lists both: {allbad_lists}")
    ok &= allbad_failed and allbad_lists

    # ---- 2) good catalog: compression policy + protocol regression ----
    prepare_good()
    proc = subprocess.Popen(
        [str(EXE), "serve", "--config", str(GOOD / "mapserver.yaml")],
        stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT,
    )
    if not wait_up(proc):
        print("server did not start")
        return 1
    print("\n=== 2) compression policy ===")
    gzip_hdr = {"Accept-Encoding": "gzip"}
    s, h, _ = req("/tms/1.0.0/world/2/1/1.png", gzip_hdr)
    print(f"PNG tile  +gzip -> {s} content-encoding={h.get('content-encoding')} "
          f"(expect None: skipped)")
    ok &= s == 200 and "content-encoding" not in h
    s, h, _ = req("/mvt/places/2/1/1.pbf", gzip_hdr)
    print(f"MVT tile  +gzip -> {s} content-encoding={h.get('content-encoding')} (expect None)")
    ok &= s == 200 and "content-encoding" not in h
    s, h, b = req("/wms?service=WMS&request=GetCapabilities&version=1.3.0", gzip_hdr)
    print(f"WMS caps  +gzip -> {s} content-encoding={h.get('content-encoding')} "
          f"(expect gzip: text still compresses, {len(b)}B wire)")
    ok &= s == 200 and h.get("content-encoding") == "gzip"

    print("\n=== 3) protocol regression ===")
    cases = [
        ("health", "/health", 200),
        ("WMS caps", "/wms?service=WMS&request=GetCapabilities&version=1.3.0", 200),
        ("WMTS caps", "/wmts?request=GetCapabilities", 200),
        ("TMS tile", "/tms/1.0.0/world/2/1/1.png", 200),
        ("TMS geo grid", "/tms/1.0.0/world/WorldCRS84Quad/2/1/1.png", 200),
        ("XYZ tile", "/tiles/world/2/1/1.png", 200),
        ("WMTS REST", "/wmts/rest/world/default/WebMercatorQuad/2/1/1.png", 200),
        ("WMTS KVP", "/wmts?request=GetTile&layer=world&style=default"
                    "&tilematrixset=WebMercatorQuad&tilematrix=2&tilerow=1&tilecol=1"
                    "&format=image/png", 200),
        ("MVT tile", "/mvt/places/2/1/1.pbf", 200),
        ("MVT tilejson", "/mvt/places/tilejson.json", 200),
        ("WMS GetMap", "/wms?service=WMS&request=GetMap&version=1.3.0&layers=world"
                      "&crs=CRS:84&bbox=100,36,108,40&width=512&height=256"
                      "&format=image/png", 200),
        ("WMS GetFeatureInfo", "/wms?service=WMS&request=GetFeatureInfo&version=1.3.0"
                              "&query_layers=world&crs=CRS:84&bbox=100,36,108,40"
                              "&width=512&height=256&i=256&j=128", 200),
        ("MBTiles info", "/mbtiles/test", 200),
        ("MBTiles tile", "/mbtiles/test/1/1/0", 200),
        ("MBTiles 404", "/mbtiles/test/6/50/36", 404),
        ("COG range", None, 206),  # special: needs Range header
    ]
    for name, path, expect in cases:
        if name == "COG range":
            s, h, _ = req("/cog/world", {"Range": "bytes=0-99"})
        else:
            s, h, _ = req(path)
        mark = "OK " if s == expect else "FAIL"
        if s != expect:
            ok = False
        print(f"{mark} {name:20s} -> {s} (expect {expect})")

    # ETag/304 on a tile
    s, h, _ = req("/tms/1.0.0/world/3/2/3.png")
    etag = h.get("etag")
    s2, _, _ = req("/tms/1.0.0/world/3/2/3.png", {"If-None-Match": etag or ""})
    print(f"{'OK ' if s2 == 304 else 'FAIL'} ETag/304             -> {s2} (expect 304)")
    ok &= s2 == 304

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
    print("\nRESULT:", "ALL PASS" if ok else "FAILURES PRESENT")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
