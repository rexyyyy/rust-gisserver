# Rebuild rust-gisserver map-api (debug); set MSVC + Windows SDK env first, then call cargo.
# Usage: powershell -ExecutionPolicy Bypass -File build_mapapi.ps1
$sdk   = "C:\Program Files (x86)\Windows Kits\10"
$sdkv  = "10.0.26100.0"
$cargo = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $env:USERPROFILE '.cargo\bin' }

# Locate the newest MSVC toolset through vswhere; leave it unset if VS is absent.
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$msvc = $null
if (Test-Path $vswhere) {
  $vsRoot = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  if ($vsRoot) {
    $msvc = Get-ChildItem (Join-Path $vsRoot "VC\Tools\MSVC") -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^\d+(\.\d+)+$' } |
            Sort-Object { [version]$_.Name } -Descending |
            Select-Object -First 1 -ExpandProperty FullName
  }
}
if (-not $msvc) { Write-Warning "MSVC toolset not found via vswhere; continuing without MSVC paths" }

$sdkBin = "$sdk\bin\$sdkv\x64"
$sdkLib = "$sdk\Lib\$sdkv\um\x64;$sdk\Lib\$sdkv\ucrt\x64;$sdk\Lib\$sdkv\shared\x64"
$sdkInc = "$sdk\Include\$sdkv\um;$sdk\Include\$sdkv\ucrt;$sdk\Include\$sdkv\shared;$sdk\Include\$sdkv\winrt"

if ($msvc) {
  $env:PATH    = "$msvc\bin\Hostx64\x64;$sdkBin;$cargo;" + $env:PATH
  $env:LIB     = "$msvc\lib\x64;$sdkLib"
  $env:INCLUDE = "$msvc\include;$sdkInc"
} else {
  $env:PATH    = "$sdkBin;$cargo;" + $env:PATH
  $env:LIB     = $sdkLib
  $env:INCLUDE = $sdkInc
}

# Keep an externally provided target dir; otherwise use the repo default.
if (-not $env:CARGO_TARGET_DIR) {
  $env:CARGO_TARGET_DIR = Join-Path (Resolve-Path "$PSScriptRoot\..").Path "target"
}

Set-Location (Resolve-Path "$PSScriptRoot\..").Path
$testLog  = Join-Path $env:CARGO_TARGET_DIR "test_ogcapi_tiles.log"
$libLog   = Join-Path $env:CARGO_TARGET_DIR "test_map_api.log"
$buildLog = Join-Path $env:CARGO_TARGET_DIR "build_mapapi.log"

Write-Host "=== cargo test -p r-mapserver --lib (resolution 口径守卫) ==="
& "$cargo\cargo.exe" test -p r-mapserver --lib *>"$testLog"
$testCode = $LASTEXITCODE
Write-Host "test exit code = $testCode"
Get-Content "$testLog" -Tail 12
if ($testCode -ne 0) { exit $testCode }

Write-Host "=== cargo test -p r-mapserver --lib (含 apikeys 缓存守卫) ==="
& "$cargo\cargo.exe" test -p r-mapserver --lib *>"$libLog"
$libCode = $LASTEXITCODE
Write-Host "lib test exit code = $libCode"
Get-Content "$libLog" -Tail 12
if ($libCode -ne 0) { exit $libCode }

Write-Host "=== cargo build -p r-mapserver (debug) ==="
& "$cargo\cargo.exe" build -p r-mapserver *>"$buildLog"
$code = $LASTEXITCODE
Write-Host "cargo exit code = $code"
Get-Content "$buildLog" -Tail 8
exit $code
