$wd = "d:\mycomputer\documents\gis-wt\thirdparty\run-env"
$env:MANAGER_DB_PASSWORD = "123456"
$env:MANAGER_DB_HOST = "localhost"
$env:MANAGER_DB_PORT = "5434"
$env:MANAGER_JWT_SECRET = "dev-secret-key-for-local-testing-2026"
$env:PROJ_LIB = ""
$env:PROJ_DLL = "$wd\bin\proj_9.dll"
$env:PROJ_DATA = "$wd\share\proj"

Get-Process manager -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

$proc = Start-Process -FilePath "$wd\bin\manager.exe" `
  -ArgumentList "serve","--config","config\manager\manager.xml" `
  -WorkingDirectory $wd `
  -WindowStyle Hidden `
  -PassThru
Write-Host "MANAGER_PID=$($proc.Id)"
exit 0
