# Rebuild map-api (release); output goes to target\release for copying into the install dir.
# Same env setup as build_mapapi.ps1, but CARGO_TARGET_DIR stays at the repo default (source of installed exe).
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

Set-Location (Resolve-Path "$PSScriptRoot\..").Path
Write-Host "=== cargo build -p r-mapserver --release ==="
& "$cargo\cargo.exe" build -p r-mapserver --release *> "target\build_release_cesium_fix.log"
$code = $LASTEXITCODE
Write-Host "cargo exit code = $code"
Get-Content "target\build_release_cesium_fix.log" -Tail 8
exit $code
