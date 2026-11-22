import socket, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
ok = False
for _ in range(60):
    try:
        s = socket.create_connection(("127.0.0.1", 8080), timeout=2)
        s.close()
        ok = True
        break
    except OSError:
        time.sleep(1)
with open(ROOT / "port8080.txt", "w") as f:
    f.write(f"up={ok}")
