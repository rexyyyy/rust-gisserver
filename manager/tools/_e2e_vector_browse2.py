"""矢量补强探测：4326 下 WMS GetMap + 低层级 MVT 瓦片(应含要素)。"""
import math

import _e2e_common as C

LAYER = "e2e-fp"
BBOX = [34580564.3799, 2752354.5479, 34583351.4979, 2754298.3043]


def to4326(x, y):
    _, tj = C.jget(C.SRV + "/api/v1/transform/points", "POST",
                   {"src_srs": "EPSG:4522", "dst_srs": "EPSG:4326", "points": [[x, y]]})
    p = (tj.get("points") or [[None, None]])[0]
    return float(p[0]), float(p[1])


lon0, lat0 = to4326(BBOX[0], BBOX[1])
lon1, lat1 = to4326(BBOX[2], BBOX[3])
print("bbox4326 =", round(lon0, 5), round(lat0, 5), round(lon1, 5), round(lat1, 5))

# WMS GetMap in 4326 (v1.1.1: lon,lat 顺序)
gmap = (C.MAP + "/wms?service=WMS&version=1.1.1&request=GetMap&layers=%s"
        "&srs=EPSG:4326&bbox=%.6f,%.6f,%.6f,%.6f&width=512&height=512&format=image/png"
        % (LAYER, lon0, lat0, lon1, lat1))
code, b = C.http(gmap)
print("[WMS GetMap 4326] HTTP %s bytes=%d magic=%r" % (code, len(b), b[:8]))
if code != 200:
    print("   err:", b[:200].decode("utf-8", "replace"))

# MVT 多层级中心瓦，找含要素的一级
cx = (lon0 + lon1) / 2
cy = (lat0 + lat1) / 2
for z in (11, 12, 13, 14):
    n = 2 ** z
    x = int((cx + 180) / 360 * n)
    y = int((1 - math.log(math.tan(math.radians(cy)) + 1 / math.cos(math.tan(math.radians(cy)))) / math.pi) / 2 * n)
    code, b = C.http("%s/mvt/%s/%d/%d/%d.pbf" % (C.MAP, LAYER, z, x, y))
    print("[MVT z=%d %d/%d] HTTP %s bytes=%d head=%r" % (z, x, y, code, len(b), b[:3]))
