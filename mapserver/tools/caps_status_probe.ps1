# Tile endpoint error-code probe: verify client errors -> 4xx vs server faults -> 500,
# and that the capability matrix (engine /admin/capabilities) matches actual behaviour.
# The manager uses these status codes to decide publishability, so they are part of the contract.
#
# Usage: pwsh -File tools/caps_status_probe.ps1 [-Base http://127.0.0.1:8099]
# Prerequisite: map-api started with the repo config/mapserver.xml and loaded
#       places (static vector) / wh (COG) / test (raster archive) / test-vector (vector archive).
# Exit 0 when all expectations hold, otherwise 1.
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

# Static vector layer: matrix claims WMS/WMTS/XYZ/MVT/WFS/OAF all enabled; confirm 200 one by one.
Expect 'places WMS GetMap'        "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=places&SRS=EPSG:4326&BBOX=100,22,106,27&WIDTH=128&HEIGHT=128&FORMAT=image/png" 200
Expect 'places XYZ z2'            "$Base/tiles/places/2/3/1.png" 200
Expect 'places WMTS z2'           "$WMTS&LAYER=places&TILEMATRIX=2&TILEROW=1&TILECOL=3&FORMAT=image/png" 200
Expect 'places MVT z2'            "$Base/mvt/places/2/3/1.pbf" 200
Expect 'places WFS GetFeature'    "$Base/wfs?SERVICE=WFS&VERSION=1.1.0&REQUEST=GetFeature&TYPENAME=places&MAXFEATURES=2" 200
Expect 'places OAF items'         "$Base/ogc/features/v1/collections/places/items?limit=2" 200

# Raster archive: WMTS/XYZ direct read is 200; WMS and MVT are wrong endpoints -> 400.
Expect 'test XYZ z0'              "$Base/tiles/test/0/0/0.png" 200
Expect 'test WMTS z0'             "$WMTS&LAYER=test&TILEMATRIX=0&TILEROW=0&TILECOL=0&FORMAT=image/png" 200
Expect 'test WMS (no raster src)' "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=test&SRS=EPSG:4326&BBOX=-180,-90,180,90&WIDTH=64&HEIGHT=64&FORMAT=image/png" 400
Expect 'test MVT (not vector)'    "$Base/mvt/test/0/0/0.pbf" 400

# Vector archive: only /mvt accepts it; raster endpoints must reject, never pass pbf through with 200.
Expect 'test-vector MVT z0'       "$Base/mvt/test-vector/0/0/0.pbf" 200
Expect 'test-vector XYZ (reject)' "$Base/tiles/test-vector/0/0/0.png" 400
Expect 'test-vector WMTS (reject)' "$WMTS&LAYER=test-vector&TILEMATRIX=0&TILEROW=0&TILECOL=0&FORMAT=image/png" 400
Expect 'test-vector WMS (reject)' "$Base/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=test-vector&SRS=EPSG:4326&BBOX=-180,-90,180,90&WIDTH=64&HEIGHT=64&FORMAT=image/png" 400

# No tile at this address -> 404 (clients should stop retrying), not 500.
Expect 'wh COG XYZ z10 (render)'  "$Base/tiles/wh/10/803/437.png" 200
Expect 'test XYZ missing tile'    "$Base/tiles/test/12/2048/2048.png" 404
Expect 'test WMTS missing tile'   "$WMTS&LAYER=test&TILEMATRIX=12&TILEROW=2048&TILECOL=2048&FORMAT=image/png" 404
Expect 'test-vector MVT missing'  "$Base/mvt/test-vector/12/2048/2048.pbf" 404
Expect 'OAT missing tile'         "$Base/ogc/tiles/v1/collections/test/tiles/WebMercatorQuad/12/2048/2048" 404
Expect 'OAT z0'                   "$Base/ogc/tiles/v1/collections/test/tiles/WebMercatorQuad/0/0/0" 200
Expect 'COG z26 above matrix'     "$Base/tiles/wh/26/1/1.png" 404
Expect 'unknown layer'            "$Base/tiles/nope/0/0/0.png" 404
Expect 'unknown grid'             "$Base/tiles/wh/NoSuchGrid/0/0/0.png" 404

# COG layer requesting MVT / PostGIS layer requesting raster tiles: mismatched endpoints -> 400.
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
