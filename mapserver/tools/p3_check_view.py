import psycopg2, traceback
out = []
try:
    conn = psycopg2.connect("postgres://postgres:123456@localhost:5434/gis")
    conn.autocommit = True
    cur = conn.cursor()
    cur.execute("SELECT to_regclass('public.v_wuhua_tall')")
    out.append(f"regclass: {cur.fetchone()[0]}")
    cur.execute("SELECT count(*) FROM public.v_wuhua_tall")
    out.append(f"rows: {cur.fetchone()[0]}")
    cur.execute("SELECT column_name, data_type FROM information_schema.columns WHERE table_name='v_wuhua_tall' ORDER BY ordinal_position")
    out.append(f"cols: {cur.fetchall()}")
    conn.close()
    out.append("OK")
except Exception:
    out.append(traceback.format_exc())
with open(r"D:\mycomputer\documents\gis-wt\thirdparty\rust-gisserver\setup_view_result.txt", "w", encoding="utf-8") as f:
    f.write("\n".join(out))
