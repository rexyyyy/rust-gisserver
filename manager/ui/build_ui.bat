@echo off
REM rust-gisserver manager UI build script
REM Usage: build_ui.bat [dev|prod]
REM
REM Paths resolve from the script location; prefer node/pnpm on PATH, otherwise fall back to
REM the registered local node directory below (edit this one spot when changing machines).
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
