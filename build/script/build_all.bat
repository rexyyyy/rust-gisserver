@echo off
chcp 65001 >nul
REM Build rust-gisserver (mapserver + manager) and optional frontend, in release mode.
REM All cargo output is redirected to <repo>\build\target via CARGO_TARGET_DIR,
REM NOT the per-crate target dirs. install.bat reads from there.
REM Usage: build_all.bat [frontend]

setlocal enabledelayedexpansion

REM ---- Canonical build output directory ----
set "CARGO_TARGET_DIR=%~dp0..\target"
if not exist "%CARGO_TARGET_DIR%" mkdir "%CARGO_TARGET_DIR%"

echo ============================================
echo Building rust-gisserver (target: %CARGO_TARGET_DIR%)
echo ============================================

REM Check Rust toolchain
where rustc >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Rust not found. Install from https://rustup.rs
    exit /b 1
)

echo [1/4] Rust version:
rustc --version
cargo --version

REM Load MSVC environment if available
if exist "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" (
    echo [2/4] Loading MSVC 2022 Community...
    call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
) else if exist "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat" (
    echo [2/4] Loading MSVC 2022 Professional...
    call "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat"
) else if exist "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat" (
    echo [2/4] Loading MSVC 2019 Community...
    call "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat"
) else (
    echo [WARN] MSVC not found, using default toolchain
)

REM Ensure Windows SDK import libs / headers are on LIB/INCLUDE.
REM vcvars normally adds these, but on some installs the SDK lib path is missing,
REM which makes link.exe fail with LNK1181 (cannot find kernel32.lib).
REM Fall back to the newest Windows SDK actually present on disk.
set "SDKROOT=C:\Program Files (x86)\Windows Kits\10"
if exist "%SDKROOT%" (
    set "SDKVER="
    for /f "delims=" %%V in ('dir /b /ad /o-n "%SDKROOT%\Lib" 2^>nul') do (
        if not defined SDKVER if exist "%SDKROOT%\Lib\%%V\um\x64\kernel32.lib" set "SDKVER=%%V"
    )
    if defined SDKVER (
        set "LIB=%LIB%;%SDKROOT%\Lib\%SDKVER%\um\x64;%SDKROOT%\Lib\%SDKVER%\ucrt\x64"
        set "INCLUDE=%INCLUDE%;%SDKROOT%\Include\%SDKVER%\um;%SDKROOT%\Include\%SDKVER%\shared;%SDKROOT%\Include\%SDKVER%\ucrt"
        echo [2/4] Windows SDK %SDKVER% lib/include appended
    ) else (
        echo [WARN] Windows SDK lib not found under %SDKROOT%\Lib
    )
)

REM Build mapserver
echo [3/4] Building mapserver (release)...
cd /d "%~dp0..\..\mapserver"
if not exist "Cargo.toml" (
    echo [ERROR] mapserver not found at %~dp0..\..\mapserver
    exit /b 1
)
cargo build --release -p mapserver
if errorlevel 1 (
    echo [ERROR] mapserver build failed
    exit /b 1
)
echo [OK] mapserver built: %CARGO_TARGET_DIR%\release\map-api.exe

REM Build manager
echo [4/4] Building manager (release)...
cd /d "%~dp0..\..\manager\backend"
if not exist "Cargo.toml" (
    echo [ERROR] manager not found at %~dp0..\..\manager\backend
    exit /b 1
)
cargo build --release
if errorlevel 1 (
    echo [ERROR] manager build failed
    exit /b 1
)
echo [OK] manager built: %CARGO_TARGET_DIR%\release\manager.exe

REM Optional: build frontend
if "%1"=="frontend" (
    echo [EXTRA] Building frontend...
    cd /d "%~dp0..\..\manager\ui"
    if exist "package.json" (
        where pnpm >nul 2>&1 && pnpm install && pnpm build || (
            where npm >nul 2>&1 && npm install && npm run build || (
                echo [WARN] No pnpm/npm found, skip frontend build
            )
        )
    )
)

echo.
echo ============================================
echo Build complete!
echo ============================================
echo target dir: %CARGO_TARGET_DIR%
echo   map-api.exe : %CARGO_TARGET_DIR%\release\map-api.exe
echo   manager.exe : %CARGO_TARGET_DIR%\release\manager.exe
endlocal
