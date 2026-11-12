import json
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:6181/manager-api"
OUT = r"D:\mycomputer\documents\gis-wt\thirdparty\manager\mgr_dbg.txt"


def req(method, path, token=None):
    h = {"Authorization": "Bearer " + token} if token else {}
    r = urllib.request.Request(BASE + path, headers=h, method=method)
    try:
        with urllib.request.urlopen(r, timeout=20) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


st, raw = req("POST", "/auth/login", None)
# 登录需要 body，直接用空 body 也行（会失败），改用 urllib 手写
import urllib.parse
data = json.dumps({"username": "admin", "password": "admin123"}).encode()
r = urllib.request.Request(BASE + "/auth/login", data=data,
                           headers={"Content-Type": "application/json"}, method="POST")
with urllib.request.urlopen(r, timeout=20) as resp:
    tok = json.loads(resp.read().decode())["data"]["access_token"]

paths = [
    "/system/user/{userId}",          # 字面量形式：若返回 400 说明 {x} 被当字面量
    "/system/user/1",
    "/system/user/1?a=1",
    "/system/dept/100",
    "/system/dept/{deptId}",
]
out = []
for p in paths:
    st, raw = req("GET", p, token=tok)
    out.append(f"{st:4d}  {p}   {raw[:200]}")
open(OUT, "w", encoding="utf-8").write("\n".join(out))
print("done")
