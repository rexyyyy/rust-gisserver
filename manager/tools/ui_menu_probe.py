#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""浏览器端菜单逐项实测：登录后点击侧边栏每个菜单，记录 URL 与是否 404。

用法：python manager/tools/ui_menu_probe.py [vite_url]
需要 agent-browser CLI 可执行；本脚本会自行调用并清代理。
"""
import json
import os
import re
import subprocess
import sys

VITE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5100"

AB_DIR = r"C:\Users\Administrator\.workbuddy\binaries\node\versions\22.22.2-3"
AB = [os.path.join(AB_DIR, "node.exe"),
      os.path.join(AB_DIR, "node_modules", "agent-browser", "bin", "agent-browser.js")]

# Chromium 会读代理环境变量，本地地址必须绕过，否则导航到 about:blank
ENV = dict(os.environ)
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    ENV.pop(k, None)
ENV["NO_PROXY"] = "127.0.0.1,localhost"
ENV["no_proxy"] = "127.0.0.1,localhost"

MENU = [
    "用户管理", "角色管理", "菜单管理", "部门管理", "岗位管理",
    "字典管理", "参数设置", "通知公告", "日志管理", "操作日志", "登录日志",
]


def ab(*args, timeout=90):
    p = subprocess.run(AB + list(args), capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=ENV, timeout=timeout)
    return (p.stdout or "").strip()


def js(expr):
    out = ab("eval", expr)
    try:
        return json.loads(out)
    except Exception:
        return out.strip().strip('"')


def main():
    ab("close", "--all")
    ab("open", VITE + "/login")
    ab("wait", "2000")
    ab("fill", "input[placeholder='用户名']", "admin")
    ab("fill", "input[placeholder='密码']", "admin123")
    ab("click", "button")
    ab("wait", "3500")
    url = ab("get", "url")
    print("LOGIN -> %s" % url)
    if "/index" not in url:
        print("LOGIN FAILED")
        return 1

    results = []
    for m in MENU:
        # 目录型菜单需先展开，逐个点击时用可见项匹配
        clicked = js(
            "(()=>{const els=[...document.querySelectorAll('.el-menu-item,.el-sub-menu__title')]"
            ".filter(e=>(e.innerText||'').trim()==='%s' && e.offsetParent!==null);"
            "if(!els.length) return 'NO_VISIBLE';els[0].click();return 'CLICKED';})()" % m
        )
        ab("wait", "1600")
        u = ab("get", "url").replace("\r", "")
        body = js("document.body.innerText")
        is404 = "访问的页面不存在" in (body or "")
        results.append((m, clicked, u, "404" if is404 else "OK"))
        print("%-8s %-10s %-42s %s" % (m, clicked, u, "404" if is404 else "OK"))

    bad = [r for r in results if r[3] == "404"]
    print("\n合计: %d 项, 404: %d" % (len(results), len(bad)))
    for r in bad:
        print("  404 -> %s" % r[0])
    return 1 if bad else 0


if __name__ == "__main__":
    try:
        rc = main()
    finally:
        ab("close", "--all")
    sys.exit(rc)
