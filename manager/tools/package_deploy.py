#!/usr/bin/env python3
"""Pack a deployable source bundle (zip) for rust-gisserver.

Why a source bundle
-------------------
The two service images are built from source (no registry push in this repo),
so the target machine needs: the Rust sources (gisserver + mapserver, which are
a path-dependency pair and MUST stay siblings), the manager backend + UI
sources, and the deploy/ orchestration assets.

What is excluded
----------------
Build artifacts and local-only data, which are large and/or machine-specific:

    */target/            Rust build output (10+ GB)
    */node_modules/      npm/pnpm install output (~1 GB)
    */dist*/             frontend build output
    2026-11-12/          13 GB full-test scratch workspace
    build/ run-env/      local build + installed runtime copies
    .git/                history (not needed to build)
    */storage/           runtime data (avatars)

The bundle keeps the repository-root layout because:
  * mapserver/Dockerfile builds with the repo root as context and does
    `COPY gisserver ./gisserver` (path dependency ../gisserver),
  * the compose files reference ../../.. (repo root) and ../.. (manager/).

Usage:
    python manager/tools/package_deploy.py                 # -> dist/rust-gisserver-deploy-<ts>.zip
    python manager/tools/package_deploy.py -o /tmp/out.zip # custom output path
    python manager/tools/package_deploy.py --platform linux # only the linux platform entry
"""
from __future__ import annotations

import argparse
import fnmatch
import os
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# Directories (relative to repo root) whose entire subtree is copied verbatim.
INCLUDE_TREES = [
    "gisserver/src",
    "gisserver/tests",
    "mapserver/services",
    "manager/backend/src",
    "manager/backend/sql",
    "manager/backend/locales",
    "manager/backend/config",
    "manager/backend/tools",
    "manager/ui/src",
    "manager/ui/public",
    "manager/ui/vite",
    "manager/ui/scripts",
    "manager/deploy/config",
]

# Individual files copied verbatim.
INCLUDE_FILES = [
    ".dockerignore",
    ".gitattributes",
    "gisserver/Cargo.toml",
    "gisserver/Cargo.lock",
    "gisserver/LICENSE",
    "mapserver/Cargo.toml",
    "mapserver/Cargo.lock",
    "mapserver/LICENSE",
    "mapserver/Dockerfile",
    "mapserver/docker-entrypoint.sh",
    "manager/Dockerfile",
    "manager/docker-entrypoint.sh",
    "manager/.dockerignore",
    "manager/backend/Cargo.toml",
    "manager/ui/package.json",
    "manager/ui/pnpm-lock.yaml",
    "manager/ui/pnpm-workspace.yaml",
    "manager/ui/index.html",
    "manager/ui/tsconfig.json",
    "manager/ui/vite.config.ts",
    "manager/ui/uno.config.ts",
    "manager/ui/eslint.config.ts",
]

# Deployment orchestration: language x platform compose entries plus the shared
# asset dir (config templates, .env.example, start/validate scripts, README).
DEPLOY_DIRS = [
    "manager/deploy",
    "manager/deploy_zh-cn",
    "manager/deploy_en",
]

# Never ship these even if they sit inside an included tree.
EXCLUDE_DIR_NAMES = {
    "target", "node_modules", "dist", "dist_bak", "dist_gs2", "dist_gs3",
    "dist_old_gs", "dist_paged", ".vite", ".git", "__pycache__", "storage",
    "_node_modules_broken", ".idea", ".vscode",
}
EXCLUDE_FILE_NAMES = {".DS_Store", "Thumbs.db"}
# Local secrets / machine-specific artifacts that must never enter the bundle.
# NB: *.env.example is the deployment template — it MUST ship (.env must not).
EXCLUDE_FILE_SUFFIX = {".log", ".pid", ".bak", ".swp"}
EXCLUDE_PATTERNS = [
    ".env",
    "*/.env",
    "*/gisdata/*",
    "*.local",
]


