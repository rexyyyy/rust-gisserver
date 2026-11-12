# 瓦片端点错误口径探活：验证「请求问题 -> 4xx / 服务端故障 -> 500」的分流，
# 以及能力矩阵（引擎 /admin/capabilities）与实际行为是否一致。
# 管理端（rust-gisserver）就是按状态码判断某层能否发布某服务，所以这些码是契约的一部分。
#
# 用法：pwsh -File tools/caps_status_probe.ps1 [-Base http://127.0.0.1:8099]
# 前置：map-api 已用仓库 config/mapserver.xml 启动，且加载了
#       places(静态矢量) / wh(COG) / test(栅格归档) / test-vector(矢量归档) 四层。
# 返回：全部符合期望则 exit 0，否则 exit 1。
param(
  [string]$Base = 'http://127.0.0.1:8099'
)
$ErrorActionPreference = 'Continue'

$fails = @()

function Expect($label, $url, [int]$want) {
  $f = [IO.Path]::GetTempFileName()
  $got = [int](& curl.exe -s -o $f -w "%{http_code}" --max-time 120 $url)
  $size = (Get-Item $f).Length
  $peek = ''
  if ($got -ne $want) {
    $peek = ((Get-Content $f -Raw -ErrorAction SilentlyContinue) -replace '\s+', ' ')
    if ($peek.Length -gt 120) { $peek = $peek.Substring(0, 120) + '...' }
  }
  Remove-Item $f -Force -ErrorAction SilentlyContinue
  if ($got -eq $want) {
    'OK    {0,-34} {1}  ({2}B)' -f $label, $got, $size
  } else {
    'FAIL  {0,-34} got {1} want {2}  body: {3}' -f $label, $got, $want, $peek
    $script:fails += $label
  }
}

$WMTS = "$Base/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&STYLE=default&TILEMATRIXSET=WebMercatorQuad"

# 静态矢量层：矩阵声称 WMS/WMTS/XYZ/MVT/WFS/OAF 全开，逐一确认 200。
Expect 'places WMS GetMap'        "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=places&SRS=EPSG:4326&BBOX=100,22,106,27&WIDTH=128&HEIGHT=128&FORMAT=image/png" 200
Expect 'places XYZ z2'            "$Base/tiles/places/2/3/1.png" 200
Expect 'places WMTS z2'           "$WMTS&LAYER=places&TILEMATRIX=2&TILEROW=1&TILECOL=3&FORMAT=image/png" 200
Expect 'places MVT z2'            "$Base/mvt/places/2/3/1.pbf" 200
Expect 'places WFS GetFeature'    "$Base/wfs?SERVICE=WFS&VERSION=1.1.0&REQUEST=GetFeature&TYPENAME=places&MAXFEATURES=2" 200
Expect 'places OAF items'         "$Base/ogc/features/v1/collections/places/items?limit=2" 200

# 栅格归档：WMTS/XYZ 直读可 200；WMS 与 MVT 属选错接口 -> 400。
Expect 'test XYZ z0'              "$Base/tiles/test/0/0/0.png" 200
Expect 'test WMTS z0'             "$WMTS&LAYER=test&TILEMATRIX=0&TILEROW=0&TILECOL=0&FORMAT=image/png" 200
Expect 'test WMS (no raster src)' "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=test&SRS=EPSG:4326&BBOX=-180,-90,180,90&WIDTH=64&HEIGHT=64&FORMAT=image/png" 400
Expect 'test MVT (not vector)'    "$Base/mvt/test/0/0/0.pbf" 400

# 矢量归档：只有 /mvt 认它；栅格端点必须拒绝，不能 200 透传 pbf。
Expect 'test-vector MVT z0'       "$Base/mvt/test-vector/0/0/0.pbf" 200
Expect 'test-vector XYZ (reject)' "$Base/tiles/test-vector/0/0/0.png" 400
Expect 'test-vector WMTS (reject)' "$WMTS&LAYER=test-vector&TILEMATRIX=0&TILEROW=0&TILECOL=0&FORMAT=image/png" 400
Expect 'test-vector WMS (reject)' "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=test-vector&SRS=EPSG:4326&BBOX=-180,-90,180,90&WIDTH=64&HEIGHT=64&FORMAT=image/png" 400

# 该地址没有瓦 -> 404（客户端应停止重试），不是 500。
Expect 'wh COG XYZ z10 (render)'  "$Base/tiles/wh/10/803/437.png" 200
Expect 'test XYZ missing tile'    "$Base/tiles/test/12/2048/2048.png" 404
Expect 'test WMTS missing tile'   "$WMTS&LAYER=test&TILEMATRIX=12&TILEROW=2048&TILECOL=2048&FORMAT=image/png" 404
Expect 'test-vector MVT missing'  "$Base/mvt/test-vector/12/2048/2048.pbf" 404
Expect 'OAT missing tile'         "$Base/ogc/tiles/v1/collections/test/tiles/WebMercatorQuad/12/2048/2048" 404
Expect 'OAT z0'                   "$Base/ogc/tiles/v1/collections/test/tiles/WebMercatorQuad/0/0/0" 200
Expect 'COG z26 above matrix'     "$Base/tiles/wh/26/1/1.png" 404
Expect 'unknown layer'            "$Base/tiles/nope/0/0/0.png" 404
Expect 'unknown grid'             "$Base/tiles/wh/NoSuchGrid/0/0/0.png" 404

# COG 层请求 MVT / PostGIS 层请求栅格瓦：同为接口不匹配 -> 400。
Expect 'MVT on COG (reject)'      "$Base/mvt/wh/0/0/0.pbf" 400
Expect 'WMS unknown layer'        "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=nope&SRS=EPSG:4326&BBOX=-180,-90,180,90&WIDTH=64&HEIGHT=64&FORMAT=image/png" 400
Expect 'postgis WMTS (reject)'    "$WMTS&LAYER=wuhua_buildings&TILEMATRIX=6&TILEROW=41&TILECOL=33&FORMAT=image/png" 400
Expect 'postgis XYZ (reject)'     "$Base/tiles/wuhua_buildings/6/33/41.png" 400

''
if ($fails.Count -eq 0) {
  'ALL PASS'
  exit 0
}
"FAILED: $($fails -join ', ')"
exit 1
