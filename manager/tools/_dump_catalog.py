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

print("=== workspaces (includeDeleted=1) ===")
code, b = call("/map/catalog/workspaces?includeDeleted=1")
j = json.loads(b)
data = j.get("data")
items = data if isinstance(data, list) else (data or {}).get("list") or []
for w in items:
    print(" ws id=%s name=%s publishedCount=%s" % (w.get("id"), w.get("name"), w.get("publishedCount")))

print("=== published (includeDeleted=1) ===")
code, b = call("/map/catalog/published?includeDeleted=1")
j = json.loads(b)
data = j.get("data")
items = data if isinstance(data, list) else (data or {}).get("list") or []
for p in items:
    print(" pub id=%s layer=%s wsId=%s kind=%s state=%s" % (
        p.get("id"), p.get("layerName") or p.get("layer_name"),
        p.get("workspaceId") or p.get("workspace_id"),
        p.get("kind"), p.get("state")))
