$wd = "d:\mycomputer\documents\gis-wt\thirdparty\run-env"
Get-Process manager, map-api -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800
$env:MANAGER_DB_PASSWORD = "123456"
$env:MANAGER_DB_HOST = "localhost"
$env:MANAGER_DB_PORT = "5434"
$env:MANAGER_JWT_SECRET = "dev-secret-key"
$env:PROJ_LIB = ""
$env:PROJ_DLL = "$wd\bin\proj_9.dll"
$env:PROJ_DATA = "$wd\share\proj"

$m = Start-Process -FilePath "$wd\bin\manager.exe" -ArgumentList "serve","--config","config\manager\manager.xml" -WorkingDirectory $wd -WindowStyle Hidden -PassThru
Start-Sleep -Milliseconds 400
$s = Start-Process -FilePath "$wd\bin\mapserve.exe" -ArgumentList "serve","--config","config\mapserver.xml" -WorkingDirectory $wd -WindowStyle Hidden -PassThru
Write-Host "manager=$($m.Id) mapserver=$($s.Id)"
exit 0
