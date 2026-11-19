@echo off
chcp 65001 >nul
REM Install rust-gisserver (mapserver + manager + frontend + PROJ + configs)
REM Usage: install.bat [install_dir]

setlocal enabledelayedexpansion

echo ============================================
echo Installing rust-gisserver
echo ============================================

REM ---- install dir ----
set "INSTALL_DIR=%~dp0..\..\run-env"
if not "%1"=="" set "INSTALL_DIR=%~1"
echo Install directory: %INSTALL_DIR%

REM ---- source exes ----
set "MAPSERVER_EXE=%~dp0..\target\release\map-api.exe"
set "MANAGER_EXE=%~dp0..\target\release\manager.exe"

if not exist "%MAPSERVER_EXE%" (
    echo [ERROR] map-api.exe not found: %MAPSERVER_EXE%
    echo         Run build_all.bat first
    exit /b 1
)
if not exist "%MANAGER_EXE%" (
    echo [ERROR] manager.exe not found: %MANAGER_EXE%
    echo         Run build_all.bat first
    exit /b 1
)

REM ---- 1/6 dirs ----
echo [1/6] Creating directory structure...
if not exist "%INSTALL_DIR%"        mkdir "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%\bin"     mkdir "%INSTALL_DIR%\bin"
if not exist "%INSTALL_DIR%\config"  mkdir "%INSTALL_DIR%\config"
if not exist "%INSTALL_DIR%\data"    mkdir "%INSTALL_DIR%\data"
if not exist "%INSTALL_DIR%\cache"   mkdir "%INSTALL_DIR%\cache"
if not exist "%INSTALL_DIR%\logs"    mkdir "%INSTALL_DIR%\logs"
if not exist "%INSTALL_DIR%\share"   mkdir "%INSTALL_DIR%\share"
if not exist "%INSTALL_DIR%\share\proj" mkdir "%INSTALL_DIR%\share\proj"
if not exist "%INSTALL_DIR%\storage" mkdir "%INSTALL_DIR%\storage"
if not exist "%INSTALL_DIR%\storage\avatar" mkdir "%INSTALL_DIR%\storage\avatar"

REM ---- 2/6 exes ----
echo [2/6] Copying executables...
copy /Y "%MAPSERVER_EXE%" "%INSTALL_DIR%\bin\map-api.exe" >nul
copy /Y "%MANAGER_EXE%" "%INSTALL_DIR%\bin\manager.exe"   >nul

REM ---- 3/6 PROJ libs (linear scan, NO nested parens) ----
echo [3/6] Copying PROJ runtime...
set "PROJ_SRC="
if exist "%~dp0..\..\..\build"                         set "PROJ_SRC=%~dp0..\..\..\build"
if "%PROJ_SRC%"=="" if exist "%~dp0..\..\..\dist"       set "PROJ_SRC=%~dp0..\..\..\dist"
if "%PROJ_SRC%"=="" if exist "%~dp0..\..\..\thirdparty\GDAL3.13\dll" set "PROJ_SRC=%~dp0..\..\..\thirdparty\GDAL3.13\dll"

set "PROJ_DLL_FOUND=0"
for %%D in (proj_9 sqlite3 libcurl tiff zlib libssh2 brotlidec brotlicommon jpeg62 libcrypto-3-x64) do (
    if exist "%PROJ_SRC%\%%D.dll" (
        copy /Y "%PROJ_SRC%\%%D.dll" "%INSTALL_DIR%\bin\%%D.dll" >nul
        if "%%D"=="proj_9" set "PROJ_DLL_FOUND=1"
    ) else (
        echo [WARN] %%D.dll not found at %PROJ_SRC%
    )
)
if "%PROJ_DLL_FOUND%"=="1" (
    echo [OK] proj_9.dll copied
) else (
    echo [WARN] proj_9.dll missing - PROJ coordinate transforms won't work
)

REM proj.db
set "PROJ_DB_SRC="
if exist "%~dp0..\..\..\build\share\proj.db"                    set "PROJ_DB_SRC=%~dp0..\..\..\build\share\proj.db"
if "%PROJ_DB_SRC%"=="" if exist "%~dp0..\..\..\dist\share\proj.db" set "PROJ_DB_SRC=%~dp0..\..\..\dist\share\proj.db"
if "%PROJ_DB_SRC%"=="" if exist "%~dp0..\..\..\thirdparty\GDAL3.13\share\proj.db" set "PROJ_DB_SRC=%~dp0..\..\..\thirdparty\GDAL3.13\share\proj.db"
if not "%PROJ_DB_SRC%"=="" (
    copy /Y "%PROJ_DB_SRC%" "%INSTALL_DIR%\share\proj\proj.db" >nul
    echo [OK] proj.db copied
) else (
    echo [WARN] proj.db not found
)

REM ---- 4/6 configs ----
echo [4/6] Copying configs...
if exist "%~dp0..\..\mapserver\config" (
    xcopy /E /I /Y "%~dp0..\..\mapserver\config" "%INSTALL_DIR%\config" >nul
)
if exist "%~dp0..\..\manager\backend\config" (
    xcopy /E /I /Y "%~dp0..\..\manager\backend\config" "%INSTALL_DIR%\config\manager" >nul
    copy /Y "%INSTALL_DIR%\config\manager\manager.xml" "%INSTALL_DIR%\config\manager.xml" >nul
    REM rewrite static_dir="..." to "web" for installed layout
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0fix_staticdir.ps1" "%INSTALL_DIR%\config\manager.xml" "%INSTALL_DIR%\config\manager\manager.xml"
)

