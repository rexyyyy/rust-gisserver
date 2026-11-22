import subprocess
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
out = subprocess.run(['tasklist', '/FO', 'CSV'], capture_output=True, text=True, encoding='gbk', errors='replace').stdout
for l in out.splitlines():
    if l.lower().startswith('"map-api.exe'):
        pid = l.split('","')[1]
        subprocess.run(['taskkill', '/F', '/PID', pid], capture_output=True)
        print(f"killed map-api pid={pid}")
with open(ROOT / "kill_out.txt", "w") as f:
    f.write("done")
