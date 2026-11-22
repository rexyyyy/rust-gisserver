"""单独验证头像上传接口：返回状态、body、以及后端日志线索。"""
import json
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:6181/manager-api"
BOUNDARY = "----probeBoundary7f3a9c1e"

PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
    "1f15c4890000000a49444154789c6300010000050001"
    "0d0a2db40000000049454e44ae426082"
)

body = b""
body += ("--%s\r\n" % BOUNDARY).encode()
body += b'Content-Disposition: form-data; name="avatarfile"; filename="probe.png"\r\n'
body += b"Content-Type: image/png\r\n\r\n"
body += PNG + b"\r\n"
body += ("--%s--\r\n" % BOUNDARY).encode()

# 登录
r = urllib.request.Request(
    BASE + "/auth/login", data=json.dumps({"username": "admin", "password": "admin123"}).encode(),
    headers={"Content-Type": "application/json"}, method="POST")
with urllib.request.urlopen(r, timeout=20) as resp:
    login = json.loads(resp.read().decode())
token = login["data"]["access_token"]
print("token ok:", bool(token))

# 上传
r = urllib.request.Request(
    BASE + "/system/user/profile/avatar", data=body, method="POST",
    headers={
        "Content-Type": "multipart/form-data; boundary=%s" % BOUNDARY,
        "Authorization": "Bearer " + token,
    })
try:
    with urllib.request.urlopen(r, timeout=30) as resp:
        print("STATUS", resp.status)
        print("BODY", resp.read().decode("utf-8", "replace"))
        print("CT", resp.headers.get("Content-Type"))
except urllib.error.HTTPError as e:
    print("STATUS", e.code)
    print("BODY", e.read().decode("utf-8", "replace"))
    print("HEADERS", dict(e.headers))
except Exception as e:
    print("EXC", type(e).__name__, e)

# 对照：确认其它 POST 路由（如已注册的 /system/post）是否同样 404
r = urllib.request.Request(
    BASE + "/system/post",
    data=json.dumps({"postName": "x", "postCode": "x", "postSort": 1, "status": "0"}).encode(),
    headers={"Content-Type": "application/json", "Authorization": "Bearer " + token}, method="POST")
try:
    with urllib.request.urlopen(r, timeout=20) as resp:
        print("POST /system/post ->", resp.status, resp.read().decode("utf-8", "replace")[:120])
except urllib.error.HTTPError as e:
    print("POST /system/post ->", e.code, e.read().decode("utf-8", "replace")[:120])
