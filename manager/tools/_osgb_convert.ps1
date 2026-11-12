# OSGB4326 -> 3D Tiles conversion via gis-srv (tiles.osgb / gislab.osgb2tiles)
# Adapted from gis-wt/scripts/_convert_mcy3.ps1. ASCII-only to avoid PS 5.1 BOM issues.
$ErrorActionPreference = "Continue"
$build      = "D:\mycomputor\documents\gis-wt\build"
$root       = "D:\mycomputor\documents\gis-wt"
$installBin = "D:\mycomputor\documents\gis-wt\thirdparty\install\bin"
$InDir      = "E:\data_km\OSGB4326"
$OutDir     = Join-Path $root ".testdata\3dtiles\osgb4326_3dtiles"
$log        = Join-Path $build "_convert_osgb4326.log"
"STEP0 $(Get-Date) start input=$InDir output=$OutDir" | Out-File -FilePath $log -Encoding utf8
function Log($s) { ("{0} {1}" -f (Get-Date -Format HH:mm:ss), $s) | Out-File -Append -FilePath $log -Encoding utf8 }

$env:Path = "$build;$installBin;" + $env:Path
$env:OSG_PLUGIN_PATH = Join-Path $installBin "osgPlugins-3.6.5"
if (Test-Path (Join-Path $build "share")) {
    $env:PROJ_LIB  = Join-Path $build "share\proj"
    $env:PROJ_DATA = Join-Path $build "share\proj"
    $env:GDAL_DATA = Join-Path $build "share\gdal"
}
$env:GIS_SRV_PORT = "7070"
Log "env ready; OSG_PLUGIN_PATH exists=$(Test-Path $env:OSG_PLUGIN_PATH); INPUT exists=$(Test-Path $InDir)"

function Start-GisSrv($exe, $buildDir, $extra) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.WorkingDirectory = $buildDir
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables.Clear()
    foreach ($k in [Environment]::GetEnvironmentVariables('Process').Keys) {
        if ($k -ieq 'PATH') { continue }
        try { $psi.EnvironmentVariables[$k] = [Environment]::GetEnvironmentVariable($k, 'Process') } catch { }
    }
    $psi.EnvironmentVariables['PATH'] = "$buildDir;" + [Environment]::GetEnvironmentVariable('PATH', 'Process')
    foreach ($kv in $extra.GetEnumerator()) { $psi.EnvironmentVariables[$kv.Key] = $kv.Value }
    try { return [System.Diagnostics.Process]::Start($psi) }
    catch { Log ("Start-GisSrv inner err: " + $_.Exception.Message); return $null }
}

try {
    $exe = Join-Path $build "gis-srv.exe"
    Log ("exe exists=" + (Test-Path $exe))
    Get-Process -Name 'gis-srv' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    $extra = @{ OSG_PLUGIN_PATH = $env:OSG_PLUGIN_PATH; GIS_SRV_PORT = '7070' }
    if ($env:PROJ_LIB) { $extra['PROJ_LIB'] = $env:PROJ_LIB; $extra['PROJ_DATA'] = $env:PROJ_DATA; $extra['GDAL_DATA'] = $env:GDAL_DATA }
    $proc = Start-GisSrv $exe $build $extra
    if (-not $proc) { Log "gis-srv process is NULL (failed to start)"; exit 1 }
    Log ("gis-srv pid=" + $proc.Id)
} catch { Log ("START ERR: " + $_.Exception.Message); exit 1 }

$base = "http://127.0.0.1:7070"
$healthy = $false
for ($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 500
    try { $r = Invoke-RestMethod -Uri "$base/health" -TimeoutSec 3; if ($r.status -eq 'ok') { $healthy = $true; break } } catch { }
}
if (-not $healthy) { Log "HEALTH FAIL"; exit 1 }
Log "HEALTHY"

try {
    $pl = Invoke-RestMethod -Uri "$base/api/v1/plugins" -TimeoutSec 15
    $o  = $pl.plugins | Where-Object { $_.id -eq 'gislab.osgb2tiles' } | Select-Object -First 1
    Log ("plugin: " + $o.id + " v" + $o.version)
} catch { Log ("plugin err: " + $_.Exception.Message) }

if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force -ErrorAction SilentlyContinue }

function Submit($useTex) {
    $body = @{ tool_id = "tiles.osgb"; params = @{
        input        = $InDir
        output       = $OutDir
        dataset      = $true
        use_textures = $useTex
    } } | ConvertTo-Json -Depth 5
    Log ("POST job use_textures=$useTex")
    $jr = Invoke-RestMethod -Uri "$base/api/v1/jobs" -Method Post -ContentType 'application/json; charset=utf-8' -Body $body -TimeoutSec 30
    Log ("job submitted: " + $jr.job_id)
    return $jr.job_id
}

function Wait-Job($jid) {
    for ($i = 0; $i -lt 18000; $i++) {
        Start-Sleep -Seconds 2
        try {
            $st = Invoke-RestMethod -Uri "$base/api/v1/jobs/$jid" -TimeoutSec 5
            if ($i % 15 -eq 0 -or $st.status -ne 'running') {
                Log ("  poll $i : status=$($st.status) progress=$($st.progress) phase=$($st.phase) cur=$($st.current)/$($st.total) msg=$($st.message)")
            }
            if ($st.status -in @('done', 'failed', 'cancelled', 'interrupted')) {
                Log ("TERMINAL: $($st.status) error=$($st.error)")
                return $st
            }
        } catch { Log ("  poll $i err: " + $_.Exception.Message) }
    }
    Log "POLL TIMEOUT"; return $null
}

$jid = Submit $true
$st  = Wait-Job $jid
if ($st -and $st.status -eq 'failed') {
    Log "use_textures=true FAILED; retry without textures"
    $jid2 = Submit $false
    $st2  = Wait-Job $jid2
    if ($st2) { Log ("retry terminal: " + $st2.status) }
} else { Log ("final status: " + $st.status) }

try {
    if (Test-Path $OutDir) {
        Log ("output exists; tileset.json present=" + (Test-Path (Join-Path $OutDir "tileset.json")))
        $b3 = (Get-ChildItem $OutDir -Recurse -Filter *.b3dm -File).Count
        Log ("b3dm count=$b3")
    } else { Log "output dir MISSING" }
} catch { Log ("final scan err: " + $_.Exception.Message) }
Log "DONE"
