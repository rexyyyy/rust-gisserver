@echo off
chcp 65001 >/dev/null
REM Build rust-gisserver and rust-gisserver in release mode
REM Usage: build_all.bat [frontend]

setlocal enabledelayedexpansion

echo ============================================
echo Building rust-gisserver and rust-gisserver
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

REM Build rust-gisserver
echo [3/4] Building rust-gisserver (release)...
cd /d "%~dp0..\.."
cargo build --release -p mapserver
if errorlevel 1 (
    echo [ERROR] rust-gisserver build failed
    exit /b 1
)
echo [OK] rust-gisserver built: target\release\map-api.exe

REM Build rust-gisserver
echo [4/4] Building rust-gisserver (release)...
cd /d "%~dp0..\..\..\manager\backend"
if not exist "Cargo.toml" (
    echo [ERROR] manager not found at ..\..\..\manager\backend
    exit /b 1
)
cargo build --release
if errorlevel 1 (
    echo [ERROR] rust-gisserver build failed
    exit /b 1
)
echo [OK] rust-gisserver built: target\release\manager.exe

REM Optional: build frontend
if "%1"=="frontend" (
    echo [EXTRA] Building frontend...
    cd /d "%~dp0..\..\..\manager\ui"
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
echo rust-gisserver: %~dp0..\..\target\release\map-api.exe
echo manager:   %~dp0..\..\..\manager\target\release\manager.exe
endlocal
