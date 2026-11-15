# Replace the installed map-api.exe with a fresh release build and restart the 8080 service
$install = "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver-install"
$src     = "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\target\release\map-api.exe"
$dst     = "$install\map-api\map-api.exe"

# 1) Stop the old process holding port 8080
$conn = Get-NetTCPConnection -LocalPort 8080 -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
if ($conn) {
  Stop-Process -Id $conn.OwningProcess -Force
  Start-Sleep -Seconds 2
  Write-Host "killed old pid=$($conn.OwningProcess)"
}

# 2) Replace the exe
Copy-Item $src $dst -Force
Write-Host "installed: $dst  $((Get-Item $dst).LastWriteTime)"

# 3) Start it the same way as the install dir's start_mapapi.bat (that bat ends with pause, so do not call it)
Push-Location $install
if (Test-Path "$install\.env") {
  Get-Content "$install\.env" | ForEach-Object {
    if ($_ -match '^([^#][^=]+)=(.*)$') { Set-Item "env:$($Matches[1].Trim())" $Matches[2].Trim() }
  }
}
# data_root defaults to the data\ subdirectory of the working directory.
# Do not inject DATA_ROOT here; set it only for an external data drive.
$env:PROJ_DLL  = "$install\map-api\share\proj_9.dll"
$env:PROJ_DATA = "$install\map-api\share\proj"
$p = Start-Process -FilePath "$install\map-api\map-api.exe" `
     -ArgumentList "serve","--config","config\mapserver\mapserver.xml" `
     -WorkingDirectory $install -WindowStyle Hidden `
     -RedirectStandardOutput "$install\logs\mapapi_out.log" `
     -RedirectStandardError  "$install\logs\mapapi_err.log" -PassThru
Pop-Location
Write-Host "started pid=$($p.Id)"

# 4) Poll port 8080 until ready
$ok = $false
foreach ($i in 1..30) {
  Start-Sleep -Seconds 1
  try { (Invoke-WebRequest -Uri "http://localhost:8080/health" -UseBasicParsing -TimeoutSec 2) | Out-Null; $ok = $true; break } catch {}
}
Write-Host "port8080_up=$ok"
