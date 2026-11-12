"""MBTiles / PMTiles 归档全链路：发布 *_mvt -> mvt 瓦片浏览实测。"""
import os

import _e2e_common as C

MBD = r"D:\mycomputor\documents\gis-wt\.testdata\e2e\mbtiles"
PMD = r"D:\mycomputor\documents\gis-wt\.testdata\e2e\pmtiles"
PBF = r"D:\mycomputor\documents\gis-wt\.testdata\e2e\shp_mvt"
BBOX4326 = [102.65625, 24.846565, 103.007812, 25.165173]
C.login()

mb = C.catalog_publish("e2embws", "e2embdir", "mbtiles_dir", MBD, "baizhi",
                       "e2e-baizhi-mbt", ["mvt"], "EPSG:4326", "E2E MBTiles(baizhi)", bbox=BBOX4326)
pm = C.catalog_publish("e2epmws", "e2epmdir", "pmtiles_dir", PMD, "baizhi",
                       "e2e-baizhi-pmt", ["mvt"], "EPSG:4326", "E2E PMTiles(baizhi)", bbox=BBOX4326)
print("PUB_ID mbtiles=%s pmtiles=%s" % (mb, pm))
if mb:
    C.endpoints(mb)

# 找一个真实有要素的瓦 z/x/y（pbf 体积最大的）
best = None
for root, d, fs in os.walk(PBF):
    for f in fs:
        if f.endswith(".pbf"):
            fp = os.path.join(root, f)
            rel = os.path.relpath(fp, PBF).replace("\\", "/").split("/")
            if len(rel) >= 3:
                sz = os.path.getsize(fp)
                if best is None or sz > best[0]:
                    best = (sz, rel[0], rel[1], rel[2].split(".")[0])
print("best tile size=%s z/x/y=%s/%s/%s" % best)
_, z, x, y = best
for layer in ("e2e-baizhi-mbt", "e2e-baizhi-pmt"):
    code, b = C.http("%s/mvt/%s/%s/%s/%s.pbf" % (C.MAP, layer, z, x, y))
    print("[MVT %s] HTTP %s bytes=%d head=%r" % (layer, code, len(b), b[:3]))
