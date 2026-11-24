import subprocess, sys
py = sys.executable
try:
    import psycopg2  # noqa
    print("psycopg2 already ok")
except ImportError:
    subprocess.run([py, "-m", "pip", "install", "psycopg2-binary", "--quiet"], check=True)
    print("installed")

import psycopg2
conn = psycopg2.connect("postgres://postgres:123456@localhost:5432/gis")
conn.autocommit = True
cur = conn.cursor()
cur.execute("""
CREATE OR REPLACE VIEW public.v_wuhua_tall AS
SELECT gid, height, geom
FROM public.wuhua_buildings
WHERE height >= 10
""")
cur.execute("SELECT count(*) FROM public.v_wuhua_tall")
n = cur.fetchone()[0]
cur.execute("SELECT column_name, data_type FROM information_schema.columns "
            "WHERE table_name='v_wuhua_tall' ORDER BY ordinal_position")
cols = cur.fetchall()
print(f"view rows={n} cols={cols}")
conn.close()
print("DB-OK")
