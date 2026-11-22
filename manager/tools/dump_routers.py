#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""登录 manager 后端并导出 getRouters 原始 JSON，用于前后端路由契约核对。"""
import json
import sys
import urllib.request
import urllib.error

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:6180/manager-api"
OUT = sys.argv[2] if len(sys.argv) > 2 else "routers_raw.json"


def post(path, payload, token=None):
    url = BASE + path
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.loads(r.read().decode("utf-8"))


def get(path, token):
    req = urllib.request.Request(BASE + path, method="GET")
    req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.loads(r.read().decode("utf-8"))


def main():
    login = post("/auth/login", {"username": "admin", "password": "admin123", "code": "", "uuid": ""})
    if login.get("code") != 200:
        print("LOGIN FAIL", json.dumps(login, ensure_ascii=False)[:500])
        return 1
    token = login["data"]["access_token"]
    print("LOGIN 200 token ok")
    res = get("/system/menu/getRouters", token)
    if res.get("code") != 200:
        print("ROUTERS FAIL", json.dumps(res, ensure_ascii=False)[:500])
        return 1
    routers = res["data"]
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(routers, f, ensure_ascii=False, indent=2)
    print("ROUTERS 200 count=%d saved=%s" % (len(routers), OUT))

    def walk(nodes, depth=0):
        for n in nodes:
            print("  " * depth + "- path=%-28s component=%-22s name=%-22s hidden=%s link=%s children=%d"
                  % (repr(n.get("path")), repr(n.get("component")), repr(n.get("name")),
                     n.get("hidden"), repr((n.get("meta") or {}).get("link")),
                     len(n.get("children") or [])))
            if n.get("children"):
                walk(n["children"], depth + 1)

    walk(routers)
    return 0


if __name__ == "__main__":
    sys.exit(main())
