"""r-manager + r-mapserver 联合功能测试（针对已运行的 live 实例）。

与 r-mapserver/tools/capability_probe.py（自建最小 catalog、验路由语义）不同，
本脚本直接打 r-mapserver 仓库 config/ 下的真实图层目录（数据源 E:\\data\\1 +
PostGIS :5434 的 gis 库），并按 r-manager 的实际管理链路做接口级走查：

  map-api :8080   数据面：WMS / WMTS / XYZ / OGC API Features / OGC API Tiles /
                  MVT（vector + PostGIS 双坐标接口）/ PMTiles / MBTiles /
                  COG Range / 3D Tiles / terrain / admin + 错误分流语义
  manager :6180   管理面：登录 → JWT → 代理 /map/services（跨引擎聚合）→
                  引擎列表与健康探测 → getRouters → 服务配置读写回环（PUT 回写
                  原文，产生 .bak 但不改变内容）→ SPA 静态首页与前端路由回退

瓦片坐标说明：测试点取昆明五华区（约 102.66E, 25.08N）。WebMercatorQuad 下
z8/200/109、z10/803/438、z12/3213/1753；WorldCRS84Quad 下 z6/51/28。用法：

  python tools/func_test.py [mapapi_base] [manager_base]
  默认 http://127.0.0.1:8080 与 http://127.0.0.1:6180

退出码 = 失败用例数（0 表示全部通过）。
"""
import json
import sys
import urllib.error
import urllib.request

MAPAPI = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8080").rstrip("/")
MGR = (sys.argv[2] if len(sys.argv) > 2 else "http://127.0.0.1:6180").rstrip("/")
API = MGR + "/manager-api"

results = []


def http(url, method="GET", body=None, headers=None, timeout=60):
    h = dict(headers or {})
    data = None
    if body is not None:
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        h.setdefault("Content-Type", "application/json")
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, {k.lower(): v for k, v in r.getheaders()}, r.read()
    except urllib.error.HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read()


def check(name, fn):
    """fn() -> (bool, detail)；异常记为失败但不中断整套测试。"""
    try:
        ok, detail = fn()
    except Exception as e:  # noqa: BLE001
        ok, detail = False, f"exception: {e!r}"
    results.append((name, ok, detail))
    print(("  PASS " if ok else "  FAIL ") + name + (f"  [{detail}]" if detail else ""))


def is_img(b):
    return b[:8] == b"\x89PNG\r\n\x1a\n" or b[:3] == b"\xff\xd8\xff"


def is_pbf(b):
    """MVT 顶层是 varint 字段头；报错响应通常是 JSON/XML（以 { 或 < 开头）。"""
    return len(b) > 0 and b[:1] not in (b"{", b"<")


# ------------------------------------------------------------------ 断言助手
def st(url, want=(200,), img=False, pbf=False, hdr=None, method="GET", body=None):
    code, headers, b = http(url, method=method, body=body, headers=hdr)
    if code not in want:
        return False, f"HTTP {code} {b[:120]!r}"
    if img and not is_img(b):
        return False, f"非图像 {b[:32]!r}"
    if pbf and code == 200 and not is_pbf(b):
        return False, f"非瓦片 {b[:32]!r}"
    return True, f"{len(b)}B ctype={headers.get('content-type', '-')}"


def js(url, pred, hdr=None, method="GET", body=None):
    code, _, b = http(url, method=method, body=body, headers=hdr)
    if code != 200:
        return False, f"HTTP {code} {b[:160]!r}"
    try:
        j = json.loads(b)
    except Exception:
        return False, "响应非 JSON"
    return pred(j)


