"""前端联调冒烟：vite dev server 起后，验证代理链路与静态资源可达。

用法：python manager/tools/ui_smoke.py [vite_url] [backend_base]
默认 vite http://127.0.0.1:5100 ，后端 http://127.0.0.1:6180
"""
import json
import os
import sys
import urllib.error
import urllib.request

VITE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5100"
BACKEND = sys.argv[2] if len(sys.argv) > 2 else "http://127.0.0.1:6180"
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "ui_smoke.txt")

passed = 0
failed = 0
lines = []

BOUNDARY = "----uiSmokeBoundary1a2b3c"
PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
    "1f15c4890000000a49444154789c6300010000050001"
    "0d0a2db40000000049454e44ae426082"
)


def check(name, cond, extra=""):
    global passed, failed
    if cond:
        passed += 1
        lines.append(f"PASS  {name} {extra}")
    else:
        failed += 1
        lines.append(f"FAIL  {name} {extra}")


def get(url, headers=None, timeout=20):
    r = urllib.request.Request(url, headers=headers or {})
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            # HTTP 头不区分大小写，用 resp.headers.get() 取，别转 dict 后按原样键查
            hdr = {k.lower(): v for k, v in resp.headers.items()}
            return resp.status, resp.read(), hdr
    except urllib.error.HTTPError as e:
        hdr = {k.lower(): v for k, v in e.headers.items()}
        return e.code, e.read(), hdr
    except Exception as e:
        return 0, str(e).encode("utf-8", "replace"), {}


def post_json(url, payload, timeout=20):
    r = urllib.request.Request(
        url, data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"msg": raw[:200]}
    except Exception as e:
        return 0, {"msg": str(e)}


def post_multipart(url, token, timeout=30):
    body = b""
    body += ("--%s\r\n" % BOUNDARY).encode()
    body += b'Content-Disposition: form-data; name="avatarfile"; filename="smoke.png"\r\n'
    body += b"Content-Type: image/png\r\n\r\n"
    body += PNG + b"\r\n"
    body += ("--%s--\r\n" % BOUNDARY).encode()
    r = urllib.request.Request(
        url, data=body, method="POST",
        headers={
            "Content-Type": "multipart/form-data; boundary=%s" % BOUNDARY,
            "Authorization": "Bearer " + token,
        })
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"msg": raw[:200]}
    except Exception as e:
        return 0, {"msg": str(e)}


# 1. vite 首页可达
st, body, hdr = get(VITE + "/")
check("vite 首页可达", st == 200 and b"<div id=\"app\"" in body,
      f"status={st} len={len(body)} ct={hdr.get('content-type','')}")

# 2. 登录接口经 vite 代理转发到后端
st, js = post_json(VITE + "/manager-api/auth/login",
                    {"username": "admin", "password": "admin123"})
token = ""
if st == 200 and isinstance(js, dict):
    token = ((js.get("data") or {}).get("access_token")) or ""
check("经 vite 代理登录", st == 200 and bool(token), f"status={st} token={'yes' if token else 'no'}")

if not token:
    lines.append("")
    lines.append("合计: 通过 %d / 失败 %d" % (passed, failed))
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    print("\n".join(lines))
    sys.exit(1)

# 3. 带 token 访问受保护接口
st, body, hdr = get(VITE + "/manager-api/system/user/getInfo",
                     {"Authorization": "Bearer " + token})
try:
    js = json.loads(body.decode("utf-8"))
except Exception:
    js = {}
check("经代理访问 getInfo", st == 200 and js.get("code") == 200, f"status={st}")

# 4. 头像上传经代理
st, js = post_multipart(VITE + "/manager-api/system/user/profile/avatar", token)
img_url = ""
if isinstance(js, dict):
    img_url = ((js.get("data") or {}).get("imgUrl")) or ""
check("经代理上传头像", st == 200 and img_url.startswith("/profile/avatar/"),
      f"status={st} imgUrl={img_url}")

# 5. 头像静态回显经 vite 的 /profile 代理（浏览器 <img> 路径，不带 token）
if img_url:
    st, body, hdr = get(VITE + img_url)
    ct = hdr.get("content-type", "")
    check("经代理回显头像", st == 200 and ct.startswith("image/") and len(body) > 0,
          f"status={st} ct={ct} {len(body)} bytes")

    # 6. 路径穿越防护
    st, body, hdr = get(VITE + "/profile/avatar/../../etc/passwd")
    check("路径穿越被拒", st in (400, 404), f"status={st}")

lines.append("")
lines.append("合计: 通过 %d / 失败 %d" % (passed, failed))
with open(OUT, "w", encoding="utf-8") as f:
    f.write("\n".join(lines))
print("\n".join(lines))
sys.exit(0 if failed == 0 else 1)
