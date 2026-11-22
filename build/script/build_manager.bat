@echo off
chcp 65001 >nul
REM Build manager backend only, release by default.
REM CARGO_TARGET_DIR = <repo>\build\target
REM Usage: build_manager.bat [debug|release]

REM NO setlocal - call vcvars + setlocal = env lost

set "CARGO_TARGET_DIR=%~dp0..\target"
if not exist "%CARGO_TARGET_DIR%" mkdir "%CARGO_TARGET_DIR%"

REM ---- MSVC ----
set "MSVC_BAT="
if exist "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" set "MSVC_BAT=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
if "%MSVC_BAT%"=="" if exist "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat" set "MSVC_BAT=C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat"
if "%MSVC_BAT%"=="" if exist "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat" set "MSVC_BAT=C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat"
if "%MSVC_BAT%"=="" goto :skip_msvc
call "%MSVC_BAT%"
set "CARGO_TARGET_DIR=%~dp0..\target"

:skip_msvc
where rustc >nul 2>&1
if errorlevel 1 (
    echo [ERROR] rustc not found
    exit /b 1
)

set "MODE=%~1"
if "%MODE%"=="" set "MODE=release"

cd /d "%~dp0..\..\manager\backend"
if not exist Cargo.toml (
    echo [ERROR] manager backend not found
    exit /b 1
)

echo ============================================
echo Building manager (%MODE%)
echo CARGO_TARGET_DIR=%CARGO_TARGET_DIR%
echo ============================================

if /i "%MODE%"=="debug" goto :dbg
cargo build --release
goto :chk
:dbg
cargo build

:chk
if errorlevel 1 (
    echo [FAIL] exit code %ERRORLEVEL%
    exit /b %ERRORLEVEL%
)

for %%F in ("%CARGO_TARGET_DIR%\release\manager.exe") do echo [OK] manager.exe - %%~zF bytes
exit /b 0
