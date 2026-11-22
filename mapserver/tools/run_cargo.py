#!/usr/bin/env python3
"""Run an arbitrary cargo command in the rust-gisserver workspace.

Output goes to cargo.log in the system temp directory (full) with the tail
printed to stdout.
Injects the MSVC linker directory into PATH when missing (a bare Python
subprocess does not inherit the VS developer environment).

Usage: python tools/run_cargo.py check --workspace
       python tools/run_cargo.py test --workspace
       python tools/run_cargo.py build --release -p r-mapserver
"""
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOG = Path(tempfile.gettempdir()) / "cargo.log"
VS = Path(r"C:\Program Files\Microsoft Visual Studio\2022\Community")
KITS = Path(r"C:\Program Files (x86)\Windows Kits\10")


def latest_dir(base: Path, prefix: str):
    if not base.is_dir():
        return None
    cands = sorted(
        (d for d in base.iterdir()
         if d.is_dir() and d.name.startswith(prefix)
         and all(p.isdigit() for p in d.name[len(prefix):].split(".") if p)),
        key=lambda d: [int(p) for p in d.name[len(prefix):].split(".") if p.isdigit()],
        reverse=True,
    )
    return cands[0] if cands else None


def msvc_env():
    """INCLUDE/LIB/PATH for MSVC + Windows Kits (vcvars64.bat is unusable in
    this sandbox: it shells out to reg.exe, which the security policy blocks)."""
    msvc = latest_dir(VS / "VC" / "Tools" / "MSVC", "14.")
    inc_ver = latest_dir(KITS / "Include", "10.")
    lib_ver = latest_dir(KITS / "Lib", "10.")
    if not (msvc and inc_ver and lib_ver):
        return {}
    inc = [msvc / "include"] + [
        p for sub in ("ucrt", "um", "shared", "winrt", "cppwinrt")
        if (p := inc_ver / sub).is_dir()
    ]
    lib = [msvc / "lib" / "x64"] + [
        p for sub in ("ucrt", "um")
        if (p := lib_ver / sub / "x64").is_dir()
    ]
    return {
        "INCLUDE": ";".join(str(p) for p in inc),
        "LIB": ";".join(str(p) for p in lib),
        "PATH": os.pathsep.join(
            [str(msvc / "bin" / "HostX64" / "x64"), os.environ.get("PATH", "")]
        ),
    }


def main():
    args = sys.argv[1:]
    if not args:
        args = ["check"]
    env = dict(os.environ)
    env.update(msvc_env())
    cmd = ["cargo"] + args
    proc = subprocess.run(
        cmd, cwd=ROOT, capture_output=True, text=True, env=env,
        encoding="utf-8", errors="replace", timeout=1200,
    )
    out = (proc.stdout or "") + (proc.stderr or "")
    LOG.parent.mkdir(parents=True, exist_ok=True)
    LOG.write_text(out, encoding="utf-8")
    lines = [l for l in out.splitlines() if l.strip()]
    tail = lines[-14:] if proc.returncode != 0 else lines[-4:]
    print("\n".join(tail))
    print(f"exit={proc.returncode} ({' '.join(cmd)})")
    print(f"full log: {LOG}")


if __name__ == "__main__":
    main()
