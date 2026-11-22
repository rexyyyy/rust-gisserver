#!/usr/bin/env python3
"""Generate en_US seed SQL from the zh_CN sources (system.sql / map.sql).

Only Chinese inside single-quoted SQL string literals is translated. DDL,
column order, comments and every non-text token are preserved verbatim, so the
English file is structurally identical and can be used instead of the Chinese
one at install time (pick exactly one, never both).

Generated files are build artifacts -- never edit them by hand. Edit the
mapping in i18n_terms.json and re-run this script.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
SQL_DIR = HERE.parent / "backend" / "sql"
TERMS = json.loads((HERE / "i18n_terms.json").read_text(encoding="utf-8"))
PHRASES = TERMS["phrases"]
WORDS = sorted(TERMS["words"].items(), key=lambda kv: -len(kv[0]))
HAN = re.compile(r"[\u3400-\u9fff]")
LITERAL = re.compile(r"'([^']*)'")
SOURCES = ("system.sql", "map.sql")


def translate(value: str) -> str:
    if value in PHRASES:
        return PHRASES[value]
    out = value
    for zh, en in WORDS:
        out = out.replace(zh, en)
    # Word values carry a leading space so combinations read "User Management"
    # instead of "UserManagement"; collapse the resulting runs afterwards.
    return re.sub(r"[ \t]+", " ", out).strip()


def render(src: str) -> tuple[str, list[str]]:
    missing: list[str] = []

    def repl(match: re.Match[str]) -> str:
        value = match.group(1)
        if not HAN.search(value):
            return match.group(0)
        translated = translate(value)
        if HAN.search(translated):
            missing.append(value)
        return "'" + translated.replace("'", "''") + "'"

    lines = [LITERAL.sub(repl, line) for line in src.split("\n")]
    return "\n".join(lines), missing


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="verify generated files are up to date; do not write")
    args = ap.parse_args()

    ok = True
    missing: list[tuple[str, str]] = []
    for name in SOURCES:
        src = (SQL_DIR / name).read_text(encoding="utf-8")
        rendered, missed = render(src)
        missing.extend((name, value) for value in missed)
        target = SQL_DIR / (name[:-4] + ".en_US.sql")
        if args.check:
            if not target.is_file():
                print(f"MISSING {target.name}")
                ok = False
            elif target.read_text(encoding="utf-8") != rendered:
                print(f"STALE   {target.name}: re-run gen_i18n_sql.py")
                ok = False
            else:
                print(f"OK      {target.name} ({len(src.splitlines())} lines)")
        else:
            target.write_text(rendered, encoding="utf-8", newline="\n")
            print(f"WROTE   {target.name} ({len(src.splitlines())} lines)")

    unique = sorted({value for _, value in missing})
    print(f"Untranslated Chinese literals: {len(missing)} occurrences / {len(unique)} distinct")
    if unique:
        print("  (COMMENT ON object descriptions are data, not UI text; see contract section 5)")
        for value in unique[:20]:
            print("   - " + value[:100].replace("\n", " "))
        if len(unique) > 20:
            print(f"   ... {len(unique) - 20} more")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
