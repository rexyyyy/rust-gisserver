"""判定：路由到底存不存在。

若路由已注册但方法不匹配 → 405（Method Not Allowed）
若路由未注册 → 404（axum 空 body fallback）
"""
import json
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:6181/manager-api"

r = urllib.request.Request(
    BASE + "/auth/login",
    data=json.dumps({"username": "admin", "password": "admin123"}).encode(),
    headers={"Content-Type": "application/json"}, method="POST")
with urllib.request.urlopen(r, timeout=20) as resp:
    token = json.loads(resp.read().decode())["data"]["access_token"]

cases = [
    ("GET", "/system/user/profile"),
    ("GET", "/system/user/profile/avatar"),
    ("PUT", "/system/user/profile/avatar"),
    ("POST", "/system/user/profile/updatePwd"),
    ("GET", "/system/user/list?pageNum=1&pageSize=2"),
    ("GET", "/system/user/1"),
    ("GET", "/system/user/deptTree"),
    ("GET", "/system/user/authRole/1"),
    ("GET", "/system/user/list/dept/100"),
]

for method, path in cases:
    req = urllib.request.Request(
        BASE + path, method=method,
        headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        data=b"{}" if method in ("POST", "PUT") else None)
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            body = resp.read().decode("utf-8", "replace")[:90]
            print(f"{method:5s} {path:45s} -> {resp.status}  {body}")
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")[:90]
        print(f"{method:5s} {path:45s} -> {e.code}  {body}")
    except Exception as e:
        print(f"{method:5s} {path:45s} -> EXC {e}")
