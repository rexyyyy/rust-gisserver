"""rust-gisserver 性能基准：起服 -> 多场景压测 -> 输出 JSON 报告。

用法（在项目根目录）：
    python tools/bench_server.py --label before
    python tools/bench_server.py --label after

场景：
  A. WMS GetMap 512x256（串行 20 次，测渲染管线 CPU）
  B. TMS 冷瓦片（128 个不同瓦片，并发 16，测解码+渲染+编码全链路）
  C. TMS 热瓦片（同一瓦片，并发 16，output_cache 命中路径）
  D. MVT 冷瓦片（128 个不同瓦片，并发 16）
  E. MBTiles 冷瓦片（128 个不同瓦片，并发 16）
  F. /health 纯并发（128，并发 16，对照基线）
内存：每个场景后采样 RSS（tasklist）。
"""
import argparse
import http.client
import json
import shutil
import statistics
import subprocess
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "target" / "release" / "map-api.exe"
BENCH_DIR = Path(tempfile.gettempdir()) / "rms-bench"
HOST, PORT = "127.0.0.1", 18082

GETMAP = (
    "/wms?service=WMS&request=GetMap&version=1.3.0&layers=world"
    "&crs=CRS:84&bbox=100,36,108,40&width=512&height=256&format=image/png"
)
# 4x the pixels of GETMAP: the per-pixel projection cost (now the affine fast
# path) dominates here.
GETMAP_BIG = (
    "/wms?service=WMS&request=GetMap&version=1.3.0&layers=world"
    "&crs=CRS:84&bbox=100,36,108,40&width=1024&height=512&format=image/png"
)


def prepare_catalog():
    if BENCH_DIR.exists():
        shutil.rmtree(BENCH_DIR, ignore_errors=True)
    for sub in ("cog", "vector", "mbtiles"):
        (BENCH_DIR / sub).mkdir(parents=True, exist_ok=True)
    shutil.copy(ROOT / "config/cog/world.yaml", BENCH_DIR / "cog/world.yaml")
    shutil.copy(ROOT / "config/vector/places.yaml", BENCH_DIR / "vector/places.yaml")
    shutil.copy(ROOT / "config/mbtiles/test.yaml", BENCH_DIR / "mbtiles/test.yaml")
    (BENCH_DIR / "mapserver.yaml").write_text(
        "server:\n  host: 127.0.0.1\n  port: 18082\n  cache_mib: 64\n"
        f"catalog:\n  root: {BENCH_DIR.as_posix()}\n  backend: files\n",
        encoding="utf-8",
    )


def start_server():
    proc = subprocess.Popen(
        [str(EXE), "serve", "--config", str(BENCH_DIR / "mapserver.yaml")],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    for _ in range(80):
        try:
            c = http.client.HTTPConnection(HOST, PORT, timeout=2)
            c.request("GET", "/health")
            if c.getresponse().status == 200:
                c.close()
                return proc
            c.close()
        except OSError:
            pass
        time.sleep(0.25)
    proc.kill()
    raise SystemExit("server did not come up")


def rss_mib():
    """Working set of the map-api process via psapi (tasklist is unavailable
    in this sandbox)."""
    try:
        import ctypes
        from ctypes import wintypes

        class PMC(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        out = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq map-api.exe", "/FO", "CSV", "/NH"],
            capture_output=True, text=True, timeout=10,
        ).stdout
        pid = None
        for line in out.splitlines():
            parts = line.strip().strip('"').split('","')
            if parts and parts[0] == "map-api.exe" and len(parts) >= 2:
                pid = int(parts[1])
                break
        if pid is None:
            return None
        pmc = PMC()
        pmc.cb = ctypes.sizeof(pmc)
        h = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)
        if not h:
            return None
        try:
            if not ctypes.windll.psapi.GetProcessMemoryInfo(
                h, ctypes.byref(pmc), pmc.cb
            ):
                return None
            return round(pmc.WorkingSetSize / 1048576, 1)
        finally:
            ctypes.windll.kernel32.CloseHandle(h)
    except Exception:
        return None