# ---------------------------------------------------------------- map-api 数据面
def section_mapapi():
    print("\n== r-mapserver map-api 数据面（真实 config/，E:\\data\\1 + PostGIS gis 库）==")

    # 管理面 / 探活
    check("GET /admin/services 图层清单",
          lambda: js(MAPAPI + "/admin/services",
                     lambda j: (j.get("count", 0) > 0, f"{j.get('count')} 个服务 registry_mode={j.get('registry_mode')}")))
    check("GET /admin/components 组件探测", lambda: st(MAPAPI + "/admin/components"))
    check("GET /health 探活", lambda: st(MAPAPI + "/health"))
    check("GET /ready 就绪", lambda: st(MAPAPI + "/ready"))

    # COG 栅格：WMS / WMTS / XYZ
    check("WMS 1.1.1 GetMap world（轻量 COG）出图",
          lambda: st(MAPAPI + "/wms?service=WMS&version=1.1.1&request=GetMap&layers=world"
                            "&bbox=100,36,108,40&srs=EPSG:4326&width=256&height=256&format=image/png", img=True))
    check("WMS 1.3.0 GetMap wh（76MB COG）出图",
          lambda: st(MAPAPI + "/wms?service=WMS&version=1.3.0&request=GetMap&layers=wh"
                            "&bbox=24.9,102.6,25.1,102.9&crs=EPSG:4326&width=512&height=512&format=image/png", img=True))
    check("WMTS KVP GetTile world@WebMercatorQuad",
          lambda: st(MAPAPI + "/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=world"
                            "&STYLE=default&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=2&TILEROW=1&TILECOL=3"
                            "&FORMAT=image/png", img=True))
    check("WMTS KVP GetTile world@WorldCRS84Quad",
          lambda: st(MAPAPI + "/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=world"
                            "&STYLE=default&TILEMATRIXSET=WorldCRS84Quad&TILEMATRIX=2&TILEROW=1&TILECOL=3"
                            "&FORMAT=image/png", img=True))
    check("XYZ /tiles/wh/8/200/109.png 出图",
          lambda: st(MAPAPI + "/tiles/wh/8/200/109.png", img=True))
    check("COG 直读 /cog/world", lambda: st(MAPAPI + "/cog/world"))

    # 矢量：WFS / OGC API Features / OGC API Tiles
    check("WFS GetFeature places 返回 GML",
          lambda: _wfs_has_member("/wfs?service=WFS&version=2.0.0&request=GetFeature&typeNames=places&count=3"))
    check("WFS GetFeature wuhua_building(PostGIS) 返回要素",
          lambda: _wfs_has_member("/wfs?service=WFS&version=2.0.0&request=GetFeature"
                                  "&typeNames=wuhua_building&count=3"))
    check("OGC features collections 清单",
          lambda: js(MAPAPI + "/ogc/features/v1/collections", lambda j: ("collections" in j, "")))
    check("OGC features items places?limit=3",
          lambda: js(MAPAPI + "/ogc/features/v1/collections/places/items?limit=3",
                     lambda j: (len(j.get("features", [])) == 3, f"{len(j.get('features', []))} features")))
    check("OGC features items wuhua_building?limit=2",
          lambda: js(MAPAPI + "/ogc/features/v1/collections/wuhua_building/items?limit=2",
                     lambda j: (len(j.get("features", [])) == 2, f"{len(j.get('features', []))} features")))
    check("OGC tiles collections 清单",
          lambda: js(MAPAPI + "/ogc/tiles/v1/collections", lambda j: ("collections" in j, "")))
    check("OGC tiles places/WebMercatorQuad/6/27/51（矢量层出 pbf）",
          lambda: st(MAPAPI + "/ogc/tiles/v1/collections/places/tiles/WebMercatorQuad/6/27/51", pbf=True))
    check("OGC tiles wh/WebMercatorQuad/6/27/51（栅格层出 png）",
          lambda: st(MAPAPI + "/ogc/tiles/v1/collections/wh/tiles/WebMercatorQuad/6/27/51", img=True))

    # MVT：vector / PostGIS（双坐标接口）
    check("MVT vector recon_fp z6 瓦片",
          lambda: st(MAPAPI + "/mvt/recon_fp/6/51/28.pbf", pbf=True))
    check("MVT PostGIS wuhua_building z8 瓦片",
          lambda: st(MAPAPI + "/mvt/wuhua_building/8/200/109.pbf", pbf=True))
    check("MVT PostGIS wuhua_building z12 瓦片",
          lambda: st(MAPAPI + "/mvt/wuhua_building/12/3216/1753.pbf", pbf=True))
    check("MVT PostGIS 3857 显式接口 z10",
          lambda: st(MAPAPI + "/mvt/wuhua_building/3857/10/803/438.pbf", pbf=True))
    check("MVT PostGIS tilejson.json",
          lambda: js(MAPAPI + "/mvt/wuhua_building/tilejson.json",
                     lambda j: (j.get("vector_layers"), f"bounds={j.get('bounds')}")))
    check("MVT 未知 srid 接口返回 400",
          lambda: _want(MAPAPI + "/mvt/wuhua_building/4543/0/0/0.pbf", (400,)))

    # 瓦片包：pmtiles / mbtiles
    check("PMTiles 归档直读 wuhua_buildings",
          lambda: st(MAPAPI + "/pmtiles/wuhua_buildings"))
    check("MVT pmtiles 图层 wuhua_bld_mgr z10",
          lambda: st(MAPAPI + "/mvt/wuhua_bld_mgr/10/803/438.pbf", pbf=True, want=(200, 204)))
    check("MBTiles 归档信息 /mbtiles/test",
          lambda: js(MAPAPI + "/mbtiles/test", lambda j: (j.get("format") == "png", f"zoom={j.get('zoom_range')}")))
    check("MBTiles 位图瓦片 /mbtiles/test/3/4/2.png",
          lambda: st(MAPAPI + "/mbtiles/test/3/4/2.png", img=True))
    check("MBTiles 矢量瓦片 /mbtiles/test-vector/3/4/2.pbf",
          lambda: st(MAPAPI + "/mbtiles/test-vector/3/4/2.pbf", pbf=True))
    check("MVT mbtiles 图层 test-vector z3",
          lambda: st(MAPAPI + "/mvt/test-vector/3/4/2.pbf", pbf=True))

    # 3D Tiles / terrain
    check("3DTiles 服务目录 GET /3dtiles", lambda: st(MAPAPI + "/3dtiles"))
    check("3DTiles kmczc tileset.json",
          lambda: js(MAPAPI + "/3dtiles/kmczc/tileset.json", lambda j: ("root" in j, "")))
    check("3DTiles recon tileset.json",
          lambda: js(MAPAPI + "/3dtiles/recon/tileset.json", lambda j: ("root" in j, "")))
    check("3DTiles 目录穿越被拒",
          lambda: _want(MAPAPI + "/3dtiles/kmczc/../../../windows/win.ini", (400, 403, 404)))
    check("terrain 服务目录 GET /terrain",
          lambda: js(MAPAPI + "/terrain", lambda j: (j.get("count", 0) >= 1, f"{j.get('count')} 个地形服务")))
    check("terrain layer.json",
          lambda: js(MAPAPI + "/terrain/wuhua12/layer.json", lambda j: ("available" in j, "")))
    check("terrain quantized-mesh 瓦片 8/401/163",
          lambda: st(MAPAPI + "/terrain/wuhua12/8/401/163.terrain"))

    # 错误分流（r-mapserver c131a9b：请求问题/真故障 分 400/404/500）
    check("未知图层 XYZ 返回 404", lambda: _want(MAPAPI + "/tiles/no_such_layer/6/52/25.png", (404,)))
    check("已删除服务不再出瓦片 404",
          lambda: _want(MAPAPI + "/tiles/cog_crs4326_test/6/52/25.png", (404,)))
    check("非法 bbox 的 GetMap 返回 400",
          lambda: _want(MAPAPI + "/wms?service=WMS&version=1.1.1&request=GetMap&layers=world"
                              "&bbox=abc&srs=EPSG:4326&width=256&height=256&format=image/png", (400,)))
    check("空瓦片返回 404 empty tile",
          lambda: _want(MAPAPI + "/mvt/wuhua_building/12/3212/1752.pbf", (404,)))
    check("GET /api/cache 缓存统计", lambda: st(MAPAPI + "/api/cache"))


