@echo off
cd /d "%~dp0"
if exist .env ( for /f "usebackq tokens=1,* delims== eol=#" %%a in (".env") do set "%%a=%%b" )
set "PROJ_LIB="
set "PROJ_DLL=%~dp0bin\proj_9.dll"
set "PROJ_DATA=%~dp0share\proj"
echo Starting manager on port 6180...
bin\manager.exe
