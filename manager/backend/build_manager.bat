@echo off
REM rust-gisserver manager backend build script
REM Usage: build_manager.bat [debug|release]
REM
REM Environment policy: when cl.exe is already on PATH (VS Developer Prompt / preconfigured env),
REM keep it untouched; otherwise fall back to the registered VS/Windows SDK paths below.
REM Set PATH/LIB/INCLUDE manually instead of calling vcvars64.bat, which probes the SDK
REM via reg.exe and is blocked in restricted environments. Paths stay script-relative.
setlocal

set "MSVC=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
set "SDK=C:\Program Files (x86)\Windows Kits\10"
set "SDKVER=10.0.26100.0"

where cl.exe >nul 2>nul
if errorlevel 1 (
  set "PATH=%MSVC%\bin\HostX64\x64;%SDK%\bin\%SDKVER%\x64;%USERPROFILE%\.cargo\bin;%PATH%"
  set "LIB=%MSVC%\lib\x64;%SDK%\Lib\%SDKVER%\um\x64;%SDK%\Lib\%SDKVER%\ucrt\x64"
  set "INCLUDE=%MSVC%\include;%SDK%\Include\%SDKVER%\ucrt;%SDK%\Include\%SDKVER%\um;%SDK%\Include\%SDKVER%\shared;%SDK%\Include\%SDKVER%\winrt"
  echo [BUILD] cl.exe not in PATH, applied bundled MSVC/SDK env
) else (
  echo [BUILD] using cl.exe already on PATH
)

set "MODE=%~1"
if "%MODE%"=="" set "MODE=release"

pushd "%~dp0"

if /i "%MODE%"=="release" (
  cargo build --release
) else (
  cargo build
)
set "RC=%ERRORLEVEL%"

popd
echo [BUILD] exit=%RC%
exit /b %RC%
