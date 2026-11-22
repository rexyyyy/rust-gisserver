# End-to-end MVT check for custom projection grid (EPSG:4522 / wh_custom_grid_4522), after proj integration.
# Criteria: >15 B = has features (an empty tile is exactly 15 B); identical SHA256 for default
$ErrorActionPreference = "Stop"
$Base = "http://127.0.0.1:8080"
$Layer = "e2e-fp"
$Tms = "wh_custom_grid_4522"
# and explicit grid paths proves encode_projected. Non-square dimensions come from WMTS GetCapabilities.
$Dim = @{ 0 = @(2,1); 1 = @(3,2); 2 = @(5,3); 3 = @(9,6); 4 = @(17,12) }

function Fetch($url) {
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -SkipHttpErrorCheck -TimeoutSec 60
    $b = if ($r.Content -is [byte[]]) { $r.Content } else { [System.Text.Encoding]::UTF8.GetBytes([string]$r.Content) }
    [pscustomobject]@{ Status = $r.StatusCode; Bytes = $b; Len = $b.Length }
}
function Sha($b) { (Get-FileHash -InputStream ([IO.MemoryStream]::new($b)) -Algorithm SHA256).Hash }

$total = 0; $nonempty = 0; $empty15 = 0; $other = 0; $badStatus = 0
$cmpSamples = @()
foreach ($z in 0..4) {
    $w, $h = $Dim[[int]$z]
    for ($x = 0; $x -lt $w; $x++) {
        for ($y = 0; $y -lt $h; $y++) {
            $total++
            $d = Fetch "$Base/mvt/$Layer/$z/$x/$y"           # default path (tms=None -> default_grid)
            $e = Fetch "$Base/mvt/$Layer/$Tms/$z/$x/$y"       # explicit grid path
            if ($d.Status -ne 200 -or $e.Status -ne 200) { $badStatus++; Write-Host "!! HTTP $z/$x/$y default=$($d.Status) explicit=$($e.Status)"; continue }
            if ($d.Len -eq 15) { $empty15++ } elseif ($d.Len -gt 15) { $nonempty++ } else { $other++ }
            $same = (Sha $d.Bytes) -eq (Sha $e.Bytes)
            if (-not $same) { Write-Host "!! SHA DIFF $z/$x/$y (default $($d.Len) vs explicit $($e.Len))" }
            # Record SHA of the first 6 non-empty tiles for the report
            if ($d.Len -gt 15 -and $cmpSamples.Count -lt 6) {
                $cmpSamples += [pscustomobject]@{ Tile = "$z/$x/$y"; Len = $d.Len; SHA = (Sha $d.Bytes).Substring(0,16); Same = $same }
            }
        }
    }
}
Write-Host "`n==== 扫描汇总 (grid $Tms) ===="
Write-Host "合法瓦总数   : $total   (预期 2+6+15+54+204=281)"
Write-Host "含要素(>15B) : $nonempty"
Write-Host "空瓦(=15B)   : $empty15"
Write-Host "异常长度     : $other"
Write-Host "非200状态    : $badStatus"
Write-Host "SHA 一致性   : 所有非空采样 Same=$($cmpSamples | ForEach-Object { $_.Same })"
Write-Host "`n---- 非空瓦样例 ----"
$cmpSamples | Format-Table -AutoSize | Out-String | Write-Host

# Out-of-range requests should be 404 (beyond matrix_w/h)
Write-Host "==== 越界 404 验证 ===="
foreach ($t in @(@{z=0;x=2;y=0}, @{z=0;x=0;y=1}, @{z=2;x=5;y=0})) {
    $r = Fetch "$Base/mvt/$Layer/$Tms/$($t.z)/$($t.x)/$($t.y)"
    Write-Host "越界 $($t.z)/$($t.x)/$($t.y) -> HTTP $($r.Status) (预期 404)"
}
