#!/usr/bin/env python3
"""Generate in-place language switch patches for the manager seed data.

system.en_US.sql / map.en_US.sql are INSTALL-TIME alternatives to the zh_CN
baselines: every seed row is written with ON CONFLICT DO NOTHING, so running
them against a database that already holds Chinese seed data is a silent no-op
(see README "English seed data (pick one at install time)").

This script derives a patch from the zh_CN <-> en_US pair instead:

  * parse every INSERT ... VALUES tuple in both versions,
  * pair rows by the table primary key (keys never contain Chinese, so they
    are identical in both versions),
  * emit UPDATE statements only for columns whose literal actually differs,
  * carry over the COMMENT ON statements (overwrite-style, always safe).

Custom rows (added by users) and runtime tables are never touched. Every
statement assigns a fixed literal, so the patch is idempotent.

Outputs are build artifacts -- never edit them by hand. Re-run this script
after the zh_CN baselines or i18n_terms.json change.

  python manager/tools/gen_i18n_update_sql.py
  python manager/tools/gen_i18n_update_sql.py --check
"""
from __future__ import annotations

import argparse
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
SQL_DIR = HERE.parent / "backend" / "sql"
SOURCES = ("system.sql", "map.sql")
RESERVED_COLS = {
    "constraint", "primary", "unique", "check", "foreign", "exclude",
    "index", "key", "like", "tablespace", "with", "inherits", "partition"
}
# Some seed rows use an explicit column list that omits the surrogate PK.
FALLBACK_LOCATOR = {"sys_workspace": ["name"]}
# Guard for tables with a partial unique index: soft-deleted copies stay untouched.
EXTRA_WHERE = {"sys_workspace": "del_flag = 0"}

DDL = re.compile(r"(?is)create\s+table\s+(?:if\s+not\s+exists\s+)?(\w+)\s*\((.*?)\n\);")
INSERT = re.compile(r"(?is)^\s*insert\s+into\s+(\w+)\s*(?:\(([^)]*)\))?\s*values\s*(.*)$")


# --------------------------------------------------------------------------
# SQL tokenising
# --------------------------------------------------------------------------
def split_statements(text: str) -> list[str]:
    """Split on semicolons outside strings / dollar quotes / comments."""
    out, buf = [], []
    i, n = 0, len(text)
    in_str = in_dollar = in_line = in_block = False
    while i < n:
        c = text[i]
        if in_line:
            if c == "\n":
                in_line = False
                buf.append(c)
            i += 1
            continue
        if in_block:
            if text.startswith("*/", i):
                in_block = False
                i += 2
            else:
                if c == "\n":
                    buf.append(c)
                i += 1
            continue
        if in_dollar:
            if text.startswith("$$", i):
                in_dollar = False
                buf.append("$$")
                i += 2
            else:
                buf.append(c)
                i += 1
            continue
        if in_str:
            buf.append(c)
            if c == "'":
                if i + 1 < n and text[i + 1] == "'":
                    buf.append("'")
                    i += 2
                    continue
                in_str = False
            i += 1
            continue
        if text.startswith("--", i):
            in_line = True
            i += 2
            continue
        if text.startswith("/*", i):
            in_block = True
            i += 2
            continue
        if text.startswith("$$", i):
            in_dollar = True
            buf.append("$$")
            i += 2
            continue
        if c == "'":
            in_str = True
            buf.append(c)
            i += 1
            continue
        if c == ";":
            stmt = "".join(buf).strip()
            if stmt:
                out.append(stmt)
            buf = []
            i += 1
            continue
        buf.append(c)
        i += 1
    tail = "".join(buf).strip()
    if tail:
        out.append(tail)
    return out


def split_top_level(s: str) -> list[str]:
    """Split a VALUES tuple body on commas at paren depth 0."""
    parts, buf = [], []
    depth, in_str, i = 0, False, 0
    while i < len(s):
        c = s[i]
        if in_str:
            buf.append(c)
            if c == "'":
                if i + 1 < len(s) and s[i + 1] == "'":
                    buf.append(s[i + 1])
                    i += 2
                    continue
                in_str = False
            i += 1
            continue
        if c == "'":
            in_str = True
            buf.append(c)
        elif c == "(":
            depth += 1
            buf.append(c)
        elif c == ")":
            depth -= 1
            buf.append(c)
        elif c == "," and depth == 0:
            parts.append("".join(buf).strip())
            buf = []
        else:
            buf.append(c)
        i += 1
    parts.append("".join(buf).strip())
    return parts


def extract_tuples(rest: str) -> list[list[str]]:
    """Collect every top-level (...) tuple following VALUES."""
    rest = re.split(r"(?i)\b(?:on\s+conflict|returning)\b", rest)[0]
    out, i = [], 0
    while i < len(rest):
        if rest[i] != "(":
            i += 1
            continue
        start, depth, in_str, j = i + 1, 1, False, i + 1
        while j < len(rest) and depth:
            c = rest[j]
            if in_str:
                if c == "'":
                    if j + 1 < len(rest) and rest[j + 1] == "'":
                        j += 2
                        continue
                    in_str = False
            elif c == "'":
                in_str = True
            elif c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
            j += 1
        out.append(split_top_level(rest[start : j - 1]))
        i = j
    return out


