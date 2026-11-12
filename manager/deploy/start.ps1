# ============================================================================
# P0-4 一键启动 + 探活 —— deploy/start.ps1
#
# 读 .env -> docker compose up（默认 --build）-> 等 PostGIS healthy -> 跑 validate.ps1。
#
# 用法：
#   pwsh -File start.ps1              # 构建并起栈，再自检
#   pwsh -File start.ps1 -NoBuild     # 不重建镜像直接起
#   pwsh -File start.ps1 -SkipValidate
# ============================================================================
param(
    [switch]$NoBuild,
    [switch]$SkipValidate
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# 载入 .env 到进程环境（compose 读 + validate 本地检查共用）
$envFile = Join-Path $PSScriptRoot '.env'
if (-not (Test-Path $envFile)) {
    throw '缺少 .env：请先 copy .env.example .env 并填写 MAPSERVER_DB_PASSWORD / MANAGER_JWT_SECRET'
}
Get-Content -LiteralPath $envFile | ForEach-Object {
    if ($_ -match '^\s*([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2].Trim(), 'Process')
    }
}

# 起栈
$up = @('compose', 'up', '-d')
if (-not $NoBuild) { $up += '--build' }
Write-Output ('docker ' + ($up -join ' '))
docker @up

# 等 PostGIS healthy（最多 3 分钟）
Write-Output '等待 PostGIS 健康…'
$deadline = (Get-Date).AddMinutes(3)
$healthy = $false
do {
    Start-Sleep -Seconds 3
    $ps = docker compose ps --format '{{.Name}} {{.State}} {{.Health}}' 2>$null
    if ($ps -match 'gis-postgis.*running.*healthy') { $healthy = $true; break }
} while ((Get-Date) -lt $deadline)
if (-not $healthy) { Write-Warning 'PostGIS 未在 3 分钟内转 healthy，继续尝试自检' }

# 自检
if (-not $SkipValidate) {
    $dbPort = if ($env:MAPSERVER_DB_PORT) { [int]$env:MAPSERVER_DB_PORT } else { 5434 }
    & (Join-Path $PSScriptRoot 'validate.ps1') -DbPort $dbPort -CheckDataRoot
}

Write-Output ''
Write-Output '前端    : http://localhost:6180/'
Write-Output 'map-api : http://localhost:8080/health'
Write-Output '停止    : docker compose down   （删卷加 -v）'