def _wfs_has_member(path):
    code, _, b = http(MAPAPI + path)
    if code != 200:
        return False, f"HTTP {code} {b[:120]!r}"
    txt = b.decode("utf-8", "replace")
    n = txt.count("member>")
    return (n > 0, f"{n} 个 member，{len(b)}B")


def _want(url, want):
    code, _, b = http(url)
    return (code in want, f"HTTP {code}" + ("" if code in want else f" {b[:100]!r}"))


# ------------------------------------------------------------------ manager 管理面
TOKEN = {}


def _login(username, password):
    code, _, b = http(API + "/auth/login", method="POST",
                      body={"username": username, "password": password})
    if code != 200:
        return False, f"HTTP {code} {b[:160]!r}"
    j = json.loads(b)
    if j.get("code") != 200:
        return False, f"业务码 {j.get('code')} msg={j.get('msg')}"
    tok = j.get("data", {}).get("access_token") or j.get("data", {}).get("token")
    if not tok:
        return False, f"响应未含 token {str(j)[:160]}"
    TOKEN[username] = tok
    return True, ""


def _auth(user, path, method="GET", body=None):
    return http(API + path, method=method, body=body,
                headers={"Authorization": "Bearer " + TOKEN.get(user, "")})


def section_manager():
    print("\n== r-manager 管理面（登录 / 代理 / 引擎 / 配置回写 / 权限）==")

    check("POST /auth/login admin", lambda: _login("admin", "admin123"))
    check("POST /auth/login test(role3)", lambda: _login("test", "666666"))

    check("GET /system/user/getInfo 当前用户",
          lambda: _aj("admin", "/system/user/getInfo", lambda j: (j.get("code") == 200, f"user={j.get('user', {}).get('userName')}")))
    check("GET /system/menu/getRouters admin 菜单树",
          lambda: _aj("admin", "/system/menu/getRouters", _routers_pred))
    check("GET /system/menu/getRouters role3 菜单树",
          lambda: _aj("test", "/system/menu/getRouters", _routers_pred))
    check("GET /map/services 跨引擎聚合",
          lambda: _aj("admin", "/map/services", _services_pred))
    check("GET /map/services?page=1&pageSize=5 分页",
          lambda: _aj("admin", "/map/services?page=1&pageSize=5",
                      lambda j: (len(j.get("data", {}).get("services", [])) == 5,
                                 f"total={j.get('data', {}).get('total')} rows={len(j.get('data', {}).get('services', []))}")))
    check("GET /map/services?kind=postgis 类型过滤",
          lambda: _aj("admin", "/map/services?kind=postgis",
                      lambda j: (all(s.get("kind") == "postgis"
                                     for s in j.get("data", {}).get("services", [])),
                                 f"{len(j.get('data', {}).get('services', []))} 个 postgis 服务")))
    check("GET /map/engines 引擎列表",
          lambda: _aj("admin", "/map/engines",
                      lambda j: (len(j.get("data") or []) >= 1, f"{len(j.get('data') or [])} 个引擎")))
    check("GET /map/engines/map-api/health 健康探测",
          lambda: _aj("admin", "/map/engines/map-api/health",
                      lambda j: (j.get("code") == 200 and (j.get("data") or {}).get("up"), f"data={str(j.get('data'))[:60]}")))
    check("GET /map/engines/map-api/detail 引擎详情",
          lambda: _aj("admin", "/map/engines/map-api/detail",
                      lambda j: (j.get("code") == 200, f"keys={list((j.get('data') or {}).keys())[:6]}")))
    check("GET /map/engines/events 事件日志",
          lambda: _aj("admin", "/map/engines/events?limit=5", lambda j: (j.get("code") == 200, "")))
    check("GET /map/service-groups 分组清单",
          lambda: _aj("admin", "/map/service-groups", lambda j: (j.get("code") == 200, "")))
    check("GET /map/logs 访问统计",
          lambda: _aj("admin", "/map/logs", lambda j: (j.get("code") == 200, "")))
    check("GET /map/browse 数据浏览（向导）",
          lambda: _aj("admin", "/map/browse?path=E:/data/1/cog", lambda j: (j.get("code") == 200, "")))
    check("GET /monitor/online 在线用户",
          lambda: _aj("admin", "/monitor/online", lambda j: (True, f"code={j.get('code')}")))
    check("GET /monitor/logininfor/list 登录日志",
          lambda: _aj("admin", "/monitor/logininfor/list?pageNum=1&pageSize=5",
                      lambda j: (j.get("code") == 200, f"total={j.get('total')}")))

    # 服务配置读写回环（PUT 回写原文 → 内容不变，仅生成 .bak）
    check("GET+PUT /map/services/cog/world 配置读写回环", _config_roundtrip)
    check("role3 PUT 配置（map:service:edit 权限）", lambda: _config_roundtrip("test"))

    # 归属引擎（第二阶段）：读 → 设为默认 → 清除 → 读
    check("GET/PUT/DELETE /map/services/cog/world/engine 归属回环", _engine_roundtrip)

    def unauth():
        code, _, b = http(API + "/map/services")
        return (code in (401, 403), f"HTTP {code}")

    check("未带 token 访问管理接口被拒", unauth)

    def bad_token():
        code, _, b = http(API + "/map/services",
                          headers={"Authorization": "Bearer not-a-valid-jwt"})
        return (code in (401, 403), f"HTTP {code} {b[:60]!r}")

    check("伪造 token 访问管理接口被拒", bad_token)

    def role3_engine_write():
        # role3 具备 map:engine:edit：引擎写路径应到业务层（不存在 → code 500），
        # 而不是被权限层拦下（code/HTTP 403）。
        j = json.loads(_auth("test", "/map/engines/nope", method="DELETE")[2])
        return (j.get("code") != 403, f"code={j.get('code')} msg={str(j.get('msg'))[:40]}")

    check("role3 引擎写路径不被权限层遮蔽", role3_engine_write)


