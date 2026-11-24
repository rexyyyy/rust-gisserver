-- ====================================================================
--  rust-gisserver database bootstrap - step 1 (Windows-safe)
--  Usage: psql -U postgres -p 5432 -f 01-init-database.sql
--  Password can be supplied via PGPASSWORD to avoid interactive prompts.
-- ====================================================================

-- 1. Create database.
--    Windows PostgreSQL does not recognize POSIX locales such as 'en_US.UTF-8';
--    specifying LC_COLLATE/LC_CTYPE that way fails with "locale does not exist".
--    TEMPLATE template0 + ENCODING 'UTF8' inherits the server's install-time
--    locale while guaranteeing UTF-8 storage, and works on every platform.
SELECT 'CREATE DATABASE "mapserver-manager" ENCODING ''UTF8'' TEMPLATE template0'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'mapserver-manager')
\gexec

-- 2. Connect to the new database.
\connect "mapserver-manager"

-- 3. Enable PostGIS.
--    Only core PostGIS is used by the code base: ST_Transform, ST_AsMVTGeom,
--    ST_TileEnvelope, ST_Intersects, geometry_columns, and friends.
--    postgis_topology / postgis_sfcgal / postgis_tiger_geocoder /
--    fuzzystrmatch are NOT used and are a frequent cause of install failures
--    (tiger_geocoder in particular requires a superuser).
CREATE EXTENSION IF NOT EXISTS postgis;

-- 4. Verify: expect one row with a version such as 3.5.2.
SELECT extname, extversion FROM pg_extension WHERE extname = 'postgis';
SELECT postgis_full_version();
