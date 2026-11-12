#!/usr/bin/env python3
"""axum 0.8 路由参数语法迁移：路径段内的 :param -> {param}，*rest -> {*rest}

关键约束：只转换**以冒号开头**的路径段（`/:id`），绝不碰普通段（`/map`）。
之前用宽松正则误把 `/map` 改成 `/{map}`，本脚本按段切分后逐段判定。
"""
import re
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent / "src"

LIT = re.compile(r'"((?:[^"\\]|\\.)*)"')
# 段内参数：整段就是一个 :name 或 *name
SEG_PARAM = re.compile(r"^:([A-Za-z_][A-Za-z0-9_]*)$")
SEG_WILD = re.compile(r"^\*([A-Za-z_][A-Za-z0-9_]*)$")


def migrate_literal(body: str) -> str:
    """只转换以 : 或 * 开头的整段。"""
    if not body.startswith("/"):
        return body
    if ":" not in body and "*" not in body:
        return body
    segs = body.split("/")
    changed = False
    for i, seg in enumerate(segs):
        m = SEG_PARAM.match(seg)
        if m:
            segs[i] = "{" + m.group(1) + "}"
            changed = True
            continue
        m = SEG_WILD.match(seg)
        if m:
            segs[i] = "{*" + m.group(1) + "}"
            changed = True
    return "/".join(segs) if changed else body


changed_files = []
changed_lits = 0

for path in sorted(ROOT.rglob("*.rs")):
    src = path.read_text(encoding="utf-8")
    out = []
    last = 0
    file_changed = False

    for m in LIT.finditer(src):
        out.append(src[last:m.start()])
        body = m.group(1)
        new = migrate_literal(body)
        if new != body:
            changed_lits += 1
            file_changed = True
            out.append('"' + new + '"')
        else:
            out.append(m.group(0))
        last = m.end()
    out.append(src[last:])

    if file_changed:
        path.write_text("".join(out), encoding="utf-8")
        changed_files.append(str(path.relative_to(ROOT.parent)))

print(f"改动文件 {len(changed_files)} 个，迁移字面量 {changed_lits} 处")
for f in changed_files:
    print("  ", f)
