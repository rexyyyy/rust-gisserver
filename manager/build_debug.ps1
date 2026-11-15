$ErrorActionPreference = 'Continue'
# Quick regression build: use MSVC on PATH if present, otherwise inject the registered VS/SDK env.
# Paths and logs resolve from the script location; no absolute paths or drive letters.
$vs  = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
$sdk = "C:\Program Files (x86)\Windows Kits\10"
$sdkver = "10.0.26100.0"

if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    $env:PATH = "$vs\bin\Hostx64\x64;$sdk\bin\$sdkver\x64;$env:USERPROFILE\.cargo\bin;" + $env:PATH
    $env:LIB = "$vs\lib\x64;$sdk\Lib\$sdkver\um\x64;$sdk\Lib\$sdkver\ucrt\x64;$sdk\Lib\$sdkver\shared\x64"
    $env:INCLUDE = "$vs\include;$sdk\Include\$sdkver\um;$sdk\Include\$sdkver\ucrt;$sdk\Include\$sdkver\shared;$sdk\Include\$sdkver\winrt"
    Write-Host "cl.exe not in PATH, applied bundled MSVC/SDK env"
}

Set-Location (Join-Path $PSScriptRoot 'backend')
$log = Join-Path $PSScriptRoot 'build_debug.log'
Write-Host "=== cargo build (manager) started $(Get-Date) ==="
cargo build *> $log
Write-Host "=== cargo build finished $(Get-Date) exit=$LASTEXITCODE (log: $log) ==="
exit $LASTEXITCODE
