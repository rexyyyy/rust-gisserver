"""COG 全链路 E2E：tif -> cog.convert -> 发布 cog_dir -> WMTS/XYZ/WMS 浏览实测。"""
import json
import os
import urllib.parse
import urllib.request

import _e2e_common as C

SRC = r"E:\data_km\img-1\ans\5301812023JL1-2DOM-1.tif"
OUTDIR = r"D:\mycomputor\documents\gis-wt\.testdata\e2e\cog"
DST = os.path.join(OUTDIR, "e2e_dom.tif")
os.makedirs(OUTDIR, exist_ok=True)
SRC_CRS = "EPSG:4522"
BBOX = [34560028.5, 2749042.5, 34562191.5, 2751557.5]

print("== 1) cog.convert 作业 ==")
st = C.job_run("cog.convert", {"input": SRC, "output": DST, "compress": "DEFLATE"}, tag="cog")
if not st or st.get("status") != "done":
    print("convert failed"); raise SystemExit(2)
res = st.get("result") or {}
print("  result.output=%s layout=%s compress=%s overviews=%s" % (
    res.get("output"), res.get("layout"), res.get("compression"), res.get("overviews")))
print("  COG ok=%s exists=%s" % (res.get("layout") == "COG", os.path.exists(DST)))

print("== 2) rust-gisserver 发布 cog_dir ==")
C.login()
pub_id = C.catalog_publish(
    "e2ecogws", "e2ecogdir", "cog_dir", OUTDIR, "e2e_dom",
    "e2e-cog", ["wms", "wmts", "xyz"], SRC_CRS, "E2E COG 影像(4522)", bbox=BBOX)
print("  PUB_ID=", pub_id)
if pub_id:
    C.endpoints(pub_id)

print("== 3) rust-gisserver 浏览探测 ==")
# WMTS GetCapabilities
for label, url in [
    ("WMTS Cap", C.MAP + "/wmts?service=WMTS&version=1.0.0&request=GetCapabilities"),
    ("WMS Cap", C.MAP + "/wms?service=WMS&version=1.3.0&request=GetCapabilities"),
]:
    code, b = C.http(url)
    txt = b.decode("utf-8", "replace")
    has = ("e2e-cog" in txt) or ("E2E COG" in txt)
    print("  [%s] HTTP %s len=%d 含图层名=%s" % (label, code, len(b), has))
# WMS GetMap (小图)
gmap = (C.MAP + "/wms?service=WMS&version=1.3.0&request=GetMap&layers=e2e-cog"
        "&bbox=34560028.5,2749042.5,34562191.5,2751557.5&width=256&height=256"
        "&crs=EPSG:4522&format=image/png")
code, b = C.http(gmap)
print("  [WMS GetMap] HTTP %s ctype=%s bytes=%d magic=%r" % (
    code, (b[:4] if code == 200 else "-"), len(b), b[:8]))
