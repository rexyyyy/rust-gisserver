# Rebuild the rust-gisserver backend (debug) and run unit tests; set MSVC + Windows SDK env first.
# Usage: powershell -ExecutionPolicy Bypass -File tools/build_manager.ps1
# Note: the local Bash shim is incomplete and PowerShell blocks cmd.exe, so .bat files fail;
#       jsonwebtoken->ring still needs MSVC link.exe + kernel32.lib, so LIB/INCLUDE/PATH must be set.
$msvc  = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
$sdk   = "C:\Program Files (x86)\Windows Kits\10"
$sdkv  = "10.0.26100.0"
$cargo = "C:\Users\kmcd\.cargo\bin"
$root  = "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\backend"

$env:PATH     = "$msvc\bin\Hostx64\x64;$sdk\bin\$sdkv\x64;$cargo;" + $env:PATH
$env:LIB      = "$msvc\lib\x64;$sdk\Lib\$sdkv\um\x64;$sdk\Lib\$sdkv\ucrt\x64;$sdk\Lib\$sdkv\shared\x64"
$env:INCLUDE  = "$msvc\include;$sdk\Include\$sdkv\um;$sdk\Include\$sdkv\ucrt;$sdk\Include\$sdkv\shared;$sdk\Include\$sdkv\winrt"

Set-Location $root
Write-Host "=== cargo test (bin crate 内联单测) ==="
& "$cargo\cargo.exe" test *>"$root\target\test_manager.log"
$testCode = $LASTEXITCODE
Write-Host "test exit code = $testCode"
Get-Content "$root\target\test_manager.log" -Tail 14
if ($testCode -ne 0) { exit $testCode }

Write-Host "=== cargo build (debug) ==="
& "$cargo\cargo.exe" build *>"$root\target\build_manager.log"
$code = $LASTEXITCODE
Write-Host "build exit code = $code"
Get-Content "$root\target\build_manager.log" -Tail 10
exit $code
