@echo off
REM Install r-mapserver and r-manager
REM Usage: install.bat [install_dir]

setlocal enabledelayedexpansion

echo ============================================
echo Installing r-mapserver and r-manager
echo ============================================

REM Default install directory
set INSTALL_DIR=%~dp0..\..\..\..\rmap_target
if not "%1"=="" set INSTALL_DIR=%1

echo Install directory: %INSTALL_DIR%

REM Check if executables exist
set MAPSERVER_EXE=%~dp0..\..\target\release\map-api.exe
set MANAGER_EXE=%~dp0..\..\..\r-manager\backend\target\release\manager.exe

if not exist "%MAPSERVER_EXE%" (
    echo [ERROR] r-mapserver not found: %MAPSERVER_EXE%
    echo Please run build_all.bat first
    exit /b 1
)

if not exist "%MANAGER_EXE%" (
    echo [ERROR] r-manager not found: %MANAGER_EXE%
    echo Please run build_all.bat first
    exit /b 1
)

REM Create directory structure
echo [1/6] Creating directory structure...
if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
if not exist "%INSTALL_DIR%\bin" mkdir "%INSTALL_DIR%\bin"
if not exist "%INSTALL_DIR%\config" mkdir "%INSTALL_DIR%\config"
if not exist "%INSTALL_DIR%\data" mkdir "%INSTALL_DIR%\data"
if not exist "%INSTALL_DIR%\cache" mkdir "%INSTALL_DIR%\cache"
if not exist "%INSTALL_DIR%\logs" mkdir "%INSTALL_DIR%\logs"
if not exist "%INSTALL_DIR%\share" mkdir "%INSTALL_DIR%\share"
if not exist "%INSTALL_DIR%\share\proj" mkdir "%INSTALL_DIR%\share\proj"

REM Copy executables
echo [2/6] Copying executables...
copy /Y "%MAPSERVER_EXE%" "%INSTALL_DIR%\bin\map-api.exe"
copy /Y "%MANAGER_EXE%" "%INSTALL_DIR%\bin\manager.exe"

REM Copy PROJ DLL and data
REM Layout: proj_9.dll and its 12 transitive deps go into bin\ (same dir as map-api.exe) because
REM LoadLibraryExW(LOAD_WITH_ALTERED_SEARCH_PATH) resolves proj_9.dll's imports only from
REM proj_9.dll's own directory. proj.db goes to share\proj (referenced by PROJ_DATA).
echo [3/6] Copying PROJ library...
set PROJ_DLL=%~dp0..\..\proj_9.dll
if exist "%PROJ_DLL%" (
    copy /Y "%PROJ_DLL%" "%INSTALL_DIR%\bin\proj_9.dll"
) else (
    echo [WARN] proj_9.dll not found
)

REM proj_9.dll transitive dependencies (must sit next to proj_9.dll).
REM Each dependency is searched independently across the candidate directories, so a partially
REM populated first candidate does not shadow the later ones. This keeps the script working from
REM a source checkout as well as from a staged tree that already carries the closure.
set PROJ_DEPS=
for %%P in (
    "%~dp0..\..\bin"
    "%~dp0..\..\..\..\build"
    "%~dp0..\..\..\..\thirdparty\GDAL3.13\dll"
) do if exist "%%~P" set "PROJ_DEPS=!PROJ_DEPS!;%%~P"
for %%D in (sqlite3 libcurl tiff zlib libssh2 brotlidec brotlicommon jpeg62 libcrypto-3-x64 MSVCP140 VCRUNTIME140 VCRUNTIME140_1) do (
    set "FOUND="
    for %%P in (%PROJ_DEPS%) do (
        if not defined FOUND if exist "%%~P\%%D.dll" (
            copy /Y "%%~P\%%D.dll" "%INSTALL_DIR%\bin\%%D.dll" >nul
            set "FOUND=1"
        )
    )
    if not defined FOUND echo [WARN] proj dependency missing: %%D.dll
)

set PROJ_DATA=%~dp0..\..\share\proj
if exist "%PROJ_DATA%" (
    xcopy /E /I /Y "%PROJ_DATA%" "%INSTALL_DIR%\share\proj"
) else (
    echo [WARN] share\proj not found
)

