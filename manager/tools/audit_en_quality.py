#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""audit_en_quality.py -- scan manager/backend .rs for Chinese literals
and report which are NOT covered by locales/en_US.json (exact or prefix).

Usage:
  python audit_en_quality.py [--json] [--rs PATH] [--locale PATH]
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path

CS_RE = re.compile(r'[\u4e00-\u9fa5]')
STR_RE = re.compile(r'"([^"\\]*(?:\\.[^"\\]*)*)"')
FN_RE = re.compile(r'\btr\s*\(\s*"')


def extract_chinese_strings(src: str):
    """Yield (literal, line_no) pairs for every double-quoted string containing
    Chinese characters, skipping ones that live inside tr("...") calls since
    those are intentionally translation sources."""
    results = []
    lines = src.splitlines()
    in_tr = False
    for i, line in enumerate(lines, 1):
        # naive: detect start of tr(" call on this line
        if FN_RE.search(line):
            in_tr = True
        # find all string literals with Chinese
        for m in STR_RE.finditer(line):
            s = m.group(1)
            if CS_RE.search(s):
                # skip strings inside tr("...") -- they are catalog sources
                if in_tr and m.start() > line.find('tr("'):
                    continue
                # also skip if this line is a locale catalog source
                # (contains "操作成功": ... style)
                if ':' in s and len(s) < 40:
                    # might be a comment or label, still capture but tag later
                    pass
                results.append((s, i))
        # reset tr state at line end (tr call rarely spans lines in this codebase)
        if 'tr(' not in line:
            in_tr = False
    return results


def is_covered(literal: str, exact: dict, prefixes: list):
    if literal in exact:
        return True, 'exact'
    for p, _ in prefixes:
        if literal.startswith(p):
            return True, f'prefix:{p}'
    return False, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--rs', default=None, help='backend src dir')
    ap.add_argument('--locale', default=None, help='en_US.json path')
    ap.add_argument('--json', action='store_true', help='JSON output')
    args = ap.parse_args()

    here = Path(__file__).resolve().parent.parent
    rs_dir = Path(args.rs) if args.rs else here / 'backend' / 'src'
    locale_path = Path(args.locale) if args.locale else here / 'backend' / 'locales' / 'en_US.json'

    data = json.loads(locale_path.read_text(encoding='utf-8'))
    exact = data.get('exact', {})
    prefixes = data.get('prefix', [])
    print(f'[audit] exact={len(exact)} prefix={len(prefixes)}', file=sys.stderr)

    uncovered = []
    covered_total = 0
    total = 0

    for rs_file in sorted(rs_dir.rglob('*.rs')):
        text = rs_file.read_text(encoding='utf-8', errors='replace')
        for literal, lineno in extract_chinese_strings(text):
            total += 1
            ok, how = is_covered(literal, exact, prefixes)
            rel = rs_file.relative_to(here)
            if ok:
                covered_total += 1
            else:
                uncovered.append({
                    'file': str(rel).replace('\\', '/'),
                    'line': lineno,
                    'literal': literal,
                })

    coverage = covered_total / total * 100 if total else 0

    if args.json:
        json.dump({
            'total_literals': total,
            'covered': covered_total,
            'uncovered_count': len(uncovered),
            'coverage_pct': round(coverage, 1),
            'uncovered': uncovered,
        }, sys.stdout, ensure_ascii=False, indent=2)
    else:
        print(f'\n=== Chinese literal coverage report ===')
        print(f'Total Chinese literals : {total}')
        print(f'Covered (exact/prefix) : {covered_total}')
        print(f'UNCOVERED              : {len(uncovered)} ({coverage:.1f}% coverage)')
        print()
        if uncovered:
            print('--- UNCOVERED LITERALS (need en_US.json entry) ---')
            for u in uncovered:
                print(f'  {u["file"]}:{u["line"]}  "{u["literal"]}"')
        else:
            print('All Chinese literals are covered!')


if __name__ == '__main__':
    main()
