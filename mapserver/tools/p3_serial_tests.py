import subprocess, time, os
env = dict(os.environ)
vs = r"C:/Program Files/Microsoft Visual Studio/2022/Community"
m = "14.44.35207"; s = "10.0.26100.0"
env['PATH'] = vs + r"/VC/Tools/MSVC/%s/bin/Hostx64/x64;C:/Program Files (x86)/Windows Kits/10/bin/%s/x64;" % (m, s) + env['PATH']
env['LIB'] = vs + r"/VC/Tools/MSVC/%s/lib/x64;C:/Program Files (x86)/Windows Kits/10/Lib/%s/um/x64;C:/Program Files (x86)/Windows Kits/10/Lib/%s/ucrt/x64" % (m, s, s)
env['INCLUDE'] = vs + r"/VC/Tools/MSVC/%s/include;C:/Program Files (x86)/Windows Kits/10/Include/%s/um;C:/Program Files (x86)/Windows Kits/10/Include/%s/ucrt;C:/Program Files (x86)/Windows Kits/10/Include/%s/shared" % (m, s, s, s)

def as_text(x):
    if x is None: return ""
    return x.decode("utf-8", "replace") if isinstance(x, bytes) else x

crates = ["pg-source", "wms-common", "wms-protocol", "wfs-protocol", "vector-query",
          "ogcapi-features", "ogcapi-tiles", "projection", "storage", "cog", "renderer",
          "pmtiles", "map-api"]
lines = []
for crate in crates:
    cmd = [r"C:/Users/Administrator/.cargo/bin/cargo.exe", "test", "-p", crate,
           "--manifest-path", r"D:\mycomputer\documents\gis-wt\thirdparty\r-mapserver\Cargo.toml"]
    t0 = time.time()
    try:
        p = subprocess.run(cmd, capture_output=True, text=False, timeout=600,
                           cwd=r"D:\mycomputer\documents\gis-wt\thirdparty\r-mapserver", env=env)
        rc, out = p.returncode, as_text(p.stdout) + as_text(p.stderr)
    except subprocess.TimeoutExpired as e:
        rc = -99
        out = as_text(e.stdout) + as_text(e.stderr)
    tails = [l for l in out.strip().splitlines() if l.strip()][-8:]
    lines.append(f"=== {crate} exit={rc} dur={time.time()-t0:.0f}s ===\n" + "\n".join(tails))
    # write incrementally so a later hang does not lose earlier results
    with open(r"D:\mycomputer\documents\gis-wt\thirdparty\r-mapserver\serial_tests.txt", "w", encoding="utf-8", errors="replace") as f:
        f.write("\n\n".join(lines))
