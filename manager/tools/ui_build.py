"""续装 / 构建 r-mapserver manager 前端依赖。

优先使用 PATH 里的 pnpm（本项目用 pnpm-lock.yaml，npm 安装会产出错误依赖树）；
找不到时才把 FALLBACK_NODE_DIR 追加进 PATH —— 那是工作机上由工具链托管的 node
目录，换机器只需改这一处（或直接把 pnpm 装进 PATH）。
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

FALLBACK_NODE_DIR = Path(os.path.expandvars(r"%USERPROFILE%")) / ".workbuddy" / "binaries" / "node" / "versions" / "22.22.2-3"
UI = Path(__file__).resolve().parents[1] / "ui"

mode = sys.argv[1] if len(sys.argv) > 1 else "install"
env = dict(os.environ)

pkg = shutil.which("pnpm")
if not pkg:
    env["PATH"] = os.pathsep.join([str(FALLBACK_NODE_DIR), env.get("PATH", "")])
    pkg = shutil.which("pnpm") or "pnpm"
    print(f"[UI-BUILD] pnpm not in PATH, fallback dir: {FALLBACK_NODE_DIR}")

if mode == "install":
    cmd = [pkg, "install"]
else:
    script = "build:prod" if mode == "prod" else "build:dev"
    cmd = [pkg, "run", script]

r = subprocess.run(cmd, cwd=UI, capture_output=True, text=True, errors="replace", timeout=3600, env=env)
print("EXIT", r.returncode)
print(r.stdout[-6000:])
if r.stderr.strip():
    print("---- STDERR ----")
    print(r.stderr[-3000:])
sys.exit(r.returncode)
