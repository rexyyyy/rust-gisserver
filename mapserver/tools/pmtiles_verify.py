#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""PMTiles 正确性核实（离线 + 可选 HTTP）。

用法：
    python pmtiles_verify.py <archive.pmtiles>            # 仅离线
    python pmtiles_verify.py <archive.pmtiles> --http 18190 wuhua_pm wuhua_buildings

核实点：
  1) v3 头（127B）magic/version/段偏移/压缩/类型/边界
  2) 目录页（root + leaf）ULEB128 列序、delta、offset sentinel、run_length
  3) tile_id(Hilbert) <-> z/x/y 双向一致（含参考向量）
  4) 条目 offset/length 落在 tile_data 段内、clustered 单调
  5) 取出瓦片 -> gunzip -> MVT 解码；要素经纬度必须落在该 z/x/y 的 bbox 内
  6) HTTP：Range 精确窗口字节、/mvt/{layer}/{z}/{x}/{y}.pbf 与本地点位字节一致；
     交换 x/y 后内容必须不同（证明不是行列对称的假通过）
"""
import hashlib
import gzip
import json
import math
import struct
import sys
import urllib.request

FAIL = []
OK = []


def check(name, cond, detail=""):
    (OK if cond else FAIL).append(name)
    print(("PASS " if cond else "FAIL ") + name + ((" | " + detail) if detail else ""))


# ---------------- varint / header / directory ----------------
def uvarint(b, pos):
    r = 0
    shift = 0
    while True:
        x = b[pos]
        pos += 1
        r |= (x & 0x7F) << shift
        if not (x & 0x80):
            return r, pos
        shift += 7
        if shift >= 64:
            raise ValueError("varint overflow")


def read_header(b):
    if b[:7] != b"PMTiles":
        raise ValueError("bad magic")
    u = lambda off: struct.unpack_from("<Q", b, off)[0]
    i = lambda off: struct.unpack_from("<i", b, off)[0]
    return {
        "version": b[7],
        "root_dir_offset": u(8), "root_dir_length": u(16),
        "metadata_offset": u(24), "metadata_length": u(32),
        "leaf_dirs_offset": u(40), "leaf_dirs_length": u(48),
        "tile_data_offset": u(56), "tile_data_length": u(64),
        "num_addressed_tiles": u(72), "num_tile_entries": u(80),
        "num_tile_contents": u(88),
        "clustered": b[96], "internal_compression": b[97],
        "tile_compression": b[98], "tile_type": b[99],
        "min_zoom": b[100], "max_zoom": b[101],
        "min_lon_e7": i(102), "min_lat_e7": i(106),
        "max_lon_e7": i(110), "max_lat_e7": i(114),
        "center_zoom": b[118], "center_lon_e7": i(119), "center_lat_e7": i(123),
    }


def parse_dir(raw):
    b = gzip.decompress(raw)
    pos = 0
    n, pos = uvarint(b, pos)
    es = [{"tile_id": 0, "offset": 0, "length": 0, "run_length": 0} for _ in range(n)]
    last = 0
    for e in es:
        d, pos = uvarint(b, pos)
        last += d
        e["tile_id"] = last
    for e in es:
        e["run_length"], pos = uvarint(b, pos)
    for e in es:
        e["length"], pos = uvarint(b, pos)
    for k in range(n):
        v, pos = uvarint(b, pos)
        es[k]["offset"] = (es[k - 1]["offset"] + es[k - 1]["length"]) if v == 0 else v - 1
    return es


# ---------------- Hilbert tile id ----------------
def zxy_to_tileid(z, x, y):
    n = 1 << z
    d = 0
    s = n >> 1
    while s > 0:
        rx = 1 if (x & s) else 0
        ry = 1 if (y & s) else 0
        d += s * s * ((3 * rx) ^ ry)
        if ry == 0:
            if rx == 1:
                x = n - 1 - x
                y = n - 1 - y
            x, y = y, x
        s >>= 1
    return (4 ** z - 1) // 3 + d


def tileid_to_zxy(tid):
    acc = 0
    z = 0
    while acc + (1 << (2 * z)) <= tid:
        acc += 1 << (2 * z)
        z += 1
    t = tid - acc
    x = y = 0
    s = 1
    while s < (1 << z):
        rx = 1 & (t // 2)
        ry = 1 & (t ^ rx)
        if ry == 0:
            if rx == 1:
                x = s - 1 - x
                y = s - 1 - y
            x, y = y, x
        x += s * rx
        y += s * ry
        t //= 4
        s <<= 1
    return z, x, y


# ---------------- tile bbox (WebMercatorQuad) ----------------
def tile_bbox(z, x, y):
    n = 1 << z

    def lon(px):
        return px / n * 360.0 - 180.0

    def lat(py):
        return math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * py / n))))

    return lon(x), lat(y + 1), lon(x + 1), lat(y)


# ---------------- minimal MVT decoder ----------------
def _fields(buf):
    pos = 0
    out = []
    while pos < len(buf):
        key, pos = uvarint(buf, pos)
        f, wt = key >> 3, key & 7
        if wt == 0:
            v, pos = uvarint(buf, pos)
        elif wt == 2:
            ln, pos = uvarint(buf, pos)
            v = buf[pos:pos + ln]
            pos += ln
        elif wt == 5:
            v = struct.unpack_from("<f", buf, pos)[0]
            pos += 4
        elif wt == 1:
            v = struct.unpack_from("<d", buf, pos)[0]
            pos += 8
        else:
            raise ValueError(f"wire type {wt}")
        out.append((f, v))
    return out


def _packed(buf):
    pos = 0
    out = []
    while pos < len(buf):
        v, pos = uvarint(buf, pos)
        out.append(v)
    return out


def decode_mvt(buf):
    """返回 [(layer, extent, [(geom_type, [ring...])...])]，坐标为 tile-local。"""
    feats = []
    for f, layer in _fields(buf):
        if f != 3:
            continue
        name, extent, keys, vals, raw_feats = "", 4096, [], [], []
        for lf, lv in _fields(layer):
            if lf == 1:
                name = lv.decode()
            elif lf == 2:
                raw_feats.append(lv)
            elif lf == 3:
                keys.append(lv.decode())
            elif lf == 4:
                vals.append(lv)
            elif lf == 5:
                extent = lv
        for rf in raw_feats:
            gtype, geom = 0, []
            for ff, fv in _fields(rf):
                if ff == 3:
                    gtype = fv
                elif ff == 4:
                    geom = _packed(fv)
            rings, cur = [], []
            i = 0
            x = y = 0
            while i < len(geom):
                cmd, cnt = geom[i] & 7, geom[i] >> 3
                i += 1
                if cmd == 1:  # MoveTo
                    for _ in range(cnt):
                        x += _zig(geom[i]); y += _zig(geom[i + 1]); i += 2
                        if cur:
                            rings.append(cur)
                        cur = [(x, y)]
                elif cmd == 2:  # LineTo
                    for _ in range(cnt):
                        x += _zig(geom[i]); y += _zig(geom[i + 1]); i += 2
                        cur.append((x, y))
                elif cmd == 7:  # ClosePath
                    if cur:
                        rings.append(cur)
                        cur = []
            if cur:
                rings.append(cur)
            feats.append((name, extent, gtype, rings))
    return feats


def _zig(v):
    return (v >> 1) ^ (-(v & 1))


def local_to_lonlat(z, tx, ty, px, py, extent):
    n = 1 << z
    wx = (tx + px / extent) / n
    wy = (ty + py / extent) / n
    lon = wx * 360.0 - 180.0
    lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * wy))))
    return lon, lat


# ---------------- main ----------------
def main():
    path = sys.argv[1]
    http = None
    if "--http" in sys.argv:
        k = sys.argv.index("--http")
        http = (int(sys.argv[k + 1]), sys.argv[k + 2], sys.argv[k + 3])

    data = open(path, "rb").read()
    h = read_header(data)

    print("== header ==")
    print(json.dumps({k: v for k, v in h.items()}, indent=None))
    check("magic/version=3", data[:7] == b"PMTiles" and h["version"] == 3)
    check("header 段偏移有序（root<=meta<=leaf<=tile_data）",
          h["root_dir_offset"] <= h["metadata_offset"] <= h["leaf_dirs_offset"] <= h["tile_data_offset"]
          and h["metadata_offset"] + h["metadata_length"] == h["leaf_dirs_offset"]
          and h["leaf_dirs_offset"] + h["leaf_dirs_length"] == h["tile_data_offset"],
          f"root={h['root_dir_offset']} meta={h['metadata_offset']}+{h['metadata_length']} "
          f"leaf={h['leaf_dirs_offset']}+{h['leaf_dirs_length']} data={h['tile_data_offset']}")
    check("tile_data 段不越界",
          h["tile_data_offset"] + h["tile_data_length"] <= len(data),
          f"{h['tile_data_offset']}+{h['tile_data_length']} <= {len(data)}")
    check("internal_compression=GZIP", h["internal_compression"] == 2)
    check("tile_type=MVT", h["tile_type"] == 1)
    check("zoom 范围合理", 0 <= h["min_zoom"] <= h["max_zoom"] <= 26,
          f"{h['min_zoom']}..{h['max_zoom']}")
    bbox_hdr = (h["min_lon_e7"] / 1e7, h["min_lat_e7"] / 1e7,
                h["max_lon_e7"] / 1e7, h["max_lat_e7"] / 1e7)
    print("header bounds lon/lat:", bbox_hdr)
    check("header 边界在中国范围（经度 70..140，纬度 -10..60）",
          70 < bbox_hdr[0] < bbox_hdr[2] < 140 and -10 < bbox_hdr[1] < bbox_hdr[3] < 60,
          str(bbox_hdr))

    print("== directory ==")
    root = parse_dir(data[h["root_dir_offset"]:h["root_dir_offset"] + h["root_dir_length"]])
    leaves = []
    for e in root:
        if e["run_length"] == 0:
            off = h["leaf_dirs_offset"] + e["offset"]
            leaves.append(parse_dir(data[off:off + e["length"]]))
    all_leaves = [x for ls in leaves for x in ls]
    print(f"root entries={len(root)} leaf_dirs={len(leaves)} leaf entries={len(all_leaves)} "
          f"num_tile_entries={h['num_tile_entries']} num_tile_contents={h['num_tile_contents']}")
    check("root 目录按 tile_id 严格递增",
          all(root[i]["tile_id"] < root[i + 1]["tile_id"] for i in range(len(root) - 1)))
    check("leaf 目录按 tile_id 严格递增",
          all(all_leaves[i]["tile_id"] < all_leaves[i + 1]["tile_id"] for i in range(len(all_leaves) - 1)))
    check("tile_id 解码后的 zoom 全部在 [min,max]",
          all(h["min_zoom"] <= tileid_to_zxy(e["tile_id"])[0] <= h["max_zoom"]
              for e in (all_leaves or root)))
    check("条目 offset+length 落在 tile_data 段内",
          all(e["offset"] + e["length"] <= h["tile_data_length"] for e in (all_leaves or root)))
    check("clustered=1 时 offset 单调不减",
          h["clustered"] != 1 or all(
              (all_leaves or root)[i]["offset"] <= (all_leaves or root)[i + 1]["offset"]
              for i in range(len(all_leaves or root) - 1)))

    print("== hilbert vectors ==")
    vec = [((1, 0, 0), 1), ((1, 0, 1), 2), ((1, 1, 1), 3), ((1, 1, 0), 4),
           ((0, 0, 0), 0), ((8, 40, 87), 36052), ((12, 3423, 1763), 19078479)]
    check("参考向量", all(zxy_to_tileid(*zxy) == tid for zxy, tid in vec))
    bad = []
    for z in range(0, 13):
        n = 1 << z
        for x in range(0, n, max(1, n // 8)):
            for y in range(0, n, max(1, n // 8)):
                if tileid_to_zxy(zxy_to_tileid(z, x, y)) != (z, x, y):
                    bad.append((z, x, y))
    check("z=0..12 抽样往返一致", not bad, str(bad[:3]))

    # 取样：逐 zoom 取第一个真实瓦片（run_length>0），再取中间若干
    entries = all_leaves or [e for e in root if e["run_length"] > 0]
    samples = []
    seen_z = set()
    for e in entries:
        if e["run_length"] == 0:
            continue
        z, x, y = tileid_to_zxy(e["tile_id"])
        if z not in seen_z:
            seen_z.add(z)
            samples.append((e, z, x, y))
    if len(samples) > 6:
        step = max(1, len(samples) // 6)
        samples = samples[::step][:6]

    print(f"== tile samples ({len(samples)}) ==")
    for e, z, x, y in samples:
        blob = data[h["tile_data_offset"] + e["offset"]: h["tile_data_offset"] + e["offset"] + e["length"]]
        raww = gzip.decompress(blob) if (len(blob) > 2 and blob[:2] == b"\x1f\x8b") else blob
        feats = decode_mvt(raww)
        pts = []
        exts = set()
        lmin = [1e9, 1e9]
        lmax = [-1e9, -1e9]
        for _, extent, _, rings in feats:
            exts.add(extent)
            for ring in rings:
                for px, py in ring:
                    lmin[0] = min(lmin[0], px); lmax[0] = max(lmax[0], px)
                    lmin[1] = min(lmin[1], py); lmax[1] = max(lmax[1], py)
                    pts.append(local_to_lonlat(z, x, y, px, py, extent))
        ex = max(exts) if exts else 4096
        # MVT 规范允许 tile 边缘 buffer：实测约 80/4096，这里放宽到 256 单位
        buf = 256
        loc_ok = bool(pts) and lmin[0] >= -buf and lmin[1] >= -buf \
            and lmax[0] <= ex + buf and lmax[1] <= ex + buf
        tb = tile_bbox(z, x, y)
        dx = (tb[2] - tb[0]) * buf / ex
        dy = (tb[3] - tb[1]) * buf / ex
        inside = loc_ok and all(tb[0] - dx <= lo <= tb[2] + dx and
                                tb[1] - dy <= la <= tb[3] + dy for lo, la in pts)
        if pts:
            lo0 = min(p[0] for p in pts); lo1 = max(p[0] for p in pts)
            la0 = min(p[1] for p in pts); la1 = max(p[1] for p in pts)
            fbox = (lo0, la0, lo1, la1)
        else:
            fbox = None
        print(f"  z{z} ({x},{y}) entry offset={e['offset']} len={e['length']} "
              f"features={len(feats)} extent={ex} local=[{lmin[0]},{lmin[1]}]..[{lmax[0]},{lmax[1]}] "
              f"bbox={fbox}")
        check(f"z{z}({x},{y}) 可解出 MVT 且要素落在 tile bbox 内（含 buffer）", bool(feats) and inside,
              f"tile_bbox={tuple(round(v, 6) for v in tb)} "
              f"overshoot_units=({max(0.0, -lmin[0]):.0f},{max(0.0, -lmin[1]):.0f},"
              f"{max(0.0, lmax[0] - ex):.0f},{max(0.0, lmax[1] - ex):.0f})")
        if fbox:
            check(f"z{z}({x},{y}) 要素 bbox 与 header 边界相容",
                  bbox_hdr[0] - 0.05 <= fbox[0] and fbox[2] <= bbox_hdr[2] + 0.05 and
                  bbox_hdr[1] - 0.05 <= fbox[1] and fbox[3] <= bbox_hdr[3] + 0.05)

    # 跨层级一致性：z 与其子瓦片的要素应落在各自 bbox 内（同一 tile_id 算法在两级都成立）
    print("== cross-zoom consistency ==")
    by_z = {}
    for e in entries:
        if e["run_length"] == 0:
            continue
        z, x, y = tileid_to_zxy(e["tile_id"])
        by_z.setdefault(z, []).append((x, y))
    zs = sorted(by_z)
    if zs:
        z = zs[len(zs) // 2]
        x, y = by_z[z][0]
        kids = [k for k in by_z.get(z + 1, []) if k[0] in (2 * x, 2 * x + 1) and k[1] in (2 * y, 2 * y + 1)]
        check(f"z{z}({x},{y}) 的 4 个子瓦片至少存在 1 个", bool(kids), f"{len(kids)} children")

    # ---------------- HTTP ----------------
    if http:
        port, layer, archive = http
        base = f"http://127.0.0.1:{port}"

        def fetch(p, headers=None):
            req = urllib.request.Request(base + p, headers=headers or {})
            try:
                r = urllib.request.urlopen(req, timeout=30)
                return r.status, dict((k.lower(), v) for k, v in r.getheaders()), r.read()
            except urllib.error.HTTPError as ex:
                return ex.code, {}, ex.read()

        print(f"== HTTP :{port} ==")
        s, hd, body = fetch(f"/pmtiles/{archive}")
        check("整档字节与本地一致", s == 200 and hashlib.sha256(body).digest() == hashlib.sha256(data).digest(),
              f"{s} {len(body)}B")

        for e, z, x, y in samples[:3]:
            start = h["tile_data_offset"] + e["offset"]
            end = start + e["length"] - 1
            s, hd, body = fetch(f"/pmtiles/{archive}", {"Range": f"bytes={start}-{end}"})
            local = data[start:end + 1]
            check(f"Range {start}-{end} 与目录条目字节一致",
                  s == 206 and hashlib.sha256(body).digest() == hashlib.sha256(local).digest(),
                  f"{s} {len(body)}B cr={hd.get('content-range')}")

        for e, z, x, y in samples[:4]:
            local = data[h["tile_data_offset"] + e["offset"]:
                         h["tile_data_offset"] + e["offset"] + e["length"]]
            s, hd, body = fetch(f"/mvt/{layer}/{z}/{x}/{y}.pbf")
            check(f"/mvt/{layer}/{z}/{x}/{y} 与目录条目字节一致",
                  s == 200 and hashlib.sha256(body).digest() == hashlib.sha256(local).digest(),
                  f"{s} {len(body)}B enc={hd.get('content-encoding')}")
            if x != y:
                s2, _, body2 = fetch(f"/mvt/{layer}/{z}/{y}/{x}.pbf")
                check(f"/mvt 交换 x/y 后字节不同（排除行列对称）",
                      s2 != 200 or hashlib.sha256(body2).digest() != hashlib.sha256(local).digest(),
                      f"swapped={s2}")

        s, _, body = fetch(f"/mvt/{layer}/{h['max_zoom'] + 1}/0/0.pbf")
        check("超出 max_zoom 返回 404", s == 404, str(s))

    print(f"\n== SUMMARY: {len(OK)} passed / {len(FAIL)} failed ==")
    if FAIL:
        print("FAILED:", FAIL)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
