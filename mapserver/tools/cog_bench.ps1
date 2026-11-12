# COG 瓦片耗时基准：区分「服务端现算慢」与「浏览器侧排队慢」。
#
#   pwsh -NoProfile -File tools\cog_bench.ps1 -Z 20 -Span 3 -Purge
#
# -Purge 先清空服务端全部缓存（含解码瓦 + 出图结果，L1/L2），因此第一行
# COLD 是真正的「现算」耗时；第二行 HOT 走缓存。COLD parallel8 用一批从没
# 请求过的瓦（窗口平移 400 格）测并发吞吐。
#
# 参考量级（wh = 6.4GB BigTIFF，EPSG:4326 源 + WebMercatorQuad 重投影）：
#   release  冷瓦 26~45ms / 热瓦 3~7ms
#   debug    冷瓦 135~1073ms      -> 测性能务必用 release 构建
param(
  [string]$Base = 'http://127.0.0.1:8080',
  [string]$Layer = 'wh',
  [int]$Z = 20,
  [int]$Span = 3,
  [switch]$Purge
)
$ErrorActionPreference = 'Stop'
$HALF = 20037508.3427892
$R = 6378137.0

# 取样窗口中心：102.65E, 25.23N（落在 wh 图层范围内）
function Tile-List([int]$z, [int]$span, [int]$off) {
  $size = 2.0 * $HALF / [Math]::Pow(2, $z)
  $cx = 102.65 * $HALF / 180.0
  $cy = [Math]::Log([Math]::Tan([Math]::PI / 4.0 + (25.23 * [Math]::PI / 180.0) / 2.0)) * $R
  $x0 = [int][Math]::Floor(($cx + $HALF) / $size) + $off
  $y0 = [int][Math]::Floor(($HALF - $cy) / $size) + $off
  $out = @()
  for ($i = 0; $i -lt $span; $i++) {
    for ($j = 0; $j -lt $span; $j++) { $out += "$z/$($x0 + $i)/$($y0 + $j).png" }
  }
  return $out
}

function Run-Serial($tiles, $label) {
  $sw = [Diagnostics.Stopwatch]::StartNew(); $bytes = 0; $each = @()
  foreach ($t in $tiles) {
    $s = [Diagnostics.Stopwatch]::StartNew()
    $r = Invoke-WebRequest "$Base/tiles/$Layer/$t" -TimeoutSec 300
    $s.Stop()
    $bytes += $r.Content.Length
    $each += ('{0:F0}' -f $s.Elapsed.TotalMilliseconds)
  }
  $sw.Stop()
  '{0,-14} n={1,-3} total={2,-8} avg={3,-8} bytes={4}' -f $label, $tiles.Count, `
    ('{0:F0}ms' -f $sw.Elapsed.TotalMilliseconds), ('{0:F0}ms' -f ($sw.Elapsed.TotalMilliseconds / $tiles.Count)), $bytes
  '               per-tile: ' + ($each -join ' ')
}

$cold = Tile-List $Z $Span 0
if ($Purge) {
  try { $null = Invoke-WebRequest "$Base/api/cache/purge?scope=all" -Method Post -TimeoutSec 60; 'purged: all caches' }
  catch { 'purge failed: ' + $_.Exception.Message }
}

"=== layer=$Layer  z=$Z  grid=${Span}x${Span}  base=$Base ==="
Run-Serial $cold 'COLD serial'
Run-Serial $cold 'HOT  serial'

# 另取一扇从未请求过的窗口，测并发
$fresh = Tile-List $Z $Span 400
$sw = [Diagnostics.Stopwatch]::StartNew()
$sum = $fresh | ForEach-Object -Parallel {
  $r = Invoke-WebRequest "$using:Base/tiles/$using:Layer/$_" -TimeoutSec 300
  $r.Content.Length
} -ThrottleLimit 8 | Measure-Object -Sum
$sw.Stop()
'COLD parallel8 n={0} total={1} avg={2} bytes={3}' -f $fresh.Count, `
  ('{0:F0}ms' -f $sw.Elapsed.TotalMilliseconds), ('{0:F0}ms' -f ($sw.Elapsed.TotalMilliseconds / $fresh.Count)), $sum.Sum