def excluded(rel_path: str, is_dir: bool) -> bool:
    """True when the path (repo-relative, posix separators) must be skipped."""
    parts = Path(rel_path).parts
    for part in parts:
        if part in EXCLUDE_DIR_NAMES:
            return True
        if not is_dir and part in EXCLUDE_FILE_NAMES:
            return True
    name = parts[-1]
    if not is_dir:
        if Path(name).suffix in EXCLUDE_FILE_SUFFIX:
            return True
    for pattern in EXCLUDE_PATTERNS:
        if fnmatch.fnmatch(rel_path, pattern):
            return True
    return False


def collect() -> list[tuple[Path, str]]:
    """Return [(absolute source path, archive name)] pairs, in stable order."""
    items: list[tuple[Path, str]] = []

    for tree in INCLUDE_TREES:
        root = REPO / tree
        if not root.exists():
            print(f"  ! 缺失，跳过: {tree}", file=sys.stderr)
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            rel_dir = Path(dirpath).relative_to(REPO).as_posix()
            dirnames[:] = [d for d in sorted(dirnames) if d not in EXCLUDE_DIR_NAMES]
            for fname in sorted(filenames):
                src = Path(dirpath) / fname
                rel = src.relative_to(REPO).as_posix()
                if excluded(rel, is_dir=False):
                    continue
                items.append((src, rel))

    for f in INCLUDE_FILES:
        src = REPO / f
        if not src.exists():
            print(f"  ! 缺失，跳过: {f}", file=sys.stderr)
            continue
        items.append((src, f))

    for d in DEPLOY_DIRS:
        root = REPO / d
        if not root.exists():
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            rel_dir = Path(dirpath).relative_to(REPO).as_posix()
            dirnames[:] = [
                x for x in sorted(dirnames)
                if x not in EXCLUDE_DIR_NAMES and not excluded(f"{rel_dir}/{x}", True)
            ]
            for fname in sorted(filenames):
                src = Path(dirpath) / fname
                rel = src.relative_to(REPO).as_posix()
                if excluded(rel, is_dir=False):
                    continue
                items.append((src, rel))

    # de-duplicate while preserving order
    seen: set[str] = set()
    unique: list[tuple[Path, str]] = []
    for src, rel in items:
        if rel in seen:
            continue
        seen.add(rel)
        unique.append((src, rel))
    return unique