# --------------------------------------------------------------------------
# Schema + seed collection
# --------------------------------------------------------------------------
def parse_ddl(text: str) -> dict[str, dict]:
    tables: dict[str, dict] = {}
    for name, body in DDL.findall(text):
        cols = []
        for raw in body.split("\n"):
            m = re.match(r"^\s*(\w+)\s+\S", raw)
            if not m:
                continue
            col = m.group(1).lower()
            if col in RESERVED_COLS:
                continue
            cols.append(col)
        pk = re.search(r"(?is)\bprimary\s+key\s*\(([^)]*)\)", body)
        pk_cols = [c.strip().lower() for c in pk.group(1).split(",")] if pk else []
        if not pk_cols:
            inline = re.search(r"(?im)^\s*(\w+)\s+[^,]*\bprimary\s+key\b", body)
            pk_cols = [inline.group(1).lower()] if inline else []
        tables[name.lower()] = {"cols": cols, "pk": pk_cols}
    return tables


def parse_limits(text: str) -> dict[tuple[str, str], int]:
    """varchar(n) widths declared by the baseline DDL."""
    out: dict[tuple[str, str], int] = {}
    for name, body in DDL.findall(text):
        for col, n in re.findall(r"(?im)^\s*(\w+)\s+varchar\(\s*(\d+)\s*\)", body):
            out[(name.lower(), col.lower())] = int(n)
    return out


def collect(text: str, tables: dict[str, dict]):
    """Return {(table, pk_tokens): (cols, tokens)} plus COMMENT ON statements."""
    rows: dict[tuple, tuple] = {}
    comments: list[str] = []
    skipped: set[str] = set()
    for stmt in split_statements(text):
        low = stmt.lstrip().lower()
        if low.startswith("comment on"):
            comments.append(stmt.strip())
            continue
        if not low.startswith("insert"):
            continue
        m = INSERT.match(stmt)
        if not m:
            continue
        table = m.group(1).lower()
        explicit = [c.strip().lower() for c in m.group(2).split(",")] if m.group(2) else None
        info = tables.get(table)
        cols = explicit or (info["cols"] if info else [])
        pk = info["pk"] if info else []
        locator = pk if pk and all(p in cols for p in pk) else FALLBACK_LOCATOR.get(table, [])
        if not cols or not locator or not all(p in cols for p in locator):
            skipped.add(table)
            continue
        idx = {c: i for i, c in enumerate(cols)}
        for toks in extract_tuples(m.group(3)):
            if len(toks) != len(cols):
                skipped.add(table)
                continue
            key = (table, tuple(toks[idx[p]] for p in locator))
            rows.setdefault(key, (cols, toks))
    return rows, comments, skipped


# --------------------------------------------------------------------------
# Patch rendering
# --------------------------------------------------------------------------
HEADER = """-- =====================================================================
-- Generated by manager/tools/gen_i18n_update_sql.py - DO NOT EDIT BY HAND
-- Direction: {direction}
-- ---------------------------------------------------------------------
-- Why this file exists: the en_US baselines use ON CONFLICT DO NOTHING, so
-- they cannot overwrite an existing zh_CN installation. This patch updates
-- only seed rows present in BOTH baselines, matched by primary key, and only
-- the text columns whose literal differs. Custom rows, runtime tables and
-- non-text business columns (perms / password / config_key / ...) are never
-- touched. Every statement assigns a fixed literal: re-running is a no-op.
-- English labels are longer than Chinese ones: columns whose new value would
-- exceed varchar(n) are widened first (upper bound only, no data rewrite).
--
-- Execute as ONE transaction (the -1 flag wraps the file in a transaction):
--   psql -h localhost -p 5434 -U postgres -d mapserver-manager -v ON_ERROR_STOP=1 -1 -f {filename}
--
-- Rollback: run {rollback} the same way.
-- Source rows: {n_rows} | UPDATE: {n_updates} | COMMENT ON: {n_comments} | widened: {n_alters}
-- COMMENT ON statements run inside a tolerant DO block: baseline objects that
-- are absent in an older database are skipped instead of aborting the patch.
-- =====================================================================

\\set ON_ERROR_STOP on

"""


