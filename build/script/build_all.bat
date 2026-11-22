@echo off
chcp 65001 >nul
REM Build rust-gisserver (mapserver + manager), release mode.
REM CARGO_TARGET_DIR = <repo>\build\target  (SHARED between all Rust crates)
REM Usage: build_all.bat [frontend]

REM IMPORTANT: do NOT use setlocal here!
REM call vcvars64.bat + setlocal = env vars lost when vcvars exits inside setlocal.

REM ---- canonical output dir ----
set "CARGO_TARGET_DIR=%~dp0..\target"
if not exist "%CARGO_TARGET_DIR%" mkdir "%CARGO_TARGET_DIR%"

echo ============================================
echo Build rust-gisserver
echo CARGO_TARGET_DIR=%CARGO_TARGET_DIR%
echo ============================================

where rustc >nul 2>&1
if errorlevel 1 (
    echo [ERROR] rustc not found
    exit /b 1
)
rustc --version
cargo --version

REM ---- MSVC (linear, NO paren blocks) ----
set "MSVC_BAT="
if exist "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" set "MSVC_BAT=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
if "%MSVC_BAT%"=="" if exist "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat" set "MSVC_BAT=C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat"
if "%MSVC_BAT%"=="" if exist "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat" set "MSVC_BAT=C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat"
if "%MSVC_BAT%"=="" goto :skip_msvc
echo [MSVC] %MSVC_BAT%
call "%MSVC_BAT%"

REM call vcvars may reset vars - re-apply CARGO_TARGET_DIR
set "CARGO_TARGET_DIR=%~dp0..\target"
goto :after_msvc

:skip_msvc
echo [WARN] MSVC not found, use default toolchain

:after_msvc
REM ---- build mapserver ----
echo.
echo [1/2] mapserver (release)...
cd /d "%~dp0..\..\mapserver"
if not exist Cargo.toml (
    echo [ERROR] mapserver not found
    exit /b 1
)
echo   cargo target: %CARGO_TARGET_DIR%
cargo build --release -p r-mapserver
if errorlevel 1 (
    echo [ERROR] mapserver build failed
    exit /b 1
)
for %%F in ("%CARGO_TARGET_DIR%\release\mapserve.exe") do echo [OK] mapserve.exe - %%~zF bytes

REM ---- build manager ----
echo.
echo [2/2] manager (release)...
cd /d "%~dp0..\..\manager\backend"
if not exist Cargo.toml (
    echo [ERROR] manager backend not found
    exit /b 1
)
echo   cargo target: %CARGO_TARGET_DIR%
cargo build --release
if errorlevel 1 (
    echo [ERROR] manager build failed
    exit /b 1
)
for %%F in ("%CARGO_TARGET_DIR%\release\manager.exe") do echo [OK] manager.exe - %%~zF bytes

REM ---- optional frontend ----
if not "%1"=="frontend" goto :done
echo.
echo [EXTRA] frontend...
cd /d "%~dp0..\..\manager\ui"
if not exist package.json goto :done
where pnpm >nul 2>&1
if errorlevel 1 goto :try_npm
echo [UI] pnpm
call pnpm install
if errorlevel 1 goto :done
call pnpm run build:prod
goto :done

:try_npm
where npm >nul 2>&1
if errorlevel 1 goto :skip_ui
echo [UI] npm
call npm install
if errorlevel 1 goto :done
call npm run build
goto :done

:skip_ui
echo [WARN] no pnpm/npm, skip frontend

:done
echo.
echo ============================================
echo Build done!
echo ============================================
echo   %CARGO_TARGET_DIR%\release\mapserve.exe
echo   %CARGO_TARGET_DIR%\release\manager.exe
cd /d "%~dp0"
