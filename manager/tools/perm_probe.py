"""非 admin 角色的权限推导实测。

验证点：
1. test 用户（role=3，本部门及以下）能否登录、拿到哪些 perms；
2. 用 test 登录后调用需要按钮级权限的端点，看是否被 403 误拒。
"""
import json
import os
import sys
import urllib.error
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:6180/manager-api"

# 沙箱里 HTTP_PROXY 会劫走本地请求
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    os.environ.pop(k, None)
os.environ["NO_PROXY"] = "127.0.0.1,localhost"
os.environ["no_proxy"] = "127.0.0.1,localhost"

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def req(method, path, token=None, body=None):
    url = BASE + path
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    r.add_header("Content-Type", "application/json")
    if token:
        r.add_header("Authorization", "Bearer " + token)
    try:
        with opener.open(r, timeout=15) as resp:
            return resp.status, json.loads(resp.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw or "{}")
        except Exception:
            return e.code, {"raw": raw[:200]}


def login(user, pwd):
    st, js = req("POST", "/auth/login", body={"username": user, "password": pwd, "code": "", "uuid": ""})
    if st != 200 or js.get("code") != 200:
        print(f"  login {user} FAILED status={st} body={js}")
        return None, None
    tok = js["data"]["access_token"]
    st2, js2 = req("GET", "/system/user/getInfo", token=tok)
    return tok, js2.get("data") or {}


print("=== admin 登录 ===")
admin_tok, admin_info = login("admin", "admin123")
if not admin_tok:
    print("admin 登录失败，先查后端是否在跑")
    sys.exit(1)
print("  roles:", admin_info.get("roles"))
print("  perms 数量:", len(admin_info.get("permissions") or []))

print()
print("=== test 登录（role=3 本部门及以下）===")
test_tok, test_info = login("test", "666666")
if not test_tok:
    print("test 登录失败")
    sys.exit(1)
perms = test_info.get("permissions") or []
print("  roles:", test_info.get("roles"))
print("  perms 数量:", len(perms))
print("  perms:", sorted(perms))

print()
print("=== 用 test token 调按钮级端点（应为 200，若 403 即推导错） ===")
cases = [
    ("GET", "/system/user/list", None, None),
    ("GET", "/system/role/list", None, None),
    ("GET", "/system/menu/list", None, None),
    ("GET", "/monitor/operlog/list", None, None),
    ("GET", "/monitor/logininfor/list", None, None),
]
for method, path, token, body in cases:
    st, js = req(method, path, token=test_tok, body=body)
    msg = js.get("msg") or js.get("raw", "")
    print(f"  {method:6} {path:32} -> {st}  {js.get('code')}  {str(msg)[:60]}")

print()
print("=== 造一个临时资源，用 test 做 DELETE（验证 delete 权限推导） ===")
st, js = req("POST", "/system/post", token=test_tok,
             body={"postName": "perm_probe_tmp", "postCode": "perm_probe_tmp",
                   "postSort": 99, "status": "0", "remark": "probe"})
print("  POST /system/post ->", st, js.get("code"), str(js.get("msg"))[:60])
new_id = None
if st == 200 and js.get("code") == 200 and js.get("data") is not None:
    d = js["data"]
    new_id = d.get("postId") or d.get("id") if isinstance(d, dict) else d
if new_id:
    st, js = req("DELETE", f"/system/post/{new_id}", token=test_tok)
    print(f"  DELETE /system/post/{new_id} -> {st}  {js.get('code')}  {str(js.get('msg'))[:60]}")
    # 清理
    st2, js2 = req("DELETE", f"/system/post/{new_id}", token=admin_tok)
    print(f"  (admin 清理) DELETE -> {st2} {js2.get('code')}")
else:
    print("  创建失败，无法测 DELETE。返回:", str(js)[:200])
