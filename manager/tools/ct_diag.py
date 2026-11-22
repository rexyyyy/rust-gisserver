"""区分「路由未注册」与「content-type/请求体导致」。

对照：向已注册的 POST 路由发 multipart 请求，看返回 404 还是 400/200。
"""
import json
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:6181/manager-api"
BOUNDARY = "----b1"

r = urllib.request.Request(
    BASE + "/auth/login",
    data=json.dumps({"username": "admin", "password": "admin123"}).encode(),
    headers={"Content-Type": "application/json"}, method="POST")
with urllib.request.urlopen(r, timeout=20) as resp:
    token = json.loads(resp.read().decode())["data"]["access_token"]

PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
    "1f15c4890000000a49444154789c6300010000050001"
    "0d0a2db40000000049454e44ae426082"
)
mp = b""
mp += ("--%s\r\n" % BOUNDARY).encode()
mp += b'Content-Disposition: form-data; name="avatarfile"; filename="p.png"\r\n'
mp += b"Content-Type: image/png\r\n\r\n"
mp += PNG + b"\r\n"
mp += ("--%s--\r\n" % BOUNDARY).encode()

cases = [
    # (说明, method, path, body, headers)
    ("POST 已注册路由 + multipart", "POST", "/system/user/profile/avatar", mp,
     {"Content-Type": "multipart/form-data; boundary=%s" % BOUNDARY}),
    ("POST 已注册路由 + json", "POST", "/system/post", None,
     {"Content-Type": "application/json"}),
    ("POST 已注册路由 + multipart(其它路由)", "POST", "/system/post", mp,
     {"Content-Type": "multipart/form-data; boundary=%s" % BOUNDARY}),
    ("POST 头像路由 + 空 body 无 multipart", "POST", "/system/user/profile/avatar", None,
     {"Content-Type": "application/json"}),
]

for desc, method, path, body, hdr in cases:
    h = dict(hdr)
    h["Authorization"] = "Bearer " + token
    req = urllib.request.Request(BASE + path, data=body, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            b = resp.read().decode("utf-8", "replace")[:120]
            print(f"{desc:42s} -> {resp.status} {b}")
            print(f"{'':42s}    Allow={resp.headers.get('Allow')} CT={resp.headers.get('Content-Type')}")
    except urllib.error.HTTPError as e:
        b = e.read().decode("utf-8", "replace")[:120]
        print(f"{desc:42s} -> {e.code} {b}")
        print(f"{'':42s}    Allow={e.headers.get('Allow')} CT={e.headers.get('Content-Type')}")
    except Exception as e:
        print(f"{desc:42s} -> EXC {type(e).__name__} {e}")
