#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""浏览器实测「地图服务 → 服务管理」菜单：登录、点菜单、验证列表加载、截图。"""
import json
import os
import subprocess
import sys
import time

VITE = "http://127.0.0.1:5100"
SHOT = r"E:\tmp\map_service_page.png"

AB_DIR = r"C:\Users\Administrator\.workbuddy\binaries\node\versions\22.22.2-3"
AB = [os.path.join(AB_DIR, "node.exe"),
      os.path.join(AB_DIR, "node_modules", "agent-browser", "bin", "agent-browser.js")]

ENV = dict(os.environ)
for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
    ENV.pop(k, None)
ENV["NO_PROXY"] = "127.0.0.1,localhost"
ENV["no_proxy"] = "127.0.0.1,localhost"


def ab(*args, timeout=45):
    p = subprocess.run(AB + list(args), capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=ENV, timeout=timeout)
    return (p.stdout or "").strip()


def js(expr, timeout=45):
    out = ab("eval", expr, timeout)
    try:
        return json.loads(out)
    except Exception:
        return out.strip().strip('"')


def main():
    t0 = time.time()
    def log(*a):
        print(f"[{time.time()-t0:5.1f}s]", *a, flush=True)

    ab("close", "--all")
    log("daemon ready")
    ab("open", VITE + "/login")
    ab("wait", "2500")
    ab("fill", "input[placeholder='用户名']", "admin")
    ab("fill", "input[placeholder='密码']", "admin123")
    ab("click", "button")
    ab("wait", "3500")
    url = ab("get", "url")
    log("LOGIN ->", url)
    if "/index" not in url:
        log("LOGIN FAILED, page text:", js("document.body.innerText.slice(0,300)"))
        return 1

    # 点开侧边栏「地图服务」目录，再点「服务管理」
    ab("click", "text=地图服务")
    ab("wait", "1200")
    ab("click", "text=服务管理")
    ab("wait", "3000")
    url = ab("get", "url")
    log("MENU ->", url)

    rows = js("JSON.stringify(Array.from(document.querySelectorAll('.el-table__row')).map(r=>Array.from(r.cells).map(c=>c.innerText.trim()).slice(0,5)))")
    log("表格行数:", len(rows) if isinstance(rows, list) else rows)
    log("表格数据:", json.dumps(rows, ensure_ascii=False)[:600])

    tag = js("JSON.stringify((document.body.innerText.match(/map-api (在线|离线)/)||[])[1]||null)")
    log("引擎状态:", tag)
    mode = js("JSON.stringify((document.body.innerText.match(/PostgreSQL|文件 sidecar/)||[])[0]||null)")
    log("登记簿:", mode)

    ab("screenshot", SHOT)
    log("截图:", SHOT, os.path.exists(SHOT))

    ok = isinstance(rows, list) and len(rows) > 0 and tag == "在线"
    log("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
