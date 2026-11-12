# routers_probe.ps1 - log in to the manager backend and check the "data module"
# contract: the dynamic router tree (getRouters) plus the endpoints the four
# feature pages call. Python is not available on this box, so this replaces the
# tools/*.py probes.
#
#   pwsh -File tools/routers_probe.ps1 [-Base http://127.0.0.1:6180/manager-api]
#
# The engine (map-api) must be reachable for the capabilities check; the other
# reads only need the manager database. Exit 0 when all expectations hold.
param(
    [string]$Base = 'http://127.0.0.1:6180/manager-api',
    [string]$User = 'admin',
    [string]$Pass = 'admin123'
)

$ErrorActionPreference = 'Stop'
$script:fail = 0

function Say($ok, $label, $detail) {
    $tag = if ($ok) { 'PASS' } else { 'FAIL' }
    if (-not $ok) { $script:fail++ }
    Write-Output ("{0}  {1}  {2}" -f $tag, $label, $detail)
}

# --- login ---
$body = @{ username = $User; password = $Pass; code = ''; uuid = '' } | ConvertTo-Json
try {
    $login = Invoke-RestMethod -Uri "$Base/auth/login" -Method Post -ContentType 'application/json' -Body $body
} catch {
    Write-Output "FAIL  login  cannot reach $Base : $($_.Exception.Message)"
    exit 1
}
if ($login.code -ne 200 -or -not $login.data.access_token) {
    Write-Output "FAIL  login  code=$($login.code) msg=$($login.msg)"
    exit 1
}
$token = $login.data.access_token
Say $true 'login' 'token issued'

# --- getRouters ---
$hdr = @{ Authorization = "Bearer $token" }
function Get2($path) {
    try { return Invoke-RestMethod -Uri "$Base$path" -Headers $hdr }
    catch { return @{ code = -1; msg = $_.Exception.Message } }
}
$routers = (Get2 '/system/menu/getRouters').data

# 1) the Data module exists as a top-level directory
$data = $routers | Where-Object { $_.path -eq '/data' }
Say ([bool]$data) 'menu 123 /data' ("present=" + [bool]$data + " children=" + @($data.children).Count)

# 2) each feature page is a child route with its own component
$want = @(
    @{ path = 'catalog';   comp = 'map/catalog/index' },
    @{ path = 'workspace'; comp = 'map/catalog/workspace' },
    @{ path = 'store';     comp = 'map/catalog/store' },
    @{ path = 'resource';  comp = 'map/catalog/resource' },
    @{ path = 'published'; comp = 'map/catalog/published' }
)
foreach ($w in $want) {
    $hit = @($data.children) | Where-Object { $_.path -eq $w.path }
    $ok = ($hit.Count -eq 1) -and ($hit[0].component -eq $w.comp)
    Say $ok ("menu " + $w.path) ("component=" + $(if ($hit.Count) { $hit[0].component } else { '<missing>' }))
}

# 3) sidebar order: 数据功能 right behind 地图服务
$top = @($routers | ForEach-Object { $_.path })
$ixMap = $top.IndexOf('/map'); $ixData = $top.IndexOf('/data')
$ok = ($ixMap -ge 0) -and ($ixData -eq $ixMap + 1)
Say $ok 'top order' ($top -join ' ')

# 4) the endpoints the four pages call (workspace / store / resource / published)
# capabilities is proxied to the engine (sys_engine.base_url); an unreachable
# engine is an environment gap, not a broken menu, so it only warns.
$cap = Get2 '/map/catalog/capabilities'
if ($cap.code -eq 200 -and $cap.data -and @($cap.data.data_types).Count -gt 0) {
    Say $true 'api capabilities' ('data_types=' + @($cap.data.data_types).Count)
} else {
    Write-Output ("WARN  api capabilities  code={0} msg={1} (engine down? check sys_engine.base_url)" -f $cap.code, $cap.msg)
}
$ws = Get2 '/map/catalog/workspaces?includeDeleted=1'
Say ($ws.code -eq 200 -and @($ws.data.workspaces).Count -gt 0) 'api workspaces' ("code=" + $ws.code + " rows=" + @($ws.data.workspaces).Count)
$st = Get2 '/map/catalog/stores?includeDeleted=1'
Say ($st.code -eq 200) 'api stores' ("code=" + $st.code + " rows=" + @($st.data.stores).Count)
$pub = Get2 '/map/catalog/published?includeDeleted=1'
Say ($pub.code -eq 200) 'api published' ("code=" + $pub.code + " rows=" + @($pub.data.published).Count)
# resources need an exact storeId (storeId=0 returns nothing by design)
$first = @($st.data.stores) | Where-Object { $_.delFlag -eq 0 } | Select-Object -First 1
if ($first) {
    $rs = Get2 ("/map/catalog/resources?storeId={0}&includeDeleted=1" -f $first.id)
    Say ($rs.code -eq 200) 'api resources' ("code=" + $rs.code + " storeId=" + $first.id + " rows=" + @($rs.data.resources).Count)
} else {
    Write-Output 'SKIP  api resources  no live store in the database'
}

if ($script:fail -eq 0) { Write-Output 'ALL PASS'; exit 0 }
Write-Output "$($script:fail) check(s) failed"; exit 1
