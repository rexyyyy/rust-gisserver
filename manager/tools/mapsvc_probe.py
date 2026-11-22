"""地图服务管理代理（/map/*）实测。

验证点：
1. admin 与 test 登录后 perms 里是否含 map:*；
2. GET /map/services（admin/test）——代理 map-api /admin/services；
3. GET /map/health、GET /map/usage（day/name）；
4. PUT /map/services/<kind>/<name>/state（test 无权 → 403；admin → 200）；
5. 状态变更回环：active → paused → active，在服标志随动；
6. POST /map/reload（admin）；
7. GET /map/logs —— 访问日志审计查询；
8. POST /map/services/<kind>/<name>/check —— 数据源体检；
9. 样式库闭环：上传（校验/落盘）→ 清单 → 图例 PNG → 原文 → 删除；
10. 未登录 401 / test1 越权 403。
"""
import json
import os
import sys
import urllib.error
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:6180/manager-api"

for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    os.environ.pop(k, None)
os.environ["NO_PROXY"] = "127.0.0.1,localhost"
os.environ["no_proxy"] = "127.0.0.1,localhost"

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

PASS, FAIL = 0, 0


def check(name, ok, detail=""):
    global PASS, FAIL
    if ok:
        PASS += 1
        print(f"  PASS  {name}")
    else:
        FAIL += 1
        print(f"  FAIL  {name}  {detail}")


def req(method, path, token=None, body=None, raw_body=None, content_type=None):
    url = BASE + path
    if raw_body is not None:
        data = raw_body
    else:
        data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    r.add_header("Content-Type", content_type or "application/json")
    if token:
        r.add_header("Authorization", "Bearer " + token)
    try:
        with opener.open(r, timeout=30) as resp:
            payload = resp.read()
            ctype = resp.headers.get("Content-Type", "")
            if "json" in ctype:
                return resp.status, json.loads(payload.decode() or "{}"), payload
            return resp.status, {"_ctype": ctype}, payload
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw or "{}"), raw.encode()
        except Exception:
            return e.code, {"raw": raw[:200]}, raw.encode()


def login(user, pwd):
    st, js, _ = req("POST", "/auth/login", body={"username": user, "password": pwd, "code": "", "uuid": ""})
    if st != 200 or js.get("code") != 200:
        return None, {}
    tok = js["data"]["access_token"]
    st2, js2, _ = req("GET", "/system/user/getInfo", token=tok)
    return tok, js2.get("data") or {}


admin_tok, _ = login("admin", "admin123")
check("admin 登录", bool(admin_tok))
if not admin_tok:
    sys.exit(1)

test_tok, test_info = login("test", "666666")
if test_tok:
    perms = set(test_info.get("permissions") or [])
    check("test perms 含 map:service:list", "map:service:list" in perms, str(sorted(perms)[:8]))
    check("test perms 含 map:service:state", "map:service:state" in perms)
    check("test perms 含 map:log:list", "map:log:list" in perms)
    check("test perms 含 map:style:list", "map:style:list" in perms)

st, js, _ = req("GET", "/map/health", token=admin_tok)
check("GET /map/health", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])
check("map-api 在线", (js.get("data") or {}).get("up") is True, json.dumps(js.get("data")))

st, js, _ = req("GET", "/map/services", token=admin_tok)
check("GET /map/services (admin)", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])
data = js.get("data") or {}
services = data.get("services") or []
check("服务清单非空", data.get("count", 0) > 0, f"count={data.get('count')}")
check("登记簿为 pg 模式", data.get("registry_mode") == "pg", str(data.get("registry_mode")))
svc_with_ep = next((s for s in services if s.get("endpoints")), None)
check("服务清单含协议端点", bool(svc_with_ep), json.dumps(services[0] if services else {}, ensure_ascii=False)[:200])

if test_tok:
    st, js, _ = req("GET", "/map/services", token=test_tok)
    check("GET /map/services (test, 有 list 权限)", st == 200 and js.get("code") == 200)

# 找一个图层服务做状态回环
svc = next((s for s in services if s["kind"] != "3dtiles" and s["state"] == "active"), None)
if svc:
    kind, name = svc["kind"], svc["name"]
    if test_tok:
        st, js, _ = req("PUT", f"/map/services/{kind}/{name}/state", token=test_tok, body={"state": "paused"})
        check("PUT state (test 有 state 权限 → 200)", st == 200 and js.get("code") == 200, f"status={st}")
        req("PUT", f"/map/services/{kind}/{name}/state", token=admin_tok, body={"state": "active"})
    # role 4（test1，仅本人）未授 map 按钮 → 403
    t1_tok, _ = login("test1", "666666")
    if t1_tok:
        st, js, _ = req("PUT", f"/map/services/{kind}/{name}/state", token=t1_tok, body={"state": "paused"})
        check("PUT state (test1 无 state 权限 → 403)", st == 403, f"status={st} body={json.dumps(js, ensure_ascii=False)[:100]}")
        req("PUT", f"/map/services/{kind}/{name}/state", token=admin_tok, body={"state": "active"})
        st, js, _ = req("POST", "/map/styles/probe-deny.sld", token=t1_tok,
                        raw_body=b"<StyledLayerDescriptor/>", content_type="application/xml")
        check("POST style (test1 无 upload 权限 → 403)", st == 403, f"status={st}")
    else:
        check("test1 登录", False, "test1 登录失败")

    # 数据源体检
    st, js, _ = req("POST", f"/map/services/{kind}/{name}/check", token=admin_tok)
    rep = js.get("data") or {}
    check("POST check (admin)", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])
    check("体检返回 checks 列表", isinstance(rep.get("checks"), list) and len(rep.get("checks", [])) > 0,
          json.dumps(rep, ensure_ascii=False)[:200])
    check("体检总体结论存在", isinstance(rep.get("ok"), bool), str(rep.get("ok")))