def _aj(user, path, pred, method="GET", body=None):
    code, _, b = _auth(user, path, method=method, body=body)
    if code != 200:
        return False, f"HTTP {code} {b[:160]!r}"
    try:
        j = json.loads(b)
    except Exception:
        return False, "响应非 JSON"
    return pred(j)


def _routers_pred(j):
    if j.get("code") != 200:
        return False, f"code={j.get('code')} msg={j.get('msg')}"
    trees = j.get("data") or []
    if len(trees) < 3:
        return False, f"仅 {len(trees)} 个一级路由"
    return True, f"{len(trees)} 个一级路由 {[t.get('name') or t.get('path') for t in trees]}"


def _services_pred(j):
    if j.get("code") != 200:
        return False, f"code={j.get('code')} msg={str(j.get('msg'))[:80]}"
    d = j.get("data") or {}
    services = d.get("services") or []
    engines = d.get("engines") or []
    if not services:
        return False, "聚合服务清单为空"
    active = sum(1 for s in services if s.get("state") == "active")
    return (True, f"{len(services)} 服务（active {active}）/ 引擎 "
                  f"{[(e.get('engineName') or e.get('name'), e.get('up')) for e in engines]}")


def _config_roundtrip(user="admin"):
    ok, detail = _aj(user, "/map/services/cog/world",
                     lambda j: (j.get("code") == 200 and bool((j.get("data") or {}).get("xml")),
                                f"path={((j.get('data') or {}).get('path') or '')[-24:]}"))
    if not ok:
        return ok, detail
    xml = _auth(user, "/map/services/cog/world")[2]
    orig = json.loads(xml).get("data", {}).get("xml", "")
    if not orig:
        return False, "未取得原文"
    code, _, b = _auth(user, "/map/services/cog/world", method="PUT", body={"xml": orig})
    if code != 200:
        return False, f"PUT HTTP {code} {b[:160]!r}"
    j = json.loads(b)
    if j.get("code") != 200:
        return False, f"PUT 业务码 {j.get('code')} msg={str(j.get('msg'))[:80]}"
    after = json.loads(_auth(user, "/map/services/cog/world")[2]).get("data", {}).get("xml", "")
    return (after == orig, f"served={j.get('data', {}).get('served')} 原文回环一致={after == orig}")


