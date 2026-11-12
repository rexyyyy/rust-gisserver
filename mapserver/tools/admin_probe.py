#!/usr/bin/env python3
"""map-api admin surface probe (files mode, no PG registry).

Covers: /health, /admin/services, state stop/restore round-trip (immediate
effect), /admin/reload, /api/usage (memory counters). Starts its own
map-api instance on a scratch port and kills it at the end.

Usage: python tools/admin_probe.py [--port 8095]
"""
import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
SIDECAR = os.path.join(ROOT, "config", "service-state.json")

PASS = 0
FAIL = 0


def check(name, cond, detail=""):
    global PASS, FAIL
    mark = "PASS" if cond else "FAIL"
    if cond:
        PASS += 1
    else:
        FAIL += 1
    print(f"[{mark}] {name}" + (f"  {detail}" if detail else ""))


def req(url, method="GET", body=None, headers=None):
    h = dict(headers or {})
    if body is not None:
        h.setdefault("Content-Type", "application/json")
    r = urllib.request.Request(url, method=method,
                               data=body.encode() if body else None,
                               headers=h)
    try:
        with urllib.request.urlopen(r, timeout=10) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, str(e)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8095)
    ap.add_argument("--exe", default=os.path.join(ROOT, "target", "release", "map-api.exe"))
    ap.add_argument("--dsn", default=None,
                    help="PG registry DSN (PG mode). Default: files sidecar mode.")
    args = ap.parse_args()
    base = f"http://127.0.0.1:{args.port}"

    if os.path.exists(SIDECAR):
        os.remove(SIDECAR)  # probe starts from a clean slate

    config_arg = "config/mapserver.yaml"
    tmp_cfg = None
    if args.dsn:
        text = open(os.path.join(ROOT, "config", "mapserver.yaml"), encoding="utf-8").read()
        text += f"\nadmin:\n  dsn: {args.dsn}\n"
        tmp_cfg = os.path.join(ROOT, "config", "mapserver.probe-admin.yaml")
        with open(tmp_cfg, "w", encoding="utf-8") as f:
            f.write(text)
        config_arg = "config/mapserver.probe-admin.yaml"

    env = dict(os.environ)
    proc = subprocess.Popen(
        [args.exe, "serve", "--config", config_arg,
         "--listen", f"127.0.0.1:{args.port}"],
        cwd=ROOT, env=env,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
    )
    try:
        # wait for listen
        up = False
        for _ in range(60):
            st, _ = req(f"{base}/health")
            up = st == 200
            if up:
                break
            time.sleep(0.5)
        check("server up (/health 200)", up)

        st, body = req(f"{base}/admin/services")
        doc = json.loads(body) if st == 200 else {}
        svcs = doc.get("services", [])
        check("GET /admin/services 200", st == 200)
        expect_mode = "pg" if args.dsn else "files"
        check(f"registry_mode={expect_mode}", doc.get("registry_mode") == expect_mode,
              str(doc.get("registry_mode")))
        check("catalog scanned", len(svcs) > 0, f"{len(svcs)} services")
        if not svcs:
            return

        # usage middleware counts a service request
        layer = next((s for s in svcs if s["kind"] != "3dtiles"), svcs[0])
        name, kind = layer["name"], layer["kind"]
        req(f"{base}/tiles/{name}/0/0/0.png")
        req(f"{base}/tiles/{name}/0/0/0.png")
        if args.dsn:
            print("  (pg mode: waiting 35s for the usage flush task...)")
            time.sleep(35)
        st, body = req(f"{base}/api/usage?period=day")
        doc = json.loads(body) if st == 200 else {}
        rows = doc.get("rows", [])
        hit = next((r for r in rows if r.get("name") == name), None)
        check("GET /api/usage 200", st == 200)
        check("usage counted for request", hit is not None and hit["hits"] >= 2,
              json.dumps(hit, ensure_ascii=False) if hit else "-")
        expect_src = "pg" if args.dsn else "memory"
        check(f"usage source={expect_src}", doc.get("source") == expect_src, str(doc.get("source")))

        # stop -> immediately not serving
        st, body = req(f"{base}/admin/services/{kind}/{name}/state", "PUT",
                       json.dumps({"state": "stopped"}))
        check(f"PUT state=stopped 200 ({kind}/{name})", st == 200)
        st, body = req(f"{base}/admin/services")
        row = next((s for s in json.loads(body).get("services", []) if s["name"] == name), {})
        check("state=stopped + serving=false", row.get("state") == "stopped" and row.get("serving") is False,
              json.dumps(row, ensure_ascii=False))
        st, body = req(f"{base}/tiles/{name}/0/0/0.png")
        check("stopped layer tile not served", st in (404, 400), f"status={st}")
        if not args.dsn:
            check("sidecar written", os.path.exists(SIDECAR))

        # restore
        st, body = req(f"{base}/admin/services/{kind}/{name}/state", "PUT",
                       json.dumps({"state": "active"}))
        check("PUT state=active 200", st == 200)
        st, body = req(f"{base}/admin/services")
        row = next((s for s in json.loads(body).get("services", []) if s["name"] == name), {})
        check("restored: serving=true", row.get("serving") is True, json.dumps(row, ensure_ascii=False))
        if not args.dsn:
            check("sidecar cleaned on active", not os.path.exists(SIDECAR))

        # reload
        st, body = req(f"{base}/admin/reload", "POST")
        doc = json.loads(body) if st == 200 else {}
        check("POST /admin/reload 200", st == 200)
        check("reload keeps layer served", name in doc.get("served", []),
              f"served={len(doc.get('served', []))} tilesets={len(doc.get('tilesets', []))}")

        # bogus state rejected
        st, _ = req(f"{base}/admin/services/{kind}/{name}/state", "PUT",
                    json.dumps({"state": "bogus"}))
        check("bogus state -> 400", st == 400)
    finally:
        if os.name == "nt":
            proc.send_signal(signal.CTRL_BREAK_EVENT)
        else:
            proc.terminate()
        try:
            proc.wait(timeout=10)
        except Exception:
            proc.kill()
        if os.path.exists(SIDECAR):
            os.remove(SIDECAR)
        if tmp_cfg and os.path.exists(tmp_cfg):
            os.remove(tmp_cfg)
    print(f"\n{PASS} passed, {FAIL} failed")
    sys.exit(1 if FAIL else 0)


if __name__ == "__main__":
    main()