DEPLOY_GUIDE = """\
# rust-gisserver 部署包（源码 bundle）

本包是 **源码 + 编排配置** 的部署包：两台机器的镜像都在目标机现场构建
（仓库未推镜像仓库），因此目标机需要 Docker Desktop（Linux 容器模式）+
可访问外网（拉 Rust crates、npm 包、PostGIS 镜像）。

## 1. 解压

    unzip rust-gisserver-deploy-*.zip -d <目标目录>

解压后得到一个含 `gisserver/`、`mapserver/`、`manager/` 的仓库根目录。
**目录层级必须保持原样** —— mapserver 的核心库是 path 依赖 `../gisserver`，
且 compose 的 build context 依赖相对路径（`../../..` = 仓库根，`../..` = manager/）。

## 2. 选平台目录

    manager/deploy_zh-cn/win/     中文 + Windows
    manager/deploy_zh-cn/linux/   中文 + Linux
    manager/deploy_en/win/        英文 + Windows
    manager/deploy_en/linux/      英文 + Linux

Windows 用 `win/`，Linux 用 `linux/`；中文用 `deploy_zh-cn`，英文用 `deploy_en`。
四个目录是独立 compose 项目（卷互不共享），端口相同不能同时起。

## 3. 配置 .env

    cd manager/deploy_zh-cn/linux        # 按实际平台/语言替换
    cp ../../deploy/.env.example .env

至少改 `MANAGER_JWT_SECRET`（必填，compose 用 `:?` 强校验）；共享环境务必把
`MAPSERVER_DB_PASSWORD` 改成强口令。Windows 用 `copy` 代替 `cp`。

## 4. 启动

Linux：

    bash ../../deploy/start.sh --dir .

Windows：

    pwsh -File ..\\..\\deploy\\start.ps1 -Dir .

脚本会：读 .env → docker compose up -d --build → 等 PostGIS healthy → 自动自检。
也可手动：

    docker compose up -d --build

首次构建要现场编译 Rust 与前端，视网络与机器性能约 10~30 分钟。

## 5. 验收

    前端    : http://localhost:6180/
    map-api : http://localhost:8080/health
    WMS     : http://localhost:8080/wms?service=WMS&request=GetCapabilities

`/health` 返回里应有 `"proj":true`：

    {"status":"ok","proj":true,"mode":"standalone","manager":null}

`proj:true` 表示 PROJ 投影后端已加载 —— 跨 CRS 重投影（EPSG:4490 / 高斯-克吕格
45xx / UTM）可用。`proj:false` 不影响服务启动，但上述跨 CRS 请求会失败，此时查：

    docker compose logs map-api | grep -i proj

自检（任一 FAIL 退出码 1）：

    bash ../../deploy/validate.sh --dir . --check-data-root

自检输出里的 `PROJ 后端` 一行即 PROJ 状态（PASS / WARN）。

## 常见问题

- **构建慢/超时**：首次拉 crates + npm 包。若目标机在墙内，构建已默认走 USTC
  rustup 源；npm 源可在 `manager/ui/.npmrc` 追加镜像。
- **Cargo.lock 依赖**：包内已带 `gisserver/Cargo.lock` 与 `mapserver/Cargo.lock`，
  版本已固定，离线构建仍需 crates 源可达。
- **端口被占**：改 `.env` 的 `MAPSERVER_DB_PORT`（DB）、`GISDATA_DIR`（数据挂载点）。
- **英文库显示中文菜单**：英文库种子只影响 DB 文本，中文请求下会显示英文
  （中间件无 en→zh 路径），属预期。
"""


# Shell scripts must be LF inside the bundle: the Windows working tree uses
# core.autocrlf=true, so the on-disk files carry CRLF and would fail on Linux
# with "^M: bad interpreter". Normalize on write (git's index is already LF).
LF_ONLY_SUFFIX = {".sh"}


def write_entry(zf: zipfile.ZipFile, src: Path, rel: str) -> None:
    """Add a file to the archive, normalizing line endings for shell scripts."""
    if src.suffix in LF_ONLY_SUFFIX:
        data = src.read_bytes().replace(b"\r\n", b"\n")
        zf.writestr(rel, data)
    else:
        zf.write(src, rel)


def main() -> int:
    ap = argparse.ArgumentParser(description="打包 rust-gisserver 部署 zip")
    ap.add_argument("-o", "--output", default="", help="输出 zip 路径")
    ap.add_argument("--platform", default="", choices=["linux", "win"],
                    help="只保留单个平台的 compose 入口（默认全带）")
    ap.add_argument("--dist", default="dist", help="默认输出目录（相对仓库根）")
    args = ap.parse_args()

    ts = time.strftime("%Y%m%d-%H%M%S")
    out = Path(args.output) if args.output else REPO / args.dist / f"rust-gisserver-deploy-{ts}.zip"
    out.parent.mkdir(parents=True, exist_ok=True)

    items = collect()
    if args.platform:
        keep = f"/{args.platform}/"
        items = [it for it in items if keep in f"/{it[1]}"]
        if not items:
            print("过滤后没有文件", file=sys.stderr)
            return 1

    print(f"打包 {len(items)} 个文件 -> {out}")
    total = 0
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for src, rel in items:
            write_entry(zf, src, rel)
            total += src.stat().st_size
        zf.writestr("DEPLOY.md", DEPLOY_GUIDE)

    size_mb = out.stat().st_size / 1024 / 1024
    print(f"完成: {out}  ({size_mb:.1f} MB, 原始 {total / 1024 / 1024:.1f} MB)")
    print("包内已含 DEPLOY.md（目标机部署步骤）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