REM Copy configs
echo [4/6] Copying configs...
set MAPSERVER_CONFIG=%~dp0..\..\config
if exist "%MAPSERVER_CONFIG%" (
    xcopy /E /I /Y "%MAPSERVER_CONFIG%" "%INSTALL_DIR%\config"
)

set MANAGER_CONFIG=%~dp0..\..\..\r-manager\backend\config
if exist "%MANAGER_CONFIG%" (
    xcopy /E /I /Y "%MANAGER_CONFIG%" "%INSTALL_DIR%\config\manager"
    REM Copy manager.xml to config root for default path
    copy /Y "%INSTALL_DIR%\config\manager\manager.xml" "%INSTALL_DIR%\config\manager.xml"
    REM Fix static_dir path for installed layout.
    REM Source config may carry either "../ui/dist" or "manager/ui/dist"
    REM (the latter is what r-manager/backend/config/manager.xml actually ships),
    REM so both patterns must be rewritten to the installed "web" directory.
    powershell -Command "(Get-Content '%INSTALL_DIR%\config\manager.xml') -replace 'static_dir=\"(\.\./ui/dist|manager/ui/dist)\"', 'static_dir=\"web\"' | Set-Content '%INSTALL_DIR%\config\manager.xml'"
    powershell -Command "(Get-Content '%INSTALL_DIR%\config\manager\manager.xml') -replace 'static_dir=\"(\.\./ui/dist|manager/ui/dist)\"', 'static_dir=\"web\"' | Set-Content '%INSTALL_DIR%\config\manager\manager.xml'"
)

REM Copy frontend if built
echo [5/6] Copying frontend...
set FRONTEND_DIR=%~dp0..\..\..\r-manager\ui\dist
if exist "%FRONTEND_DIR%" (
    xcopy /E /I /Y "%FRONTEND_DIR%" "%INSTALL_DIR%\web"
    echo [OK] Frontend copied
) else (
    echo [WARN] Frontend not built
)

REM Generate .env.example
echo [6/6] Generating .env.example...
echo # r-mapserver and r-manager environment configuration > "%INSTALL_DIR%\.env.example"
echo # Copy this file to .env and modify as needed >> "%INSTALL_DIR%\.env.example"
echo. >> "%INSTALL_DIR%\.env.example"
echo # Legacy filename encoding region for path normalisation >> "%INSTALL_DIR%\.env.example"
echo # Valid values: zh-CN / zh-TW / en / ko / ja >> "%INSTALL_DIR%\.env.example"
echo # Linux and macOS have no system ANSI code page, set this explicitly >> "%INSTALL_DIR%\.env.example"
echo GIS_ENCODING_REGION=zh-CN >> "%INSTALL_DIR%\.env.example"
echo. >> "%INSTALL_DIR%\.env.example"
echo # PostgreSQL database for r-manager >> "%INSTALL_DIR%\.env.example"
echo MANAGER_DB_HOST=localhost >> "%INSTALL_DIR%\.env.example"
echo MANAGER_DB_PORT=5434 >> "%INSTALL_DIR%\.env.example"
echo MANAGER_DB_PASSWORD=your_password_here >> "%INSTALL_DIR%\.env.example"
echo MANAGER_JWT_SECRET=your_jwt_secret_here >> "%INSTALL_DIR%\.env.example"
echo. >> "%INSTALL_DIR%\.env.example"
echo # r-mapserver connection (for r-manager proxy) >> "%INSTALL_DIR%\.env.example"
echo MAPSRV_HOST=127.0.0.1 >> "%INSTALL_DIR%\.env.example"
echo MAPSRV_PORT=8080 >> "%INSTALL_DIR%\.env.example"
echo. >> "%INSTALL_DIR%\.env.example"
echo # r-mapserver >> "%INSTALL_DIR%\.env.example"
echo MAPSERVER_PORT=8080 >> "%INSTALL_DIR%\.env.example"
echo MAPSERVER_DATA_DIR=data >> "%INSTALL_DIR%\.env.example"
echo MAPSERVER_CACHE_DIR=cache >> "%INSTALL_DIR%\.env.example"
echo. >> "%INSTALL_DIR%\.env.example"
echo # API Key (optional, for auth) >> "%INSTALL_DIR%\.env.example"
echo API_KEY=your-secret-key-here >> "%INSTALL_DIR%\.env.example"

