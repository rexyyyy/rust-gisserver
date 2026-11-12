import subprocess
out = subprocess.run(['tasklist', '/FO', 'CSV'], capture_output=True, text=True, encoding='gbk', errors='replace').stdout
for l in out.splitlines():
    if l.lower().startswith('"map-api.exe'):
        pid = l.split('","')[1]
        subprocess.run(['taskkill', '/F', '/PID', pid], capture_output=True)
        print(f"killed map-api pid={pid}")
with open(r"D:\mycomputer\documents\gis-wt\thirdparty\rust-gisserver\kill_out.txt", "w") as f:
    f.write("done")