REM ---- 5/6 frontend ----
echo [5/6] Copying frontend...
if exist "%~dp0..\..\manager\ui\dist" (
    xcopy /E /I /Y "%~dp0..\..\manager\ui\dist" "%INSTALL_DIR%\web" >nul
    echo [OK] Frontend copied
) else (
    echo [WARN] Frontend not built - run build_ui.bat first
)

REM ---- 6/6 .env.example ----
REM Keep in sync with mapserver/config/env/.env.* presets and README section 3.3.
REM REQUIRED = the placeholder has no ":-default" in the config XML, so an
REM unset value expands to an empty string and the service cannot start.
echo [6/6] Generating .env.example...
(
echo # rust-gisserver runtime environment
echo # Copy to .env and replace every REQUIRED value before starting.
echo # start-mapserver.bat / start-manager.bat load .env into the process env.
echo.
echo # ==== REQUIRED: no default in config XML ====
echo # manager DSN password  ^(config/manager/manager.xml^)
echo MANAGER_DB_PASSWORD=change-me
echo # manager JWT signing secret  ^(generate: openssl rand -hex 32^)
echo MANAGER_JWT_SECRET=change-me-random-secret
echo # manager DSN encryption key ^(optional^): derived from JWT secret when empty;
echo #   rotate with care - old ciphertext becomes undecryptable; switch dsn_cipher=des/sm4 in config/manager.xml
echo # MANAGER_DSN_KEY=change-me-random-32-bytes-key
echo # map-api admin DSN password  ^(config/mapserver.xml^)
echo MAPSERVER_DB_PASSWORD=change-me
echo.
echo # ==== OPTIONAL: config XML supplies these defaults ====
echo MANAGER_DB_HOST=localhost
echo MANAGER_DB_PORT=5434
echo MAPSERVER_DB_HOST=localhost
echo MAPSERVER_DB_PORT=5434
echo MAPSRV_HOST=127.0.0.1
echo MAPSRV_PORT=8080
echo MAPSERVER_MANAGER_URL=http://127.0.0.1:6180
echo # Data root defaults to the data\ subdirectory next to start-*.bat
echo # ^(already created by install.bat^). Set DATA_ROOT only when the COG /
echo # vector / mbtiles / pmtiles / 3dtiles data lives on another drive.
echo # DATA_ROOT=E:\data\1
echo.
echo # Legacy filename encoding region: zh-CN / zh-TW / en / ko / ja
echo GIS_ENCODING_REGION=zh-CN
echo # e2e conversion fixture root ^(demo layers only^)
echo # TESTDATA_ROOT=
echo # Absolute path to shp2pgsql.exe, only needed for SHP import
echo # SHPPGSQL_BIN=
echo # PROJ_DLL / PROJ_DATA / PROJ_LIB are set by start-*.bat; do not set here.
echo # Deprecated, no longer read: MAPSERVER_PORT ^(use config/mapserver.xml^),
echo #   MAPSERVER_DATA_DIR, MAPSERVER_CACHE_DIR, API_KEY. Data root defaults
echo #   to data\ and can be overridden with DATA_ROOT.
) > "%INSTALL_DIR%\.env.example"

REM ---- per-region env presets ----
if exist "%~dp0..\..\mapserver\config\env" (
    if not exist "%INSTALL_DIR%\env" mkdir "%INSTALL_DIR%\env"
    copy /Y "%~dp0..\..\mapserver\config\env\*" "%INSTALL_DIR%\env\" >nul
)

REM ---- generate start scripts ----
REM Switch to ANSI codepage 936 before writing .bat files, so that any
REM content (ASCII or future Chinese) lands as GBK/ANSI on disk and is
REM interpreted correctly when the user double-clicks it (default 936).
REM install.bat itself uses chcp 65001 above for the .env.example heredoc.
echo Generating startup scripts...

chcp 936 >nul

(
echo @echo off
echo cd /d "%%~dp0"
echo if exist .env ^( for /f "usebackq tokens=1,* delims== eol=#" %%%%a in ^(".env"^) do set "%%%%a=%%%%b" ^)
echo set "PROJ_LIB="
echo set "PROJ_DLL=%%~dp0bin\proj_9.dll"
echo set "PROJ_DATA=%%~dp0share\proj"
echo echo Starting map-api on port 8080...
echo bin\map-api.exe serve
) > "%INSTALL_DIR%\start-mapserver.bat"

(
echo @echo off
echo cd /d "%%~dp0"
echo if exist .env ^( for /f "usebackq tokens=1,* delims== eol=#" %%%%a in ^(".env"^) do set "%%%%a=%%%%b" ^)
echo set "PROJ_LIB="
echo set "PROJ_DLL=%%~dp0bin\proj_9.dll"
echo set "PROJ_DATA=%%~dp0share\proj"
echo echo Starting manager on port 6180...
echo bin\manager.exe
) > "%INSTALL_DIR%\start-manager.bat"

chcp 65001 >nul

echo.
echo ============================================
echo Install complete!
echo ============================================
echo   dir: %INSTALL_DIR%
echo.
echo   1. Copy .env.example to .env and configure
echo   2. start-mapserver.bat ^(port 8080^)
echo   3. start-manager.bat    ^(port 6180^)
endlocal
