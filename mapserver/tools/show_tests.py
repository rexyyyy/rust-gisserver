"""Summarize test results from E:\\tmp\\cargo.log."""
from pathlib import Path

log = Path(r"E:\tmp\cargo.log")
text = log.read_text(encoding="utf-8", errors="replace")
results = [l.strip() for l in text.splitlines() if l.strip().startswith("test result")]
failed = [l for l in text.splitlines() if "FAILED" in l or "panicked" in l]
print(f"suites: {len(results)}, all-ok: {all('0 failed' in r for r in results)}")
for r in results:
    print(" ", r)
if failed:
    print("FAILURES:")
    for f in failed:
        print(" ", f.strip()[:160])
