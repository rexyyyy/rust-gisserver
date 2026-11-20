# ====================================================================
#  rust-gisserver 数据库初始化一键脚本
#  用法：powershell -ExecutionPolicy Bypass -File init_all.ps1 -Port 5432 -Password 123456
# ====================================================================
param(
    [int]    $Port     = 5432,
    [string] $DBUser   = "postgres",
    [string] $DBName   = "mapserver-manager",
    [string] $Password = ""
)

$ErrorActionPreference = "Stop"

# 找 psql
$psql = Get-ChildItem "C:\Program Files\PostgreSQL" -Recurse -Filter "psql.exe" -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
if (-not $psql) { throw "找不到 psql.exe — 请确认 PostgreSQL 已安装" }
Write-Host "psql: $psql"

if ($Password) { $env:PGPASSWORD = $Password }

Write-Host "
=== [1/3] 创建数据库 + PostGIS 扩展 ==="
& $psql -U $DBUser -p $Port -f "01-init-database.sql" 2>&1 | Write-Host

Write-Host "
=== [2/3] 初始化系统表 (system.sql) ==="
& $psql -U $DBUser -p $Port -d $DBName -f "system.sql" 2>&1 | Write-Host

Write-Host "
=== [3/3] 初始化地图服务表 (map.sql) ==="
& $psql -U $DBUser -p $Port -d $DBName -f "map.sql" 2>&1 | Write-Host

Write-Host "
=== ✅ 数据库初始化完成 ==="
Write-Host "  DB:   $DBName"
Write-Host "  Port: $Port"
Write-Host "  User: $DBUser"
