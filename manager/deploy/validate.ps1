# ============================================================================
# P0-4 三仓联通链路自检 —— deploy/validate.ps1
#
# 探活 + 功能 + data_root 对齐检查，独立于容器/本地（用 -Base 参数切目标）。
# 任意 FAIL -> 退出码 1（供 start.ps1 / CI 判定）。
#
# 用法：
#   pwsh -File validate.ps1                       # 默认探本机 8080/6180/5434
#   pwsh -File validate.ps1 -CheckDataRoot        # 附带校验 data_root（默认 ./data）及 TESTDATA_ROOT
#   pwsh -File validate.ps1 -AdminToken 'xxx'     # map-api 配了 admin.token 时
# ============================================================================
param(
    [string]$MapApiBase = 'http://localhost:8080',
    [string]$ManagerBase = 'http://localhost:6180',
    [string]$DbHost = 'localhost',
    [int]$DbPort = 5434,
    [string]$AdminToken = '',
    [switch]$CheckDataRoot
)

$results = @()
function Add($name, $status, $detail) {
    $script:results += [pscustomobject]@{ Check = $name; Status = $status; Detail = $detail }
}

$headers = @{}
if ($AdminToken) { $headers['X-Admin-Token'] = $AdminToken }

# 存活探测：拿到任意 HTTP 状态码即证明服务在；连不上/超时 = FAIL
function Probe($name, $url) {
    try {
        $r = Invoke-WebRequest -UseBasicParsing -Uri $url -Headers $headers -TimeoutSec 10
        Add $name 'PASS' ("HTTP " + $r.StatusCode)
        return $r
    } catch [Microsoft.PowerShell.Commands.HttpResponseException] {
        Add $name 'WARN' ('HTTP ' + [int]$_.Exception.Response.StatusCode + '（服务在，端点或需鉴权）')
    } catch {
        Add $name 'FAIL' $_.Exception.Message
    }
    return $null
}

# 1) PostGIS 端口连通
$ok = Test-NetConnection -ComputerName $DbHost -Port $DbPort -InformationLevel Quiet -WarningAction SilentlyContinue
Add 'PostGIS TCP' ($(if ($ok) { 'PASS' } else { 'FAIL' })) "$DbHost`:$DbPort"

# 2) map-api /health
$null = Probe 'map-api /health' "$MapApiBase/health"

# 3) map-api WMS GetCapabilities（数据面功能）
$cap = Probe 'map-api WMS GetCapabilities' "$MapApiBase/wms?service=WMS&version=1.3.0&request=GetCapabilities"
if ($cap -and $cap.Content -match 'WMS_Capabilities|ServiceMetadata') {
    $results[-1].Detail = 'capabilities 正常'
}

# 4) map-api /admin/components（管理面 + data_root 展示）
$ac = Probe 'map-api /admin/components' "$MapApiBase/admin/components"
if ($ac) {
    try {
        $j = $ac.Content | ConvertFrom-Json
        $dr = $j.data_root
        Add 'data_root 配置' 'INFO' ($(if ([string]::IsNullOrWhiteSpace($dr)) { '(未设 -> 不设限，向导可访问全盘)' } else { $dr }))
    } catch { }
}

# 5) manager 根（静态托管同端口，拿到任意响应即在）
$null = Probe 'manager /' "$ManagerBase/"

# 6) 本地 data_root 目录存在性（可选）
#    config/mapserver.xml 用 ${DATA_ROOT:-data}：未设置时取部署目录下 ./data。
if ($CheckDataRoot) {
    $dr = [Environment]::GetEnvironmentVariable('DATA_ROOT')
    if ([string]::IsNullOrWhiteSpace($dr)) { $dr = Join-Path $PSScriptRoot 'data' }
    Add 'data_root 目录' ($(if (Test-Path $dr) { 'PASS' } else { 'WARN' })) $dr

    $tdr = [Environment]::GetEnvironmentVariable('TESTDATA_ROOT')
    if ([string]::IsNullOrWhiteSpace($tdr)) {
        Add 'TESTDATA_ROOT 目录' 'INFO' 'env 未设置（e2e demo 图层跳过，非致命）'
    } else {
        Add 'TESTDATA_ROOT 目录' ($(if (Test-Path $tdr) { 'PASS' } else { 'WARN' })) $tdr
    }
}

$results | Format-Table -AutoSize
$fail = @($results | Where-Object Status -eq 'FAIL').Count
if ($fail) {
    Write-Output ("存在 {0} 项 FAIL" -f $fail)
    exit 1
} else {
    Write-Output '全部关键检查通过'
    exit 0
}
