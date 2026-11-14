# 用新编译的 release map-api.exe 替换安装目录产物并重启 8080 服务
$install = "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver-install"
$src     = "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\target\release\map-api.exe"
$dst     = "$install\map-api\map-api.exe"

# 1) 停掉占用 8080 的旧进程
$conn = Get-NetTCPConnection -LocalPort 8080 -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
if ($conn) {
  Stop-Process -Id $conn.OwningProcess -Force
  Start-Sleep -Seconds 2
  Write-Host "killed old pid=$($conn.OwningProcess)"
}

# 2) 替换 exe
Copy-Item $src $dst -Force
Write-Host "installed: $dst  $((Get-Item $dst).LastWriteTime)"

# 3) 按安装目录 start_mapapi.bat 同口径后台拉起（该 bat 末尾有 pause，不适合直接调）
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

# 4) 轮询 8080 就绪
$ok = $false
foreach ($i in 1..30) {
  Start-Sleep -Seconds 1
  try { (Invoke-WebRequest -Uri "http://localhost:8080/health" -UseBasicParsing -TimeoutSec 2) | Out-Null; $ok = $true; break } catch {}
}
Write-Host "port8080_up=$ok"
