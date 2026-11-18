# ============================================================================
# P0-4 cross-service connectivity self-check -- deploy/validate.ps1
#
# Liveness + functional + data_root alignment checks; works for containers and local (-Base selects target).
# Any FAIL -> exit code 1 (consumed by start.ps1 / CI).
#
# Usage:
#   pwsh -File validate.ps1                       # default: local 8080/6180/5434
#   pwsh -File validate.ps1 -CheckDataRoot        # also verify data_root (default ./data) and TESTDATA_ROOT
#   pwsh -File validate.ps1 -AdminToken 'xxx'     # when map-api has admin.token configured
# ============================================================================
param(
    [string]$MapApiBase = 'http://localhost:8080',
    [string]$ManagerBase = 'http://localhost:6180',
    [string]$DbHost = 'localhost',
    [int]$DbPort = 5434,
    [string]$AdminToken = '',
    [string]$Dir = '',
    [switch]$CheckDataRoot
)

$results = @()
function Add($name, $status, $detail) {
    $script:results += [pscustomobject]@{ Check = $name; Status = $status; Detail = $detail }
}

$headers = @{}
if ($AdminToken) { $headers['X-Admin-Token'] = $AdminToken }

# Liveness: any HTTP status proves the service is up; connection failure/timeout = FAIL
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

# 1) PostGIS port reachability
$ok = Test-NetConnection -ComputerName $DbHost -Port $DbPort -InformationLevel Quiet -WarningAction SilentlyContinue
Add 'PostGIS TCP' ($(if ($ok) { 'PASS' } else { 'FAIL' })) "$DbHost`:$DbPort"

# 2) map-api /health
$null = Probe 'map-api /health' "$MapApiBase/health"

# 3) map-api WMS GetCapabilities (data plane)
$cap = Probe 'map-api WMS GetCapabilities' "$MapApiBase/wms?service=WMS&version=1.3.0&request=GetCapabilities"
if ($cap -and $cap.Content -match 'WMS_Capabilities|ServiceMetadata') {
    $results[-1].Detail = 'capabilities 正常'
}

# 4) map-api /admin/components (admin plane + data_root display)
$ac = Probe 'map-api /admin/components' "$MapApiBase/admin/components"
if ($ac) {
    try {
        $j = $ac.Content | ConvertFrom-Json
        $dr = $j.data_root
        Add 'data_root 配置' 'INFO' ($(if ([string]::IsNullOrWhiteSpace($dr)) { '(未设 -> 不设限，向导可访问全盘)' } else { $dr }))
    } catch { }
}

# 5) manager root (static hosting on the same port; any response means it is up)
$null = Probe 'manager /' "$ManagerBase/"

# 6) gisdata mount-point directory existence (optional)
#    compose mounts GISDATA_DIR (default ./gisdata under the selected deploy dir)
#    at /app/data in both containers; ws1's data_root points there.
if ($CheckDataRoot) {
    $deployDir = if ($Dir) { Join-Path $PSScriptRoot $Dir } else { $PSScriptRoot }
    $dr = [Environment]::GetEnvironmentVariable('GISDATA_DIR')
    if ([string]::IsNullOrWhiteSpace($dr)) { $dr = Join-Path $deployDir 'gisdata' }
    Add 'gisdata 目录' ($(if (Test-Path $dr) { 'PASS' } else { 'WARN' })) $dr

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
