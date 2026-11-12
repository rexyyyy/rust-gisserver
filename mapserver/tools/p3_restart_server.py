import subprocess, time, socket, os
ROOT = r"D:\mycomputer\documents\gis-wt\thirdparty\rust-gisserver"

# 1) kill old server
out = subprocess.run(['tasklist', '/FO', 'CSV'], capture_output=True, text=True, encoding='gbk', errors='replace').stdout
for l in out.splitlines():
    if l.lower().startswith('"map-api.exe'):
        pid = l.split('","')[1]
        subprocess.run(['taskkill', '/F', '/PID', pid], capture_output=True)
        print(f"killed old map-api pid={pid}")

# 2) fresh binary?
exe = os.path.join(ROOT, r"target\debug\map-api.exe")
print(f"binary mtime: {time.ctime(os.path.getmtime(exe))}")

# 3) start bat (durable via scheduled task)
bat = os.path.join(ROOT, "start_mapapi.bat")
with open(bat, "w", encoding="ascii") as f:
    f.write("@echo off\r\n")
    f.write(f"cd /d {ROOT}\r\n")
    f.write('"target\\debug\\map-api.exe" > server_out.log 2> server_err.log\r\n')
subprocess.run(["schtasks", "/create", "/tn", "WB_MAPAPI", "/tr", bat, "/sc", "once", "/st", "23:59", "/f"], capture_output=True)
subprocess.run(["schtasks", "/run", "/tn", "WB_MAPAPI"], capture_output=True)

# 4) poll port 8080 (up to 60s)
ok = False
for _ in range(60):
    try:
        s = socket.create_connection(("127.0.0.1", 8080), timeout=2)
        s.close()
        ok = True
        break
    except OSError:
        time.sleep(1)
print(f"port8080_up={ok}")
subprocess.run(["schtasks", "/delete", "/tn", "WB_MAPAPI", "/f"], capture_output=True)