def render_comments(comments: list[str]) -> list[str]:
    """Wrap COMMENT ON statements so missing baseline objects are tolerated."""
    if not comments:
        return []
    outer = "$i18n$"
    while any(outer in s for s in comments):
        outer += "$"
    inner = "$stmt$"
    while inner == outer or any(inner in s for s in comments):
        inner += "$"
    out = [
        "do " + outer,
        "declare",
        "  s text;",
        "begin",
        "  foreach s in array array[",
    ]
    for i, stmt in enumerate(comments):
        comma = "," if i < len(comments) - 1 else ""
        out.append(f"    {inner}{stmt};{inner}{comma}")
    out += [
        "  ] loop",
        "    begin",
        "      execute s;",
        "    exception when undefined_table or undefined_column or undefined_object then",
        "      null;  -- object not present in this database",
        "    end;",
        "  end loop;",
        "end",
        outer + ";",
    ]
    return out


def render(source_rows, target_rows, comments, tables, limits, direction, filename, rollback) -> tuple[str, dict]:
    both = sorted(set(source_rows) & set(target_rows))
    updates: list[str] = []
    stats: dict[str, int] = {}
    widen: dict[tuple[str, str], int] = {}
    for key in both:
        table, _ = key
        cols, src = source_rows[key]
        _, dst = target_rows[key]
        if len(src) != len(dst):
            raise SystemExit(f"column count mismatch for {key}: {len(src)} vs {len(dst)}")
        idx = {c: i for i, c in enumerate(cols)}
        pk = tables[table]["pk"]
        locator = pk if pk and all(p in cols for p in pk) else FALLBACK_LOCATOR.get(table, [])
        diffs = [(c, dst[i]) for i, c in enumerate(cols) if src[i] != dst[i] and c not in locator]
        if not diffs:
            continue
        bad = [(c, v) for c, v in diffs if not (v.startswith("'") or v.lower() == "null")]
        if bad:
            raise SystemExit(f"non-text diff in {table} {key}: {bad}")
        for c, v in diffs:
            limit = limits.get((table, c))
            if limit and v.startswith("'"):
                need = len(v[1:-1].replace("''", "'"))
                if need > limit:
                    widen[(table, c)] = max(widen.get((table, c), 0), need)
        where = " and ".join(f"{p} = {src[idx[p]]}" for p in locator)
        if table in EXTRA_WHERE:
            where += " and " + EXTRA_WHERE[table]
        sets = ", ".join(f"{c} = {v}" for c, v in diffs)
        updates.append(f"update {table} set {sets} where {where};")
        stats[table] = stats.get(table, 0) + 1
    alters = [
        f"alter table {t} alter column {c} type varchar({max(100, limits[(t, c)] * 2, need)});"
        for (t, c), need in sorted(widen.items())
    ]
    body = alters + updates + render_comments(comments)
    header = HEADER.format(
        direction=direction,
        filename=filename,
        rollback=rollback,
        n_rows=len(both),
        n_updates=len(updates),
        n_comments=len(comments),
        n_alters=len(alters),
    )
    return header + "\n".join(body) + "\n", stats


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="verify generated patches are up to date; do not write")
    args = ap.parse_args()

    zh_text = "\n".join((SQL_DIR / s).read_text(encoding="utf-8") for s in SOURCES)
    tables = parse_ddl(zh_text)
    limits = parse_limits(zh_text)
    zh_rows: dict = {}
    zh_comments: list[str] = []
    en_rows: dict = {}
    en_comments: list[str] = []
    skipped: set[str] = set()
    for src in SOURCES:
        zh = (SQL_DIR / src).read_text(encoding="utf-8")
        en = (SQL_DIR / src.replace(".sql", ".en_US.sql")).read_text(encoding="utf-8")
        r, c, s = collect(zh, tables)
        zh_rows.update(r)
        zh_comments.extend(c)
        skipped |= s
        r, c, s = collect(en, tables)
        en_rows.update(r)
        en_comments.extend(c)
        skipped |= s

    outputs = [
        ("system.sql", "switch_to_en_US.sql", "zh_CN -> en_US", zh_rows, en_rows, zh_comments, en_comments, "switch_to_zh_CN.sql"),
        ("system.sql", "switch_to_zh_CN.sql", "en_US -> zh_CN (rollback)", en_rows, zh_rows, en_comments, zh_comments, "switch_to_en_US.sql"),
    ]
    ok = True
    for _, filename, direction, src_rows, dst_rows, _, comments, rollback in outputs:
        content, stats = render(src_rows, dst_rows, comments, tables, limits, direction, filename, rollback)
        target = SQL_DIR / filename
        if args.check:
            if not target.is_file() or target.read_text(encoding="utf-8") != content:
                print(f"STALE   {filename}: re-run gen_i18n_update_sql.py")
                ok = False
            else:
                print(f"OK      {filename}")
        else:
            target.write_text(content, encoding="utf-8", newline="\n")
            print(f"WROTE   {filename}: {sum(stats.values())} updates / {len(comments)} comments / {content.count('alter table ')} widened columns")
            print(f"        by table: {dict(sorted(stats.items()))}")
    if not args.check:
        print(f"Paired seed rows: zh={len(zh_rows)} en={len(en_rows)} common={len(set(zh_rows) & set(en_rows))}")
        print(f"Tables without a usable PK (skipped): {sorted(skipped)}")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
