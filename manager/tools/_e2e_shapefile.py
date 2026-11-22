"""Shapefile：shapefile_dir 可发现、不可发布(publishable=false) 演示。"""
import _e2e_common as C

OUTDIR = r"D:\mycomputor\documents\gis-wt\.testdata\e2e\shp"
C.login()
_, j = C.jget(C.MGR + "/map/catalog/workspaces", "POST", {"name": "e2eshpws", "title": "shp", "remark": "e2e"})
ws = (j.get("data") or {}).get("id")
if ws is None:
    _, wl = C.jget(C.MGR + "/map/catalog/workspaces")
    ws = next((w["id"] for w in (wl.get("data") or {}).get("workspaces", []) if w.get("name") == "e2eshpws"), None)
_, j = C.jget(C.MGR + "/map/catalog/stores", "POST", {"workspaceId": ws, "name": "e2eshpdir", "storeType": "shapefile_dir", "basePath": OUTDIR})
sid = (j.get("data") or {}).get("id")
if sid is None:
    _, sl = C.jget(C.MGR + "/map/catalog/stores?workspaceId=%s" % ws)
    sid = next((s["id"] for s in (sl.get("data") or {}).get("stores", []) if s.get("name") == "e2eshpdir"), None)
print("  store id=%s (shapefile_dir)" % sid)
_, j = C.jget(C.MGR + "/map/catalog/stores/%s/discover" % sid, "POST", {"depth": 1})
for r in (j.get("data") or {}).get("resources", []):
    print("  resource name=%s kind=%s publishable=%s srcCrs=%s" % (
        r.get("name"), r.get("kind"), r.get("publishable"), r.get("srcCrs")))
    rid = r.get("id")
_, j = C.jget(C.MGR + "/map/catalog/publish", "POST", {
    "resourceId": rid, "layerName": "e2e-shp", "title": "shp",
    "srcCrs": "EPSG:4522", "serviceTypes": ["mvt"]})
print("  publish(不可发布应被拒) code/msg:", j.get("code"), j.get("msg"))
