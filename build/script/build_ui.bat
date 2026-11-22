@echo off
chcp 65001 >nul
REM Build manager UI (Vue 3 + pnpm), prod by default.
REM Output: manager\ui\dist\  (install.bat copies to run-env\web)
REM Usage: build_ui.bat [dev|prod]

setlocal

where pnpm >nul 2>&1
if not errorlevel 1 goto :pnpm_ok
where npm >nul 2>&1
if not errorlevel 1 goto :npm_ok
echo [ERROR] neither pnpm nor npm found on PATH
echo         Install Node.js 18+ then: npm install -g pnpm
exit /b 1

:pnpm_ok
echo [UI] using pnpm
goto :ui_build

:npm_ok
echo [UI] using npm (no pnpm found)
set "PKG=npm"
goto :ui_build

:ui_build
set "MODE=%~1"
if "%MODE%"=="" set "MODE=prod"

pushd "%~dp0..\..\manager\ui"

echo ============================================
echo Building manager UI (%MODE%)
echo ============================================

if /i "%MODE%"=="prod" goto :prod
call pnpm run build:dev
set "RC=%ERRORLEVEL%"
goto :ui_done

:prod
call pnpm run build:prod
set "RC=%ERRORLEVEL%"

:ui_done
popd

if not "%RC%"=="0" (
    echo [FAIL] UI build exited with code %RC%
    exit /b %RC%
)
echo [OK] UI dist at manager\ui\dist\
exit /b 0
