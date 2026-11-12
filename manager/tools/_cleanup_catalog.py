import json
import urllib.request
import urllib.error

API = "http://127.0.0.1:6180/manager-api"
TOK = {}


def call(path, method="GET", body=None):
    h = {}
    d = None
    if body is not None:
        d = json.dumps(body).encode()
        h["Content-Type"] = "application/json"
    if TOK.get("t"):
        h["Authorization"] = "Bearer " + TOK["t"]
    req = urllib.request.Request(API + path, data=d, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


_, b = call("/auth/login", "POST", {"username": "admin", "password": "admin123"})
TOK["t"] = json.loads(b)["data"].get("access_token")

# 先删 store，再删 workspace（ws_remove 在仍有 store 时拒绝）
for store_id in (10, 13):
    code, b = call("/map/catalog/stores/%s" % store_id, "DELETE")
    print("DEL store %s -> %s %s" % (store_id, code, b[:200].decode("utf-8", "replace")))
for ws_id in (9, 12):
    code, b = call("/map/catalog/workspaces/%s" % ws_id, "DELETE")
    print("DEL ws %s -> %s %s" % (ws_id, code, b[:200].decode("utf-8", "replace")))

# 复核剩余工作区
code, b = call("/map/catalog/workspaces")
j = json.loads(b)
for w in j.get("data", {}).get("workspaces", []):
    print("  remain ws id=%s name=%s pub=%s store=%s" % (
        w.get("id"), w.get("name"), w.get("publishedCount"), w.get("storeCount")))
