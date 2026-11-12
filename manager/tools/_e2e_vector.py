"""矢量 MVT/WFS/WMS 全链路 E2E：geojson -> 发布 geojson_dir -> 浏览实测。
rust-gisserver 对 geojson 走动态重编码（mvt/wfs/oaf/wms/wmts），无需先切片。"""
import os
import shutil

import _e2e_common as C

SRC = r"D:\mycomputor\documents\gis-wt\.testdata\recon\recon_footprint_4522.geojson"
OUTDIR = r"D:\mycomputor\documents\gis-wt\.testdata\e2e\vector"
os.makedirs(OUTDIR, exist_ok=True)
shutil.copy(SRC, os.path.join(OUTDIR, "e2e_fp.geojson"))
SRC_CRS = "EPSG:4522"
BBOX = [34580564.3799, 2752354.5479, 34583351.4979, 2754298.3043]

print("== 发布 geojson_dir ==")
C.login()
pub_id = C.catalog_publish(
    "e2evecws", "e2evecdir", "geojson_dir", OUTDIR, "e2e_fp",
    "e2e-fp", ["mvt", "wfs", "wms"], SRC_CRS, "E2E 矢量 footprint(4522)", bbox=BBOX)
print("  PUB_ID=", pub_id)
ep = C.endpoints(pub_id) if pub_id else None
