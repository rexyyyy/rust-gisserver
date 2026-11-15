# ============================================================================
# P0-4 one-shot start + probe -- deploy/start.ps1
#
# Read .env -> docker compose up (default --build) -> wait for PostGIS healthy -> run validate.ps1.
#
# Usage:
#   pwsh -File start.ps1              # build, start, then self-check
#   pwsh -File start.ps1 -NoBuild     # start without rebuilding images
#   pwsh -File start.ps1 -SkipValidate
# ============================================================================
param(
    [switch]$NoBuild,
    [switch]$SkipValidate
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# Load .env into the process environment (shared by compose and local validate checks)
$envFile = Join-Path $PSScriptRoot '.env'
if (-not (Test-Path $envFile)) {
    throw '缺少 .env：请先 copy .env.example .env 并填写 MAPSERVER_DB_PASSWORD / MANAGER_JWT_SECRET'
}
Get-Content -LiteralPath $envFile | ForEach-Object {
    if ($_ -match '^\s*([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2].Trim(), 'Process')
    }
}

# Start the stack
$up = @('compose', 'up', '-d')
if (-not $NoBuild) { $up += '--build' }
Write-Output ('docker ' + ($up -join ' '))
docker @up

# Wait for PostGIS healthy (up to 3 minutes)
Write-Output '等待 PostGIS 健康…'
$deadline = (Get-Date).AddMinutes(3)
$healthy = $false
do {
    Start-Sleep -Seconds 3
    $ps = docker compose ps --format '{{.Name}} {{.State}} {{.Health}}' 2>$null
    if ($ps -match 'gis-postgis.*running.*healthy') { $healthy = $true; break }
} while ((Get-Date) -lt $deadline)
if (-not $healthy) { Write-Warning 'PostGIS 未在 3 分钟内转 healthy，继续尝试自检' }

# Self-check
if (-not $SkipValidate) {
    $dbPort = if ($env:MAPSERVER_DB_PORT) { [int]$env:MAPSERVER_DB_PORT } else { 5434 }
    & (Join-Path $PSScriptRoot 'validate.ps1') -DbPort $dbPort -CheckDataRoot
}

Write-Output ''
Write-Output '前端    : http://localhost:6180/'
Write-Output 'map-api : http://localhost:8080/health'
Write-Output '停止    : docker compose down   （删卷加 -v）'
