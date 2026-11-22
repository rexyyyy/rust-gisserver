#!/usr/bin/env python3
"""把 axum 0.7 的路由参数语法迁移到 0.8。

0.7: `/tiles/:layer/:z/:x/:y`   `/*rest`
0.8: `/tiles/{layer}/{z}/{x}/{y}`   `/{*rest}`

注意（踩过的坑）：**不能**用简单的正则 `(?<=[:/])([A-Za-z_]\\w*)` 全局替换——
那会把普通路径段一起改掉（`/map` → `/{map}`、`/engines` → `/{engines}`）。
正确做法是按 `/` 切段后逐段判定，只转换整段就是 `:name` / `*name` 的段。
"""

import re
import sys
from pathlib import Path

# 整段匹配：`:name` 或 `*name`（段内不含其它字符）
PARAM_SEG = re.compile(r"^:([A-Za-z_]\w*)$")
CATCHALL_SEG = re.compile(r"^\*([A-Za-z_]\w*)$")

# 只在字符串字面量内部做替换，且该字面量以 `/` 开头（路由路径）
STRING_LIT = re.compile(r'"(/[^"]*)"')


def migrate_path(path: str) -> tuple[str, int]:
    """把路径里的 `:name` / `*name` 段转为 `{name}` / `{*name}`。"""
    count = 0
    out = []
    for seg in path.split("/"):
        m = PARAM_SEG.match(seg)
        if m:
            out.append("{" + m.group(1) + "}")
            count += 1
            continue
        m = CATCHALL_SEG.match(seg)
        if m:
            out.append("{*" + m.group(1) + "}")
            count += 1
            continue
        out.append(seg)
    return "/".join(out), count


def migrate_text(text: str) -> tuple[str, int]:
    """替换文本中所有形如 "/..." 的字符串字面量里的路由参数语法。"""

    def repl(m: re.Match) -> str:
        new_path, n = migrate_path(m.group(1))
        return '"' + new_path + '"'

    total = 0

    def counting_repl(m: re.Match) -> str:
        nonlocal total
        new_path, n = migrate_path(m.group(1))
        total += n
        return '"' + new_path + '"'

    return STRING_LIT.sub(counting_repl, text), total


def main() -> int:
    roots = [Path("services"), Path("crates")]
    files = []
    for r in roots:
        if not r.exists():
            continue
        files.extend(p for p in r.rglob("*.rs"))

    changed_files = 0
    changed_segs = 0
    for f in files:
        src = f.read_text(encoding="utf-8")
        out, n = migrate_text(src)
        if n and out != src:
            f.write_text(out, encoding="utf-8")
            changed_files += 1
            changed_segs += n
            print(f"  {f}  ({n} 处)")

    print(f"\n迁移完成：{changed_files} 个文件，{changed_segs} 处路径段")
    return 0


if __name__ == "__main__":
    sys.exit(main())
