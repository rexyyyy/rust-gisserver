"""E2E 公共库：gis-srv 作业(提交+轮询) + r-manager catalog 发布链 + r-mapserver 浏览探测。
供 _e2e_*.py 各格式脚本复用。纯标准库。"""
import json
import time
import urllib.error
import urllib.parse
import urllib.request

SRV = "http://127.0.0.1:7070"
MGR = "http://127.0.0.1:6180/manager-api"
MAP = "http://127.0.0.1:8080"
_TOK = {}


def http(url, method="GET", body=None, headers=None, timeout=120):
    h = dict(headers or {})
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        h["Content-Type"] = "application/json"
    if _TOK.get("t"):
        h["Authorization"] = "Bearer " + _TOK["t"]
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def jget(url, method="GET", body=None, timeout=120):
    code, b = http(url, method, body, timeout=timeout)
    try:
        return code, json.loads(b)
    except Exception:
        return code, {"_raw": b[:400].decode("utf-8", "replace")}


# ---------- gis-srv 作业 ----------
def job_run(tool_id, params, tag="", poll=2400):
    code, j = jget(SRV + "/api/v1/jobs", "POST", {"tool_id": tool_id, "params": params})
    jid = j.get("job_id")
    print("[%s] job %s submitted code=%s" % (tag, jid, code))
    if not jid:
        print("   submit resp:", json.dumps(j, ensure_ascii=False)[:400])
        return None
    for i in range(poll):
        time.sleep(2)
        _, st = jget(SRV + "/api/v1/jobs/" + jid)
        s = st.get("status")
        if i % 15 == 0 or s != "running":
            print("   poll %d: %s %s%% phase=%s %s/%s" % (
                i, s, round((st.get("progress") or 0) * 100), st.get("phase"),
                st.get("current"), st.get("total")))
        if s in ("done", "failed", "cancelled", "interrupted"):
            print("[%s] TERMINAL=%s error=%s" % (tag, s, st.get("error")))
            return st
    print("[%s] TIMEOUT" % tag)
    return None


# ---------- r-manager catalog 发布链 ----------
def login():
    code, j = jget(MGR + "/auth/login", "POST", {"username": "admin", "password": "admin123"})
    _TOK["t"] = (j.get("data") or {}).get("access_token")
    print("login code=%s token=%s" % (code, "OK" if _TOK["t"] else j))
    return _TOK["t"]


def catalog_publish(ws_name, store_name, store_type, base_path, resource_name,
                    layer_name, service_types, src_crs, title, bbox=None):
    # workspace
    code, j = jget(MGR + "/map/catalog/workspaces", "POST", {"name": ws_name, "title": title, "remark": "e2e"})
    ws_id = (j.get("data") or {}).get("id")
    if ws_id is None:
        _, wl = jget(MGR + "/map/catalog/workspaces")
        for w in (wl.get("data") or {}).get("workspaces", []):
            if w.get("name") == ws_name:
                ws_id = w.get("id")
    print("  workspace id=%s" % ws_id)
    # store
    code, j = jget(MGR + "/map/catalog/stores", "POST", {
        "workspaceId": ws_id, "name": store_name, "storeType": store_type,
        "basePath": base_path, "remark": "e2e"})
    store_id = (j.get("data") or {}).get("id")
    if store_id is None:
        _, sl = jget(MGR + "/map/catalog/stores?workspaceId=%s" % ws_id)
        for s in (sl.get("data") or {}).get("stores", []) or []:
            if s.get("name") == store_name:
                store_id = s.get("id")
    print("  store id=%s (%s)" % (store_id, store_type))
    # discover
    code, j = jget(MGR + "/map/catalog/stores/%s/discover" % store_id, "POST", {"depth": 1})
    data = j.get("data") or {}
    res = None
    for r in data.get("resources", []):
        if r.get("name") == resource_name:
            res = r
    print("  discover: found=%s resources=%d" % (bool(res), len(data.get("resources", []))))
    if not res:
        for r in data.get("resources", [])[:8]:
            print("     -", r.get("name"), "publishable=", r.get("publishable"))
        return None
    # publish
    body = {"resourceId": res["id"], "layerName": layer_name, "title": title,
            "srcCrs": src_crs, "serviceTypes": service_types, "remark": "e2e"}
    if bbox:
        body["bbox"] = bbox
    code, j = jget(MGR + "/map/catalog/publish", "POST", body)
    print("  publish code=%s resp=%s" % (code, json.dumps(j, ensure_ascii=False)[:300]))
    pub = ((j.get("data") or {}).get("published") or {})
    return pub.get("id")


def endpoints(pub_id):
    code, j = jget(MGR + "/map/catalog/published/%s/endpoints" % pub_id)
    print("  endpoints code=%s" % code)
    print("   ", json.dumps(j.get("data"), ensure_ascii=False)[:600])
    return j.get("data")