def _engine_roundtrip():
    code, _, b = _auth("admin", "/map/services/cog/world/engine")
    if code != 200:
        return False, f"GET engine HTTP {code} {b[:120]!r}"
    before = json.loads(b).get("data")
    code, _, b = _auth("admin", "/map/services/cog/world/engine", method="PUT", body={"engineId": "map-api"})
    if code != 200 or json.loads(b).get("code") != 200:
        return False, f"PUT engine HTTP {code} {b[:120]!r}"
    mid = json.loads(_auth("admin", "/map/services/cog/world/engine")[2]).get("data") or {}
    _auth("admin", "/map/services/cog/world/engine", method="DELETE")
    after = json.loads(_auth("admin", "/map/services/cog/world/engine")[2]).get("data")
    assigned = mid.get("engineId") if isinstance(mid, dict) else None
    cleared = after is None or (isinstance(after, dict) and after.get("engineId") is None)
    return (assigned == "map-api" and cleared,
            f"归属写入={assigned} 清除后 engineId={isinstance(after, dict) and after.get('engineId')} 初始={before is None}")


# ------------------------------------------------------------------ SPA 静态托管
def section_spa():
    print("\n== 同端口 SPA 静态托管（manager static_dir=../ui/dist）==")
    code, _, b = http(MGR + "/index.html")
    ok = code == 200 and b"<!doctype html" in b.lower()
    results.append(("GET /index.html 首页", ok, f"HTTP {code} {len(b)}B"))
    print(("  PASS " if ok else "  FAIL ") + f"GET /index.html 首页 [HTTP {code} {len(b)}B]")
    code2, _, b2 = http(MGR + "/map/service")
    ok2 = code2 == 200 and b"<!doctype html" in b2.lower()
    results.append(("前端路由回退 /map/service", ok2, f"HTTP {code2} {len(b2)}B"))
    print(("  PASS " if ok2 else "  FAIL ") + f"前端路由回退 /map/service [HTTP {code2} {len(b2)}B]")
    # 静态资源可取（从 index.html 里解析一个 asset 路径）
    import re
    m = re.search(r'/assets/[\w.-]+\.js', b.decode("utf-8", "replace"))
    if m:
        code3, _, _ = http(MGR + m.group(0))
        ok3 = code3 == 200
        results.append(("静态资源 " + m.group(0), ok3, f"HTTP {code3}"))
        print(("  PASS " if ok3 else "  FAIL ") + f"静态资源 {m.group(0)} [HTTP {code3}]")


def main():
    print(f"目标：map-api {MAPAPI} | manager {MGR}")
    section_mapapi()
    section_manager()
    section_spa()
    passed = sum(1 for _, ok, _ in results if ok)
    failed = [n for n, ok, _ in results if not ok]
    print(f"\n==== 合计 {passed}/{len(results)} 通过 ====")
    for n in failed:
        print("  FAILED:", n)
    return len(failed)


if __name__ == "__main__":
    sys.exit(main())
