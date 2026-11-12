# 重编 r-mapserver 的 map-api（debug）。手动设齐 MSVC + Windows SDK 环境后直调 cargo。
# 用法：powershell -ExecutionPolicy Bypass -File build_mapapi.ps1
$msvc  = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
$sdk   = "C:\Program Files (x86)\Windows Kits\10"
$sdkv  = "10.0.26100.0"
$cargo = "C:\Users\kmcd\.cargo\bin"

$env:PATH     = "$msvc\bin\Hostx64\x64;$sdk\bin\$sdkv\x64;$cargo;" + $env:PATH
$env:LIB      = "$msvc\lib\x64;$sdk\Lib\$sdkv\um\x64;$sdk\Lib\$sdkv\ucrt\x64;$sdk\Lib\$sdkv\shared\x64"
$env:INCLUDE  = "$msvc\include;$sdk\Include\$sdkv\um;$sdk\Include\$sdkv\ucrt;$sdk\Include\$sdkv\shared;$sdk\Include\$sdkv\winrt"
$env:CARGO_TARGET_DIR = "E:\rmap_target"

Set-Location "D:\mycomputor\documents\gis-wt\thirdparty\r-mapserver"
Write-Host "=== cargo test -p gisserver-ogcapi-tiles --lib (resolution 口径守卫) ==="
& "$cargo\cargo.exe" test -p gisserver-ogcapi-tiles --lib *>"E:\rmap_target\test_ogcapi_tiles.log"
$testCode = $LASTEXITCODE
Write-Host "test exit code = $testCode"
Get-Content "E:\rmap_target\test_ogcapi_tiles.log" -Tail 12
if ($testCode -ne 0) { exit $testCode }

Write-Host "=== cargo test -p gisserver-server --lib (含 apikeys 缓存守卫) ==="
& "$cargo\cargo.exe" test -p gisserver-server --lib *>"E:\rmap_target\test_map_api.log"
$libCode = $LASTEXITCODE
Write-Host "lib test exit code = $libCode"
Get-Content "E:\rmap_target\test_map_api.log" -Tail 12
if ($libCode -ne 0) { exit $libCode }

Write-Host "=== cargo build -p gisserver-server (debug) ==="
& "$cargo\cargo.exe" build -p gisserver-server *>"E:\rmap_target\build_mapapi.log"
$code = $LASTEXITCODE
Write-Host "cargo exit code = $code"
Get-Content "E:\rmap_target\build_mapapi.log" -Tail 8
exit $code
