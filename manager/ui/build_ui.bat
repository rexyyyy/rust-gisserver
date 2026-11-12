@echo off
REM r-mapserver manager UI build script
REM Usage: build_ui.bat [dev|prod]
REM
REM 目录按脚本自身位置解析；node/pnpm 优先用 PATH 里的，找不到才回退到
REM 下方登记的工作机 node 目录（换机器时改这一处即可）。
setlocal

set "NODE="
where pnpm >nul 2>nul
if errorlevel 1 (
  set "NODE=C:\Users\kmcd\.workbuddy\binaries\node\versions\22.22.2-3"
  set "PATH=%NODE%;%PATH%"
  echo [UI-BUILD] pnpm not in PATH, fell back to bundled node: %NODE%
) else (
  echo [UI-BUILD] using pnpm already on PATH
)

set "MODE=%~1"
if "%MODE%"=="" set "MODE=prod"

pushd "%~dp0"

if /i "%MODE%"=="prod" (
  call pnpm run build:prod
) else (
  call pnpm run build:dev
)
set "RC=%ERRORLEVEL%"

popd
echo [UI-BUILD] exit=%RC%
exit /b %RC%
