# -*- coding: utf-8 -*-
"""
Regenerate the four installation manual PDFs from the markdown sources next to
this script. Requires only Chrome (headless) + Python stdlib.

Usage:
    python gen_manuals.py

Sources (authoritative) -> outputs:
    01-PostgreSQL-PostGIS-zh-CN.md -> 01-PostgreSQL-PostGIS-zh-CN.pdf
    02-PostgreSQL-PostGIS-en.md    -> 02-PostgreSQL-PostGIS-en.pdf
    03-rust-gisserver-Setup-zh-CN.md -> 03-rust-gisserver-Setup-zh-CN.pdf
    04-rust-gisserver-Setup-en.md    -> 04-rust-gisserver-Setup-en.pdf
"""
import html
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CHROME_CANDIDATES = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
]

CSS = """
@page { size: A4; margin: 18mm 16mm 18mm 16mm; }
body { font-family: "Microsoft YaHei", "Segoe UI", Arial, sans-serif;
       font-size: 11px; line-height: 1.65; color: #1a1a1a; }
h1 { font-size: 22px; color: #0b3d6e; margin: 0 0 6px 0; border-bottom: 3px solid #0b3d6e; padding-bottom: 8px; }
h2 { font-size: 16px; color: #0b3d6e; margin: 22px 0 8px 0; border-left: 5px solid #2e75b6; padding-left: 9px; }
h3 { font-size: 13px; color: #1f4e79; margin: 14px 0 6px 0; }
p { margin: 6px 0; }
ul, ol { margin: 6px 0 6px 22px; padding: 0; }
li { margin: 3px 0; }
code { font-family: Consolas, "Courier New", monospace; background: #f2f4f7;
       padding: 1px 4px; border-radius: 3px; font-size: 10.5px; color: #b02a37; }
pre { background: #f6f8fa; border: 1px solid #d9dee4; border-left: 4px solid #2e75b6;
      padding: 10px 12px; border-radius: 4px; overflow-wrap: anywhere; white-space: pre-wrap;
      font-size: 10px; line-height: 1.5; margin: 8px 0; }
pre code { background: none; padding: 0; color: #24292f; }
table { border-collapse: collapse; width: 100%; margin: 10px 0; font-size: 10px; }
th { background: #d9e2f3; color: #14375e; font-weight: 700; }
th, td { border: 1px solid #9fb3c8; padding: 5px 7px; vertical-align: middle; text-align: left; }
tr:nth-child(even) td { background: #f7f9fc; }
blockquote { margin: 8px 0; padding: 7px 11px; background: #fff8e6;
             border-left: 4px solid #e0a800; color: #5c4600; }
.meta { color: #5a6b7c; font-size: 10px; margin-bottom: 16px; }
.pagebreak { page-break-before: always; }
"""


def inline(text):
    text = html.escape(text, quote=False)
    text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    return text


def md_to_html(md):
    out, i = [], 0
    lines = md.splitlines()
    while i < len(lines):
        line = lines[i]
        s = line.strip()
        if not s:
            i += 1
            continue
        if s == "---":
            out.append('<div class="pagebreak"></div>')
            i += 1
            continue
        if s.startswith("```"):
            i += 1
            buf = []
            while i < len(lines) and not lines[i].strip().startswith("```"):
                buf.append(lines[i])
                i += 1
            i += 1
            out.append("<pre><code>" + html.escape("\n".join(buf), quote=False) + "</code></pre>")
            continue
        m = re.match(r"^(#{1,3})\s+(.*)$", s)
        if m:
            lvl = len(m.group(1))
            out.append("<h%d>%s</h%d>" % (lvl, inline(m.group(2)), lvl))
            i += 1
            continue
        if s.startswith("|") and i + 1 < len(lines) and re.match(r"^\s*\|?[\s:|-]+\|", lines[i + 1]):
            header = [c.strip() for c in s.strip("|").split("|")]
            i += 2
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")])
                i += 1
            t = ["<table><thead><tr>" + "".join("<th>%s</th>" % inline(c) for c in header) + "</tr></thead><tbody>"]
            for r in rows:
                t.append("<tr>" + "".join("<td>%s</td>" % inline(c) for c in r) + "</tr>")
            t.append("</tbody></table>")
            out.append("".join(t))
            continue
        if s.startswith("> "):
            buf = []
            while i < len(lines) and lines[i].strip().startswith(">"):
                buf.append(lines[i].strip().lstrip(">").strip())
                i += 1
            out.append("<blockquote>" + inline(" ".join(buf)) + "</blockquote>")
            continue
        if re.match(r"^[-*]\s+", s):
            buf = []
            while i < len(lines) and re.match(r"^\s*[-*]\s+", lines[i]):
                buf.append(re.sub(r"^\s*[-*]\s+", "", lines[i]))
                i += 1
            out.append("<ul>" + "".join("<li>%s</li>" % inline(x) for x in buf) + "</ul>")
            continue
        if re.match(r"^\d+\.\s+", s):
            buf = []
            while i < len(lines) and re.match(r"^\s*\d+\.\s+", lines[i]):
                buf.append(re.sub(r"^\s*\d+\.\s+", "", lines[i]))
                i += 1
            out.append("<ol>" + "".join("<li>%s</li>" % inline(x) for x in buf) + "</ol>")
            continue
        out.append("<p>%s</p>" % inline(s))
        i += 1
    return "<!DOCTYPE html><html><head><meta charset='utf-8'><style>%s</style></head><body>%s</body></html>" % (CSS, "\n".join(out))


def find_chrome():
    for p in CHROME_CANDIDATES:
        if os.path.isfile(p):
            return p
    raise RuntimeError("Chrome/Edge not found")


def main():
    chrome = find_chrome()
    pairs = [
        ("01-PostgreSQL-PostGIS-zh-CN",),
        ("02-PostgreSQL-PostGIS-en",),
        ("03-rust-gisserver-Setup-zh-CN",),
        ("04-rust-gisserver-Setup-en",),
    ]
    for (stem,) in pairs:
        md_path = os.path.join(HERE, stem + ".md")
        pdf_path = os.path.join(HERE, stem + ".pdf")
        html_path = os.path.join(HERE, stem + ".html")
        if not os.path.isfile(md_path):
            print("SKIP (no source): " + md_path)
            continue
        with open(md_path, "r", encoding="utf-8") as f:
            doc = f.read()
        with open(html_path, "w", encoding="utf-8") as f:
            f.write(md_to_html(doc))
        if os.path.exists(pdf_path):
            os.remove(pdf_path)
        r = subprocess.run(
            [chrome, "--headless", "--disable-gpu", "--no-sandbox", "--no-pdf-header-footer",
             "--print-to-pdf=" + pdf_path, html_path],
            capture_output=True, text=True, timeout=120,
        )
        if not os.path.isfile(pdf_path):
            print("FAIL " + stem + ": " + r.stderr[-400:])
            sys.exit(1)
        print("OK %s -> %d bytes" % (stem, os.path.getsize(pdf_path)))
        os.remove(html_path)


if __name__ == "__main__":
    main()
