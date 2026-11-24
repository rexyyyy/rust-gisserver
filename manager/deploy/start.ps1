# ============================================================================
# P0-4 one-shot start + probe -- deploy/start.ps1
#
# Read .env -> docker compose up (default --build) -> wait for PostGIS healthy -> run validate.ps1.
#
# Compose entrypoints live in the per-language × per-platform deploy dirs
# (../deploy_zh-cn/win, ../deploy_en/win, ...); pass -Dir to target one.
#
# Usage:
#   pwsh -File start.ps1 -Dir ..\deploy_zh-cn\win        # build, start, self-check (zh/win)
#   pwsh -File start.ps1 -Dir ..\deploy_en\win           # same for the English deployment
#   pwsh -File start.ps1 -Dir ..\deploy_zh-cn\win -NoBuild   # start without rebuilding images
#   pwsh -File start.ps1 -Dir ..\deploy_zh-cn\win -SkipValidate
# For Linux hosts use start.sh (same -Dir/--dir semantics, bash).
# ============================================================================
param(
    [switch]$NoBuild,
    [switch]$SkipValidate,
    [string]$Dir = ''
)
$ErrorActionPreference = 'Stop'
$composeDir = if ($Dir) { (Join-Path $PSScriptRoot $Dir) } else { $PSScriptRoot }
if (-not (Test-Path (Join-Path $composeDir 'docker-compose.yml'))) {
    throw "docker-compose.yml not found in '$composeDir' (pass -Dir ..\deploy_zh-cn\win or ..\deploy_en\win; Linux use start.sh)"
}
Set-Location $composeDir

# Load .env into the process environment (shared by compose and local validate checks)
$envFile = Join-Path $composeDir '.env'
if (-not (Test-Path $envFile)) {
    throw "缺少 .env：请在 $composeDir 下 copy ..\..\deploy\.env.example .env 并填写 MAPSERVER_DB_PASSWORD / MANAGER_JWT_SECRET"
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
    $dbPort = if ($env:MAPSERVER_DB_PORT) { [int]$env:MAPSERVER_DB_PORT } else { 5432 }
    & (Join-Path $PSScriptRoot 'validate.ps1') -DbPort $dbPort -Dir $Dir -CheckDataRoot
}

Write-Output ''
Write-Output '前端    : http://localhost:6180/'
Write-Output 'map-api : http://localhost:8080/health'
Write-Output '停止    : docker compose down   （删卷加 -v）'
