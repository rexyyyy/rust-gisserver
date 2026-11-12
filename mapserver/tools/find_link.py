#!/usr/bin/env python3
"""Locate link.exe (MSVC) and vcvars so cargo can link from a bare subprocess."""
import os
import subprocess
from pathlib import Path

VSWHERE = Path(
    r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
)


def find_vs_install():
    if not VSWHERE.is_file():
        return None
    out = subprocess.run(
        [str(VSWHERE), "-latest", "-products", "*",
         "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
         "-property", "installationPath"],
        capture_output=True, text=True, timeout=30,
    ).stdout.strip()
    return out.splitlines()[0] if out else None


def find_link():
    # 1) already on PATH?
    for d in os.environ.get("PATH", "").split(os.pathsep):
        if d and (Path(d) / "link.exe").is_file():
            return d
    # 2) scan MSVC tools dirs under the VS install
    root = find_vs_install()
    if not root:
        return None
    msvc = Path(root) / "VC" / "Tools" / "MSVC"
    if not msvc.is_dir():
        return None
    for ver in sorted(msvc.iterdir(), reverse=True):
        cand = ver / "bin" / "Hostx64" / "x64"
        if (cand / "link.exe").is_file():
            return str(cand)
    return None


if __name__ == "__main__":
    d = find_link()
    print(d or "NOT FOUND")
