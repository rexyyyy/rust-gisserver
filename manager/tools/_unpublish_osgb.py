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

code, b = call("/map/catalog/published")
j = json.loads(b)
data = j.get("data")
items = data if isinstance(data, list) else (data or {}).get("list") or []
print("published list code", code)
target = None
for it in items:
    ln = it.get("layerName") or it.get("layer_name")
    if ln == "osgb4326":
        target = it
        print("FOUND", it.get("id"), ln, "state=", it.get("state"))

if not target:
    print("osgb4326 not in published list (maybe id=82); try id 82 directly")
    target = {"id": 82}

pid = target.get("id") or 82
code, b = call("/map/catalog/published/%s/state" % pid, "PUT", {"state": "deleted"})
print("set deleted code", code, b[:300].decode("utf-8", "replace"))
