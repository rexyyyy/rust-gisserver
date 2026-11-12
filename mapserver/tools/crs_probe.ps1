# CRS 能力实测探针：验证内置 4326/3857 与 PROJ 后端的 CGCS2000 / UTM 互转。
# 用法：pwsh -File tools/crs_probe.ps1 [-Base http://127.0.0.1:8095] [-Layer wh4490]
param(
    [string]$Base = "http://127.0.0.1:8095",
    [string]$Layer = "wh4490"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function GetMap($crs, $bbox, $w = 256, $h = 256) {
    $url = "$Base/wms?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=$Layer" +
        "&CRS=$crs&BBOX=$bbox&WIDTH=$w&HEIGHT=$h&FORMAT=image/png&TRANSPARENT=TRUE"
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -SkipHttpErrorCheck -TimeoutSec 120
    if ($r.StatusCode -eq 200) {
        $head = ($r.Content[0..7] | ForEach-Object { $_.ToString("x2") }) -join ""
        [pscustomobject]@{ CRS = $crs; Status = $r.StatusCode; Bytes = $r.Content.Length; PNG = $head.StartsWith("89504e47") }
    }
    else {
        $txt = ($r.Content | ForEach-Object { [char]$_ }) -join ""
        $snip = if ($txt -match "ExceptionText[^>]*>([^<]*)") { $Matches[1] } else { $txt }
        [pscustomobject]@{ CRS = $crs; Status = $r.StatusCode; Bytes = 0; PNG = $false; Error = $snip }
    }
}

# 五华区影像范围（lon/lat，由 GetCapabilities 宣告）：102.5457,25.0249 .. 102.7589,25.4330
$degNoFirst = "25.0249,102.5457,25.4330,102.7589"   # WMS 1.3.0 地理 CRS：northing-first
$degEastFirst = "102.5457,25.0249,102.7589,25.4330" # CRS:84 / 1.1.1：easting-first

function MercX($lon) { [math]::Round($lon * 20037508.3427892 / 180.0, 2) }
function MercY($lat) {
    $r = $lat * [math]::PI / 180.0
    [math]::Round(20037508.3427892 * [math]::Log([math]::Tan([math]::PI / 4 + $r / 2)) / [math]::PI, 2)
}
$x0 = MercX 102.5457; $x1 = MercX 102.7589; $y1 = MercY 25.4330; $y0 = MercY 25.0249
$mercBbox = "$x0,$y0,$x1,$y1"

Write-Host "== WMS GetMap on layer '$Layer' (src EPSG:4490) =="
GetMap "EPSG:4490" $degNoFirst   | Format-Table -AutoSize | Out-String | Write-Host
GetMap "EPSG:4326" $degNoFirst   | Format-Table -AutoSize | Out-String | Write-Host
GetMap "CRS:84" $degEastFirst    | Format-Table -AutoSize | Out-String | Write-Host
GetMap "EPSG:3857" $mercBbox      | Format-Table -AutoSize | Out-String | Write-Host
GetMap "urn:ogc:def:crs:EPSG::4490" $degNoFirst | Format-Table -AutoSize | Out-String | Write-Host

Write-Host "== 同一地物在 4326 / 4490 下的出图一致性（像素差）=="
function FetchPng($crs, $bbox, $out) {
    $url = "$Base/wms?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=$Layer" +
        "&CRS=$crs&BBOX=$bbox&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE"
    Invoke-WebRequest -Uri $url -UseBasicParsing -OutFile $out -TimeoutSec 120
    (Get-Item $out).Length
}
$a = FetchPng "EPSG:4326" $degNoFirst "$env:TEMP\crs4326.png"
$b = FetchPng "EPSG:4490" $degNoFirst "$env:TEMP\crs4490.png"
Write-Host "EPSG:4326 png = $a bytes ; EPSG:4490 png = $b bytes"

Write-Host "== 不支持的 CRS 应被明确拒绝 =="
GetMap "EPSG:2326" $degNoFirst | Format-List | Out-String | Write-Host
GetMap "EPSG:32647" "0,0,1,1"  | Format-List | Out-String | Write-Host
GetMap "EPSG:9999" $degNoFirst | Format-List | Out-String | Write-Host

Write-Host "== CGCS2000 投影带（EPSG:4543，CM 102E，米）跨 CRS 出图 =="
# 4543 真实米制 bbox（PostGIS ST_Extent 实测）：556300,2770041.8 .. 575279.1,2811666.0
GetMap "EPSG:4543" "556300,2770041.8,575279.1,2811666" | Format-Table -AutoSize | Out-String | Write-Host

Write-Host "== 瓦片端点：4490 自定义网格（CGCS2000Geo）与 3857 XYZ =="
function FetchTile($url) {
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -SkipHttpErrorCheck -TimeoutSec 120
    $bytes = if ($r.Content -is [byte[]]) { $r.Content } else { [System.Text.Encoding]::UTF8.GetBytes([string]$r.Content) }
    $head = if ($bytes.Length -ge 4) { ($bytes[0..3] | ForEach-Object { $_.ToString("x2") }) -join "" } else { "-" }
    [pscustomobject]@{ Url = ($url -replace [regex]::Escape($Base), ""); Status = $r.StatusCode; Bytes = $bytes.Length; Kind = $head }
}
# CGCS2000Geo：原点 (-180,90)，z10 分辨率 0.0006866455078125 度/px，256px -> 0.1757 度/瓦
# lon 102.65 -> x=1608 ; lat 25.2 -> y=368
FetchTile "$Base/tiles/$Layer/CGCS2000Geo/10/1608/368.png" | Format-Table -AutoSize | Out-String | Write-Host
FetchTile "$Base/tms/1.0.0/$Layer/CGCS2000Geo/10/1608/655" | Format-Table -AutoSize | Out-String | Write-Host
FetchTile "$Base/tiles/$Layer/10/803/437.png" | Format-Table -AutoSize | Out-String | Write-Host
FetchTile "$Base/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=$Layer&TILEMATRIXSET=CGCS2000Geo&TILEMATRIX=10&TILEROW=368&TILECOL=1608&TILEFORMAT=image/png" | Format-Table -AutoSize | Out-String | Write-Host

Write-Host "== 源为 EPSG:4326 的图层（wh）被请求为 4490：反方向 CGCS2000 互转 =="
function GetMapOn($layer, $crs, $bbox) {
    $url = "$Base/wms?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=$layer" +
        "&CRS=$crs&BBOX=$bbox&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE"
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -SkipHttpErrorCheck -TimeoutSec 120
    [pscustomobject]@{ Layer = $layer; CRS = $crs; Status = $r.StatusCode; Bytes = $r.Content.Length }
}
GetMapOn "wh" "EPSG:4490" $degNoFirst | Format-Table -AutoSize | Out-String | Write-Host
GetMapOn "wh" "EPSG:4543" "556300,2770041.8,575279.1,2811666" | Format-Table -AutoSize | Out-String | Write-Host