REM Copy per-region .env presets. Pick one and copy to .env if needed.
if exist "%~dp0..\..\config\env\.env.zh-CN" if not exist "%INSTALL_DIR%\env" mkdir "%INSTALL_DIR%\env"
if exist "%~dp0..\..\config\env\.env.zh-CN" copy /Y "%~dp0..\..\config\env\.env.*" "%INSTALL_DIR%\env\" >nul
if exist "%~dp0..\..\config\env\README.md" copy /Y "%~dp0..\..\config\env\README.md" "%INSTALL_DIR%\env\" >nul

REM Generate startup scripts
echo Generating startup scripts...
REM PROJ_LIB is cleared on purpose: the machine-wide value may point at a PostGIS-shipped proj.db
REM with an incompatible layout version, which aborts PROJ with
REM "DATABASE.LAYOUT.VERSION.MINOR = 2 whereas a number >= 6 is expected".
REM PROJ_DATA points at the bundled proj.db, which matches the shipped PROJ version.
echo @echo off > "%INSTALL_DIR%\start-mapserver.bat"
echo cd /d "%%~dp0" >> "%INSTALL_DIR%\start-mapserver.bat"
echo if exist .env ( for /f "usebackq tokens=1,* delims== eol=#" %%%%a in (".env") do set "%%%%a=%%%%b" ) >> "%INSTALL_DIR%\start-mapserver.bat"
echo set "PROJ_LIB=" >> "%INSTALL_DIR%\start-mapserver.bat"
echo set "PROJ_DLL=%%~dp0bin\proj_9.dll" >> "%INSTALL_DIR%\start-mapserver.bat"
echo set "PROJ_DATA=%%~dp0share\proj" >> "%INSTALL_DIR%\start-mapserver.bat"
echo echo Starting r-mapserver on port 8080... >> "%INSTALL_DIR%\start-mapserver.bat"
echo bin\map-api.exe serve >> "%INSTALL_DIR%\start-mapserver.bat"

echo @echo off > "%INSTALL_DIR%\start-manager.bat"
echo cd /d "%%~dp0" >> "%INSTALL_DIR%\start-manager.bat"
echo if exist .env ( for /f "usebackq tokens=1,* delims== eol=#" %%%%a in (".env") do set "%%%%a=%%%%b" ) >> "%INSTALL_DIR%\start-manager.bat"
REM PROJ_LIB is cleared on purpose, for the same reason as in start-mapserver.bat: the
REM machine-wide value may point at a PostGIS-shipped proj.db whose layout version is
REM older than the bundled PROJ expects, which aborts PROJ initialization with
REM "DATABASE.LAYOUT.VERSION.MINOR = 2 whereas a number >= 5 is expected".
REM manager.exe does not link PROJ itself (it proxies map-api over HTTP), but clearing
REM PROJ_LIB keeps the manager process from inheriting that machine-wide proj.db, and
REM PROJ_DLL/PROJ_DATA point any PROJ-aware helper at the same bundled runtime that
REM map-api uses.
echo set "PROJ_LIB=" >> "%INSTALL_DIR%\start-manager.bat"
echo set "PROJ_DLL=%%~dp0bin\proj_9.dll" >> "%INSTALL_DIR%\start-manager.bat"
echo set "PROJ_DATA=%%~dp0share\proj" >> "%INSTALL_DIR%\start-manager.bat"
echo echo Starting r-manager on port 6180... >> "%INSTALL_DIR%\start-manager.bat"
echo bin\manager.exe >> "%INSTALL_DIR%\start-manager.bat"

echo.
echo ============================================
echo Installation complete!
echo ============================================
echo Directory: %INSTALL_DIR%
echo.
echo Next steps:
echo   1. Copy .env.example to .env and configure
echo   2. Run start-mapserver.bat (port 8080)
echo   3. Run start-manager.bat (port 3000)
echo.
echo See README.md for detailed instructions
endlocal
