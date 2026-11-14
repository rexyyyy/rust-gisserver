@echo off
chcp 65001 >/dev/null
REM Uninstall rust-gisserver
REM Usage: uninstall.bat [install_dir]

setlocal enabledelayedexpansion

echo ============================================
echo Uninstalling rust-gisserver
echo ============================================

REM Script lives at <repo>\build\script\, two levels up is repo root, run-env sits there.
set INSTALL_DIR=%~dp0..\..\run-env
if not "%1"=="" set INSTALL_DIR=%1

echo Install directory: %INSTALL_DIR%

if not exist "%INSTALL_DIR%" (
    echo [ERROR] Directory not found: %INSTALL_DIR%
    exit /b 1
)

REM Stop running services
echo [1/3] Stopping services...
taskkill /F /IM map-api.exe 2>nul
taskkill /F /IM manager.exe 2>nul

REM Ask for confirmation
echo.
set /p CONFIRM="This will delete %INSTALL_DIR%. Continue? (y/N): "
if /i not "%CONFIRM%"=="y" (
    echo Aborted
    exit /b 0
)

REM Remove directory
echo [2/3] Removing installation...
rmdir /S /Q "%INSTALL_DIR%"
if errorlevel 1 (
    echo [ERROR] Failed to remove directory
    exit /b 1
)

echo [3/3] Done
echo.
echo ============================================
echo Uninstall complete
echo ============================================
endlocal
