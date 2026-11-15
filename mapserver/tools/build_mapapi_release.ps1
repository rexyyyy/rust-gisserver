# Rebuild map-api (release); output goes to target\release for copying into the install dir.
# Same env setup as build_mapapi.ps1, but CARGO_TARGET_DIR stays at the repo default (source of installed exe).
$msvc  = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
$sdk   = "C:\Program Files (x86)\Windows Kits\10"
$sdkv  = "10.0.26100.0"
$cargo = "C:\Users\kmcd\.cargo\bin"

$env:PATH     = "$msvc\bin\Hostx64\x64;$sdk\bin\$sdkv\x64;$cargo;" + $env:PATH
$env:LIB      = "$msvc\lib\x64;$sdk\Lib\$sdkv\um\x64;$sdk\Lib\$sdkv\ucrt\x64;$sdk\Lib\$sdkv\shared\x64"
$env:INCLUDE  = "$msvc\include;$sdk\Include\$sdkv\um;$sdk\Include\$sdkv\ucrt;$sdk\Include\$sdkv\shared;$sdk\Include\$sdkv\winrt"

Set-Location "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver"
Write-Host "=== cargo build -p mapserver --release ==="
& "$cargo\cargo.exe" build -p mapserver --release *> "target\build_release_cesium_fix.log"
$code = $LASTEXITCODE
Write-Host "cargo exit code = $code"
Get-Content "target\build_release_cesium_fix.log" -Tail 8
exit $code
