# 重编 map-api（release），产物落在仓库 target\release，供复制到 install 目录替换。
# 环境设置与 build_mapapi.ps1 一致，但 CARGO_TARGET_DIR 用仓库默认（安装版 exe 来源口径）。
$msvc  = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
$sdk   = "C:\Program Files (x86)\Windows Kits\10"
$sdkv  = "10.0.26100.0"
$cargo = "C:\Users\kmcd\.cargo\bin"

$env:PATH     = "$msvc\bin\Hostx64\x64;$sdk\bin\$sdkv\x64;$cargo;" + $env:PATH
$env:LIB      = "$msvc\lib\x64;$sdk\Lib\$sdkv\um\x64;$sdk\Lib\$sdkv\ucrt\x64;$sdk\Lib\$sdkv\shared\x64"
$env:INCLUDE  = "$msvc\include;$sdk\Include\$sdkv\um;$sdk\Include\$sdkv\ucrt;$sdk\Include\$sdkv\shared;$sdk\Include\$sdkv\winrt"

Set-Location "D:\mycomputor\documents\gis-wt\thirdparty\r-mapserver"
Write-Host "=== cargo build -p gisserver-server --release ==="
& "$cargo\cargo.exe" build -p gisserver-server --release *> "target\build_release_cesium_fix.log"
$code = $LASTEXITCODE
Write-Host "cargo exit code = $code"
Get-Content "target\build_release_cesium_fix.log" -Tail 8
exit $code
