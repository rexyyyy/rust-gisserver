"""r-manager catalog 发布 OSGB4326 3D Tiles：登录 -> 工作区 -> Store(3dtiles_dir)
-> discover -> publish -> 取 endpoints。分步打印响应，便于核对链路。
用法: python tools/_publish_osgb.py [mgr_base]
默认 http://127.0.0.1:6180
"""
import json
import sys
import urllib.error
import urllib.request

API = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:6180").rstrip("/") + "/manager-api"
TILESET_PARENT = r"D:\mycomputor\documents\gis-wt\.testdata\3dtiles"
RESOURCE_NAME = "kmczc_reconv_3dtiles"
LAYER_NAME = "kmczc-reconv"
WS_NAME = "kmczcrcws"
STORE_NAME = "kmczcrcdir"

TOK = {}


def http(path, method="GET", body=None):
    h = {}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        h["Content-Type"] = "application/json"
    if TOK.get("t"):
        h["Authorization"] = "Bearer " + TOK["t"]
    req = urllib.request.Request(API + path, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def jget(path, method="GET", body=None):
    code, b = http(path, method, body)
    try:
        j = json.loads(b)
    except Exception:
        j = {"_raw": b[:400].decode("utf-8", "replace")}
    return code, j


print("== login ==")
code, j = jget("/auth/login", "POST", {"username": "admin", "password": "admin123"})
TOK["t"] = (j.get("data") or {}).get("access_token") or (j.get("data") or {}).get("token")
print("code", code, "token", "OK" if TOK["t"] else j)

print("== 1) workspace ==")
code, j = jget("/map/catalog/workspaces", "POST", {"name": WS_NAME, "title": "OSGB4326 三维", "remark": "osgb test"})
print(code, j)
ws_id = (j.get("data") or {}).get("id")

print("== 2) store 3dtiles_dir ==")
code, j = jget("/map/catalog/stores", "POST", {
    "workspaceId": ws_id, "name": STORE_NAME, "storeType": "3dtiles_dir",
    "basePath": TILESET_PARENT, "remark": "osgb4326 tileset parent"})
print(code, j)
store_id = (j.get("data") or {}).get("id")

print("== 3) discover ==")
code, j = jget(f"/map/catalog/stores/{store_id}/discover", "POST", {"depth": 1})
data = j.get("data") or {}
print("code", code, "discovered", data.get("discovered"), "stored", data.get("stored"))
res = None
for r in data.get("resources", []):
    print("   resource:", r.get("id"), r.get("name"), "dataType=", r.get("dataType"),
          "publishable=", r.get("publishable"), "srcCrs=", r.get("srcCrs"), "source=", r.get("source"))
    if r.get("name") == RESOURCE_NAME:
        res = r

if not res:
    print("!! target resource not discovered:", RESOURCE_NAME)
    sys.exit(3)

print("== 4) publish ==")
code, j = jget("/map/catalog/publish", "POST", {
    "resourceId": res["id"], "layerName": LAYER_NAME,
    "title": "kmczc 重转(EPSG:2358 投影/米)", "srcCrs": "EPSG:2358",
    "serviceTypes": ["3dtiles"], "remark": "from E:\\data_km\\3d-data\\3D\\XD7X\\kmczc"})
print(code, json.dumps(j, ensure_ascii=False)[:800])
pub_id = ((j.get("data") or {}).get("published") or {}).get("id")

print("== 5) endpoints ==")
code, j = jget(f"/map/catalog/published/{pub_id}/endpoints")
print(code, json.dumps(j, ensure_ascii=False)[:1200])
print("\nPUB_ID", pub_id, "LAYER", LAYER_NAME)
