"""矢量服务浏览探测：WFS GetFeature / WMS GetMap / MVT 瓦片。"""
import json
import math

import _e2e_common as C

LAYER = "e2e-fp"
BBOX = [34580564.3799, 2752354.5479, 34583351.4979, 2754298.3043]
cx = (BBOX[0] + BBOX[2]) / 2
cy = (BBOX[1] + BBOX[3]) / 2

# 1) 完整端点
_, ep = C.jget(C.MGR + "/map/catalog/published/86/endpoints")
eps = (ep or {}).get("data", ep) or {}
for e in (eps.get("endpoints") or []):
    print("  ENDPOINT", e.get("name"), "=>", e.get("url"))

# 2) WFS GetFeature
wfs = (C.MAP + "/wfs?service=WFS&version=2.0.0&request=GetFeature&typeNames=%s"
       "&count=3&srsName=EPSG:4326&outputFormat=application/json" % LAYER)
code, b = C.http(wfs)
print("\n[WFS GetFeature] HTTP %s bytes=%d" % (code, len(b)))
try:
    j = json.loads(b)
    feats = j.get("features", [])
    print("   numberMatched=%s 返回feature=%d 首几何类型=%s" % (
        j.get("numberMatched", j.get("total", "?")), len(feats),
        (feats[0].get("geometry", {}) or {}).get("type") if feats else "-"))
except Exception as ex:
    print("   parse:", b[:200])

# 3) WMS GetMap (4522)
gmap = (C.MAP + "/wms?service=WMS&version=1.3.0&request=GetMap&layers=%s"
        "&bbox=%g,%g,%g,%g&width=256&height=256&crs=EPSG:4522&format=image/png"
        % (LAYER, BBOX[0], BBOX[1], BBOX[2], BBOX[3]))
code, b = C.http(gmap)
print("\n[WMS GetMap 4522] HTTP %s bytes=%d" % (code, len(b)))
if code != 200:
    print("   err:", b[:300].decode("utf-8", "replace"))

# 4) MVT 瓦片：中心 4522 -> 4326 求 lon/lat，再算 z/x/y
_, tj = C.jget(C.SRV + "/api/v1/transform/points", "POST",
               {"src_srs": "EPSG:4522", "dst_srs": "EPSG:4326", "points": [[cx, cy]]})
ll = None
try:
    out = tj.get("points") or tj.get("result") or tj.get("coordinates") or tj.get("data")
    ll = out[0] if out else None
except Exception:
    ll = None
print("\n[transform 4522->4326] resp=%s -> lonlat=%s" % (json.dumps(tj)[:200], ll))
if ll:
    lon, lat = float(ll[0]), float(ll[1])
    z = 14
    n = 2 ** z
    x = int((lon + 180) / 360 * n)
    y = int((1 - math.log(math.tan(math.radians(lat)) + 1 / math.cos(math.tan(math.radians(lat)))) / math.pi) / 2 * n)
    for cand in [
        "%s/mvt/%s/%d/%d/%d.pbf" % (C.MAP, LAYER, z, x, y),
        "%s/mvt/%s/%d/%d/%d" % (C.MAP, LAYER, z, x, y),
        "%s/tiles/%s/%d/%d/%d.pbf" % (C.MAP, LAYER, z, x, y),
    ]:
        code, b = C.http(cand)
        head = b[:4]
        print("  [MVT] %s -> HTTP %s bytes=%d head=%r" % (
            cand.replace(C.MAP, ""), code, len(b), head))
        if code == 200 and len(b) > 0:
            print("     protobuf?(0x1A/0x08/0x28..) gzip?(0x1f8b):", head[:2])
            break
