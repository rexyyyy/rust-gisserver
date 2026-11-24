# ====================================================================
#  rust-gisserver database bootstrap (Windows, one-shot)
#  Usage (works from ANY working directory):
#    powershell -ExecutionPolicy Bypass -File init_all.ps1 -Password "123456"
#    powershell -ExecutionPolicy Bypass -File init_all.ps1 -Port 5432 -Password "123456"
#  Runs 01-init-database.sql, system.sql, map.sql in order.
#  Stops on the FIRST error (ON_ERROR_STOP=1 + $LASTEXITCODE check) - no false success.
# ====================================================================
param(
    [int]    $Port     = 5432,
    [string] $DBUser   = "postgres",
    [string] $DBName   = "mapserver-manager",
    [string] $Password = ""
)

$ErrorActionPreference = "Stop"

function Write-Step([string]$text) { Write-Host "`n=== $text ===" -ForegroundColor Cyan }
function Write-Fail([string]$text) { Write-Host "`n[FAIL] $text" -ForegroundColor Red; exit 1 }

# ---- locate psql.exe: PATH -> registry -> common drive locations ----
function Find-Psql {
    $cmd = Get-Command psql.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }

    $reg = "HKLM:\SOFTWARE\PostgreSQL\Installations"
    if (Test-Path $reg) {
        foreach ($key in (Get-ChildItem $reg -ErrorAction SilentlyContinue)) {
            $base = (Get-ItemProperty $key.PSPath -ErrorAction SilentlyContinue).'Base Directory'
            if ($base) {
                $candidate = Join-Path $base 'bin\psql.exe'
                if (Test-Path $candidate) { return $candidate }
            }
        }
    }

    foreach ($drive in (Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
        foreach ($rel in @('PostgreSQL', 'Program Files\PostgreSQL')) {
            $root = Join-Path $drive.Root $rel
            if (-not (Test-Path $root)) { continue }
            $found = Get-ChildItem -LiteralPath $root -Recurse -Filter 'psql.exe' -ErrorAction SilentlyContinue |
                     Select-Object -First 1 -ExpandProperty FullName
            if ($found) { return $found }
        }
    }
    return $null
}

$psql = Find-Psql
if (-not $psql) { Write-Fail "psql.exe not found. Install PostgreSQL 16 or add its bin directory to PATH." }
Write-Host "psql: $psql"

# ---- password ----
if ([string]::IsNullOrEmpty($Password)) {
    $secure = Read-Host "Password for PostgreSQL user '$DBUser'" -AsSecureString
    $Password = [Runtime.InteropServices.Marshal]::PtrToStringUni(
        [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
}
$env:PGPASSWORD = $Password

# ---- SQL files are resolved next to this script, not the caller's cwd ----
$sqlDir = $PSScriptRoot
$steps = @(
    @{ Name = '[1/3] Create database + enable PostGIS'; File = '01-init-database.sql'; Db = '' },
    @{ Name = '[2/3] System tables';                    File = 'system.sql';           Db = $DBName },
    @{ Name = '[3/3] Map tables';                       File = 'map.sql';              Db = $DBName }
)

foreach ($step in $steps) {
    $file = Join-Path $sqlDir $step.File
    if (-not (Test-Path $file)) { Write-Fail "SQL file not found: $file" }
    Write-Step $step.Name
    $psqlArgs = @('-X', '-v', 'ON_ERROR_STOP=1', '-U', $DBUser, '-p', "$Port")
    if ($step.Db) { $psqlArgs += @('-d', $step.Db) }
    $psqlArgs += @('-f', $file)
    & $psql @psqlArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "$($step.File) failed (psql exit code $LASTEXITCODE). All three scripts are idempotent, so fix the cause and re-run this script."
    }
}

# ---- verification ----
Write-Step 'Verify'
$verifySql = "SELECT extname, extversion FROM pg_extension WHERE extname = 'postgis'; SELECT count(*) AS base_tables FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE';"
& $psql -X -U $DBUser -p $Port -d $DBName -c $verifySql
if ($LASTEXITCODE -ne 0) { Write-Fail 'Verification query failed.' }

Write-Host "`n[DONE] Database initialized." -ForegroundColor Green
Write-Host "  DB: $DBName   Port: $Port   User: $DBUser"
Write-Host "  Next: set MANAGER_DB_PASSWORD / MAPSERVER_DB_PASSWORD in run-env\.env to the same password, then start the services."
