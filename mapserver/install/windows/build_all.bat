@echo off
chcp 65001 >/dev/null
REM Build r-mapserver and r-manager in release mode
REM Usage: build_all.bat [frontend]

setlocal enabledelayedexpansion

echo ============================================
echo Building r-mapserver and r-manager
echo ============================================

REM Check Rust toolchain
where rustc >/dev/null 2>&1
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

REM Build r-mapserver
echo [3/4] Building r-mapserver (release)...
cd /d "%~dp0..\.."
cargo build --release -p gisserver-server
if errorlevel 1 (
    echo [ERROR] r-mapserver build failed
    exit /b 1
)
echo [OK] r-mapserver built: target\release\map-api.exe

REM Build r-manager
echo [4/4] Building r-manager (release)...
cd /d "%~dp0..\..\..\r-manager\backend"
if not exist "Cargo.toml" (
    echo [ERROR] r-manager not found at ..\..\..\r-manager\backend
    exit /b 1
)
cargo build --release
if errorlevel 1 (
    echo [ERROR] r-manager build failed
    exit /b 1
)
echo [OK] r-manager built: target\release\manager.exe

REM Optional: build frontend
if "%1"=="frontend" (
    echo [EXTRA] Building frontend...
    cd /d "%~dp0..\..\..\r-manager\ui"
    if exist "package.json" (
        where pnpm >/dev/null 2>&1 && pnpm install && pnpm build || (
            where npm >/dev/null 2>&1 && npm install && npm run build || (
                echo [WARN] No pnpm/npm found, skip frontend build
            )
        )
    )
)

echo.
echo ============================================
echo Build complete!
echo ============================================
echo r-mapserver: %~dp0..\..\target\release\map-api.exe
echo r-manager:   %~dp0..\..\..\r-manager\backend\target\release\manager.exe
endlocal
