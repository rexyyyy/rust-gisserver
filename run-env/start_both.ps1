$wd = "d:\mycomputer\documents\gis-wt\thirdparty\run-env"
$env:MANAGER_DB_PASSWORD = "123456"
$env:MANAGER_DB_HOST = "localhost"
$env:MANAGER_DB_PORT = "5434"
$env:MANAGER_JWT_SECRET = "dev-secret-key-for-local-testing-2026"

Get-Process manager, map-api -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

$mgr = Start-Process -FilePath "$wd\bin\manager.exe" `
  -ArgumentList "serve","--config","config\manager\manager.xml" `
  -WorkingDirectory $wd `
  -WindowStyle Hidden `
  -PassThru
Write-Host "MANAGER_PID=$($mgr.Id)"

Start-Sleep -Milliseconds 500

$mapsvr = Start-Process -FilePath "$wd\bin\mapserve.exe" `
  -ArgumentList "serve","--config","config\mapserver.xml" `
  -WorkingDirectory $wd `
  -WindowStyle Hidden `
  -PassThru
Write-Host "MAPSVR_PID=$($mapsvr.Id)"
exit 0