st, js, _ = req("PUT", f"/map/services/{svc['kind']}/{svc['name']}/state", token=admin_tok, body={"state": "paused"}) if svc else (0, {}, b"")
if svc:
    check("PUT state paused (admin)", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])
    check("paused 后 serving=false", (js.get("data") or {}).get("serving") is False, json.dumps(js.get("data")))
    st, js, _ = req("PUT", f"/map/services/{svc['kind']}/{svc['name']}/state", token=admin_tok, body={"state": "active"})
    check("PUT state 恢复 active", st == 200 and js.get("code") == 200)
    check("恢复后 serving=true", (js.get("data") or {}).get("serving") is True, json.dumps(js.get("data")))
else:
    print("  (跳过状态回环：没有 active 图层服务)")

st, js, _ = req("GET", "/map/usage?period=day&group_by=name", token=admin_tok)
check("GET /map/usage", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])

# 访问日志审计
st, js, _ = req("GET", "/map/logs?limit=50", token=admin_tok)
log = js.get("data") or {}
check("GET /map/logs (admin)", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])
check("日志含 rows/status_counts/errors", isinstance(log.get("rows"), list) and isinstance(log.get("errors"), dict),
      json.dumps(log, ensure_ascii=False)[:120])
st, js, _ = req("GET", "/map/logs?status=2xx&limit=10", token=admin_tok)
rows2 = (js.get("data") or {}).get("rows") or []
check("日志 2xx 过滤", all(200 <= r.get("status", 0) < 300 for r in rows2), f"rows={len(rows2)}")

# 样式库闭环
DEMO_SLD = b"""<StyledLayerDescriptor><NamedLayer><Name>probe</Name>
<UserStyle><Name>probe_style</Name><FeatureTypeStyle>
<Rule><Name>r1</Name><LineSymbolizer><Stroke><SvgParameter name="stroke">#ff0000</SvgParameter></Stroke></LineSymbolizer></Rule>
<Rule><Name>r2</Name><LineSymbolizer><Stroke><SvgParameter name="stroke">#00ff00</SvgParameter></Stroke></LineSymbolizer></Rule>
</FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"""
st, js, _ = req("POST", "/map/styles/probe-demo.sld", token=admin_tok, body={"content": DEMO_SLD.decode()})
check("POST style 上传", st == 200 and (js.get("data") or {}).get("ok") is True, json.dumps(js, ensure_ascii=False)[:150])
check("上传解析出 2 条规则", (js.get("data") or {}).get("rules") == 2, json.dumps(js.get("data")))

st, js, _ = req("GET", "/map/styles", token=admin_tok)
names = [s.get("name") for s in (js.get("data") or {}).get("styles") or []]
check("GET styles 清单含新样式", "probe-demo.sld" in names, str(names))

st, js, payload = req("GET", "/map/styles/probe-demo.sld/legend.png?width=24&height=60", token=admin_tok)
check("GET legend.png 200", st == 200 and payload[:4] == b"\x89PNG", f"status={st} head={payload[:8]!r}")

st, js, _ = req("GET", "/map/styles/probe-demo.sld/content", token=admin_tok)
check("GET style content", "StyledLayerDescriptor" in ((js.get("data") or {}).get("content") or ""), json.dumps(js, ensure_ascii=False)[:120])

st, js, _ = req("POST", "/map/styles/bad.sld", token=admin_tok, body={"content": "<html>not sld</html>"})
check("非法 SLD 上传被拒（400 语义）", st != 200 or js.get("code") != 200, f"status={st} body={json.dumps(js, ensure_ascii=False)[:100]}")

st, js, _ = req("DELETE", "/map/styles/probe-demo.sld", token=admin_tok)
check("DELETE style", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:120])

st, js, _ = req("POST", "/map/reload", token=admin_tok)
check("POST /map/reload", st == 200 and js.get("code") == 200, json.dumps(js, ensure_ascii=False)[:150])

st, js, _ = req("GET", "/map/services", token=None)
check("未登录 → 401", st == 401, f"status={st}")

print()
print(f"结果: {PASS} 过 / {FAIL} 挂")
sys.exit(1 if FAIL else 0)
