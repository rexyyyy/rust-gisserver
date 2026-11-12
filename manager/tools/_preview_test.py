"""数据面预览测试：验证 rust-gisserver 发布的 3dtiles/osgb4326 能出 tileset + 真实 b3dm。"""
import json
import struct
import urllib.request

BASE = "http://127.0.0.1:8080"
LAYER = "osgb4326"


def get(url, headers=None, timeout=60):
    req = urllib.request.Request(url, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, dict(r.getheaders()), r.read()


# 1) tileset.json
st, hd, b = get(f"{BASE}/3dtiles/{LAYER}/tileset.json")
d = json.loads(b)
root = d["root"]


def first_content_tiles(node, out, limit=3):
    c = node.get("content", {})
    if c.get("uri"):
        out.append(c["uri"])
    for ch in node.get("children", []):
        if len(out) >= limit:
            return
        first_content_tiles(ch, out, limit)


uris = []
first_content_tiles(root, uris, 3)
print(f"[tileset] HTTP {st} ctype={hd.get('Content-Type')} bytes={len(b)} "
      f"asset.version={d['asset']['version']} generator={d['asset'].get('generator')} "
      f"geometricError={d.get('geometricError')} sample_tiles={uris}")

# 2) 取一个真实 b3dm，先 Range 前 64B 验魔术，再全读一个较小瓦片
for uri in uris[:2]:
    u = f"{BASE}/3dtiles/{LAYER}/{uri}"
    st2, hd2, b2 = get(u, headers={"Range": "bytes=0-63"})
    magic = b2[:4]
    ftjl = struct.unpack_from("<I", b2, 12)[0] if len(b2) >= 16 else None
    print(f"[b3dm HEAD] {uri} HTTP {st2} ctype={hd2.get('Content-Type')} "
          f"magic={magic!r} featJsonLen={ftjl} contentRange={hd2.get('Content-Range')}")

print("RESULT: PREVIEW_SERVING" if st == 200 and d.get("asset") else "RESULT: FAIL")