def _get(path, timeout):
    c = http.client.HTTPConnection(HOST, PORT, timeout=timeout)
    t0 = time.perf_counter()
    c.request("GET", path)
    r = c.getresponse()
    body = r.read()
    dt = time.perf_counter() - t0
    c.close()
    return dt, r.status, len(body)


def run_concurrent(path_fn, total, conc, label, timeout=30):
    """path_fn(i) -> path（i 为请求序号，用于冷瓦片打散）。"""
    results: list = [None] * total
    lock = threading.Lock()
    nxt = [0]

    def runner():
        while True:
            with lock:
                i = nxt[0]
                if i >= total:
                    return
                nxt[0] = i + 1
            try:
                results[i] = _get(path_fn(i), timeout)
            except Exception as e:  # noqa: BLE001
                results[i] = (None, -1, repr(e))

    threads = [threading.Thread(target=runner) for _ in range(conc)]
    t0 = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = time.perf_counter() - t0
    return _stats(label, total, conc, wall, results)


def run_serial(path, times, label, timeout=120):
    results = []
    t0 = time.perf_counter()
    for _ in range(times):
        results.append(_get(path, timeout))
    wall = time.perf_counter() - t0
    return _stats(label, times, 1, wall, results)


def _stats(label, total, conc, wall, results):
    lat = sorted(r[0] for r in results if r[0] is not None)
    codes = {}
    for r in results:
        codes[r[1]] = codes.get(r[1], 0) + 1
    n = len(lat)
    return {
        "label": label, "total": total, "conc": conc,
        "qps": round(total / wall, 1), "codes": codes,
        "p50_ms": round(statistics.median(lat) * 1000, 2) if n else None,
        "p99_ms": round(lat[min(n - 1, int(n * 0.99))] * 1000, 2) if n else None,
        "max_ms": round(lat[-1] * 1000, 2) if n else None,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", default="run")
    args = ap.parse_args()

    prepare_catalog()
    proc = start_server()
    time.sleep(0.5)
    print(f"[{args.label}] server up (pid {proc.pid}), RSS {rss_mib()} MiB")

    def tms_cold(i):
        return f"/tms/1.0.0/world/5/{8 + i % 12}/{8 + i // 12}.png"

    def mvt_cold(i):
        return f"/mvt/places/5/{8 + i % 12}/{8 + i // 12}.pbf"

    def mbt_cold(i):
        return f"/mbtiles/test/1/{i % 2}/{(i // 2) % 2}"

    scenarios = [
        ("A. WMS GetMap 512x256 serial x20",
         lambda: run_serial(GETMAP, 20, "getmap_512")),
        ("A2. WMS GetMap 1024x512 serial x10",
         lambda: run_serial(GETMAP_BIG, 10, "getmap_1024")),
        ("B. TMS cold tiles 128 conc16",
         lambda: run_concurrent(tms_cold, 128, 16, "tms_cold")),
        ("C. TMS hot tile 256 conc16",
         lambda: run_concurrent(lambda i: "/tms/1.0.0/world/5/16/12.png",
                                256, 16, "tms_hot")),
        ("D. MVT cold tiles 128 conc16",
         lambda: run_concurrent(mvt_cold, 128, 16, "mvt_cold")),
        ("E. MBTiles cold tiles 128 conc16",
         lambda: run_concurrent(mbt_cold, 128, 16, "mbt_cold")),
        ("F. health 128 conc16",
         lambda: run_concurrent(lambda i: "/health", 128, 16, "health_conc")),
    ]

    report = []
    for name, fn in scenarios:
        s = fn()
        rss = rss_mib()
        s["rss_mib_after"] = rss
        report.append(s)
        print(f"[{args.label}] {name:32s} qps={s['qps']:>8} "
              f"p50={str(s['p50_ms']):>8}ms p99={str(s['p99_ms']):>8}ms "
              f"max={str(s['max_ms']):>8}ms codes={s['codes']} rss={rss}MiB")

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    out = ROOT / "tools" / f"bench_{args.label}.json"
    out.write_text(json.dumps({"label": args.label, "scenarios": report},
                              indent=2), encoding="utf-8")
    print(f"[{args.label}] report -> {out}")


if __name__ == "__main__":
    main()
