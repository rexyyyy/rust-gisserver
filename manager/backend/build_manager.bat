@echo off
REM rust-gisserver manager backend build script
REM Usage: build_manager.bat [debug|release]
REM
REM 环境策略：PATH 里已有 cl.exe（说明已在 VS Developer Prompt / 已配置好的环境里）
REM 就直接使用，不覆盖；否则回退到下方登记的 VS/Windows SDK 路径（可按本机调整），
REM 手工设置 PATH/LIB/INCLUDE 而不调 vcvars64.bat —— 后者依赖 reg.exe 探测 SDK
REM 路径，在受限环境里会被拦。目录一律按脚本自身位置解析，不绑定绝对路径。
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
