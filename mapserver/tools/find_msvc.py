"""Probe MSVC + Windows Kits paths and inject INCLUDE/LIB into the cargo env."""
import os
import sys
from pathlib import Path

VS = Path(r"C:\Program Files\Microsoft Visual Studio\2022\Community")
KITS = Path(r"C:\Program Files (x86)\Windows Kits\10")


def latest_dir(base: Path, prefix: str) -> Path | None:
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


def probe():
    msvc = latest_dir(VS / "VC" / "Tools" / "MSVC", "14.")
    if msvc is None:
        print("MSVC tools not found")
        return None
    inc_ver = latest_dir(KITS / "Include", "10.")
    lib_ver = latest_dir(KITS / "Lib", "10.")
    if inc_ver is None or lib_ver is None:
        print("Windows Kits 10 not found")
        return None
    inc = [msvc / "include"]
    for sub in ("ucrt", "um", "shared", "winrt", "cppwinrt"):
        p = inc_ver / sub
        if p.is_dir():
            inc.append(p)
    lib = [msvc / "lib" / "x64"]
    for sub in ("ucrt", "um"):
        p = lib_ver / sub / "x64"
        if p.is_dir():
            lib.append(p)
    env = {
        "INCLUDE": ";".join(str(p) for p in inc),
        "LIB": ";".join(str(p) for p in lib),
        "PATH": os.pathsep.join([
            str(msvc / "bin" / "HostX64" / "x64"),
            os.environ.get("PATH", ""),
        ]),
    }
    print("MSVC:", msvc)
    print("KITS:", inc_ver)
    return env


if __name__ == "__main__":
    e = probe()
    if e:
        for k, v in e.items():
            print(f"{k}={v[:200]}...")
