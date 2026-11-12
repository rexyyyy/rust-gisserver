"""WFS + OGC API-Features + OGC API-Tiles probe.

Builds a catalog (vector places + cog world + mbtiles test-vector) and probes:
- WFS: GetCapabilities (1.0/1.1/2.0 negotiation), DescribeFeatureType XSD,
  GetFeature in GML 3.2 / GeoJSON / CSV, count/startIndex/bbox/featureID/
  propertyName/resultType=hits, namespace-prefix TYPENAME, CRS:84 axis order,
  error semantics (unknown type / unknown request).
- OGC API - Features: landing, conformance, api, collections, collection,
  items (limit/offset/bbox pagination + numberMatched), item, 404s, HTML.
- OGC API - Tiles: landing, conformance, tileMatrixSets (+WebMercatorQuad),
  collections, collection, tiles, tileset, and real tile bytes for a vector
  (MVT) and a raster (PNG) collection.
- WMS + SLD: capabilities advertise GetLegendGraphic / UserDefinedSymbolization;
  GetMap through the SLD styled path (multi-rule marks/labels/dashes), byte
  determinism, GetLegendGraphic PNG (+ rule filter + error semantics),
  vector GetFeatureInfo hit-testing (text/plain + application/json).

Usage: python tools/wfs_ogcapi_probe.py
"""
import http.client
import json
import math
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "target" / "release" / "map-api.exe"
HOST, PORT = "127.0.0.1", 18086
CAT = Path("E:/tmp/rms-wfs")

results = []


def req(path, headers=None):
    c = http.client.HTTPConnection(HOST, PORT, timeout=15)
    c.request("GET", path, headers=headers or {})
    r = c.getresponse()
    b = r.read()
    h = {k.lower(): v for k, v in r.getheaders()}
    c.close()
    return r.status, h, b


def check(name, cond, detail=""):
    results.append((name, bool(cond), detail))
    print(f"{'OK  ' if cond else 'FAIL'} {name:44s} {detail}")


def prepare_catalog():
    if CAT.exists():
        shutil.rmtree(CAT, ignore_errors=True)
    for sub in ("cog", "vector", "mbtiles"):
        (CAT / sub).mkdir(parents=True, exist_ok=True)
    # Vector layer with ABSOLUTE fixture paths (works from any cwd).
    (CAT / "vector/places.yaml").write_text(
        "name: places\n"
        "title: Sample places (GeoJSON)\n"
        "source:\n"
        "  type: vector\n"
        f"  path: {ROOT / 'fixtures/sample.geojson'}\n"
        "src_crs: EPSG:4326\n"
        "grids: [WebMercatorQuad]\n"
        "tile_px: 256\n",
        encoding="utf-8",
    )
    # Colocated SLD (auto-discovered): pop>8M -> star + name label, else
    # circle; polygon fill; dashed line. Drives the WMS styled path.
    (CAT / "vector/places.sld").write_text(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
        "<StyledLayerDescriptor version=\"1.1.0\"\n"
        "    xmlns=\"http://www.opengis.net/sld\"\n"
        "    xmlns:se=\"http://www.opengis.net/se\"\n"
        "    xmlns:ogc=\"http://www.opengis.net/ogc\">\n"
        "  <NamedLayer>\n"
        "    <se:Name>places</se:Name>\n"
        "    <UserStyle>\n"
        "      <se:FeatureTypeStyle>\n"
        "        <se:Rule>\n"
        "          <se:Name>big</se:Name>\n"
        "          <ogc:Filter>\n"
        "            <ogc:PropertyIsGreaterThan>\n"
        "              <ogc:PropertyName>pop</ogc:PropertyName>\n"
        "              <ogc:Literal>8000000</ogc:Literal>\n"
        "            </ogc:PropertyIsGreaterThan>\n"
        "          </ogc:Filter>\n"
        "          <se:PointSymbolizer>\n"
        "            <se:Graphic>\n"
        "              <se:Mark>\n"
        "                <se:WellKnownName>star</se:WellKnownName>\n"
        "                <se:Fill><se:SvgParameter name=\"fill\">#ffcc00</se:SvgParameter></se:Fill>\n"
        "                <se:Stroke><se:SvgParameter name=\"stroke\">#7a5200</se:SvgParameter></se:Stroke>\n"
        "              </se:Mark>\n"
        "              <se:Size>14</se:Size>\n"
        "            </se:Graphic>\n"
        "          </se:PointSymbolizer>\n"
        "          <se:TextSymbolizer>\n"
        "            <se:Label><ogc:PropertyName>name</ogc:PropertyName></se:Label>\n"
        "            <se:Font><se:SvgParameter name=\"font-size\">11</se:SvgParameter>\n"
        "              <se:SvgParameter name=\"font-weight\">bold</se:SvgParameter></se:Font>\n"
        "            <se:Halo><se:Radius>1.5</se:Radius>\n"
        "              <se:Fill><se:SvgParameter name=\"fill\">#ffffff</se:SvgParameter></se:Fill></se:Halo>\n"
        "            <se:Fill><se:SvgParameter name=\"fill\">#1a1a1a</se:SvgParameter></se:Fill>\n"
        "          </se:TextSymbolizer>\n"
        "        </se:Rule>\n"
        "        <se:Rule>\n"
        "          <se:Name>small</se:Name>\n"
        "          <ogc:Filter>\n"
        "            <ogc:PropertyIsLessThanOrEqualTo>\n"
        "              <ogc:PropertyName>pop</ogc:PropertyName>\n"
        "              <ogc:Literal>8000000</ogc:Literal>\n"
        "            </ogc:PropertyIsLessThanOrEqualTo>\n"
        "          </ogc:Filter>\n"
        "          <se:PointSymbolizer>\n"
        "            <se:Graphic>\n"
        "              <se:Mark><se:WellKnownName>circle</se:WellKnownName>\n"
        "                <se:Fill><se:SvgParameter name=\"fill\">#3388cc</se:SvgParameter></se:Fill>\n"
        "              </se:Mark>\n"
        "              <se:Size>9</se:Size>\n"
        "            </se:Graphic>\n"
        "          </se:PointSymbolizer>\n"
        "        </se:Rule>\n"
        "      </se:FeatureTypeStyle>\n"
        "      <se:FeatureTypeStyle>\n"
        "        <se:Rule><se:Name>area</se:Name>\n"
        "          <se:PolygonSymbolizer>\n"
        "            <se:Fill><se:SvgParameter name=\"fill\">#2d6edc</se:SvgParameter>\n"
        "              <se:SvgParameter name=\"fill-opacity\">0.3</se:SvgParameter></se:Fill>\n"
        "            <se:Stroke><se:SvgParameter name=\"stroke\">#123a70</se:SvgParameter>\n"
        "              <se:SvgParameter name=\"stroke-width\">1.5</se:SvgParameter></se:Stroke>\n"
        "          </se:PolygonSymbolizer>\n"
        "        </se:Rule>\n"
        "        <se:Rule><se:Name>corridor</se:Name>\n"
        "          <se:LineSymbolizer><se:Stroke>\n"
        "            <se:SvgParameter name=\"stroke\">#c0399b</se:SvgParameter>\n"
        "            <se:SvgParameter name=\"stroke-width\">2.5</se:SvgParameter>\n"
        "            <se:SvgParameter name=\"stroke-dasharray\">6 3</se:SvgParameter>\n"
        "          </se:Stroke></se:LineSymbolizer>\n"
        "        </se:Rule>\n"
        "      </se:FeatureTypeStyle>\n"
        "    </UserStyle>\n"
        "  </NamedLayer>\n"
        "</StyledLayerDescriptor>\n",
        encoding="utf-8",
    )
    (CAT / "cog/world.yaml").write_text(
        "name: world\n"
        "title: Sample world COG\n"
        "source:\n"
        "  type: cog\n"
        f"  path: {ROOT / 'fixtures/sample.tif'}\n"
        "src_crs: EPSG:4326\n"
        "grids: [WebMercatorQuad]\n"
        "tile_px: 256\n",
        encoding="utf-8",
    )
    (CAT / "mbtiles/test-vector.yaml").write_text(
        "name: test-vector\n"
        "title: Test vector tiles\n"
        "source:\n"
        "  type: mbtiles\n"
        f"  path: {ROOT / 'fixtures/test-vector.mbtiles'}\n"
        "src_crs: EPSG:4326\n"
        "grids: [WebMercatorQuad]\n"
        "tile_px: 256\n",
        encoding="utf-8",
    )
    (CAT / "mapserver.yaml").write_text(
        "server:\n  host: 127.0.0.1\n  port: %d\n  cache_mib: 64\n"
        "catalog:\n  root: %s\n  backend: files\n" % (PORT, str(CAT).replace("\\", "/")),
        encoding="utf-8",
    )


def wait_up(proc, timeout_s=10):
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        try:
            c = http.client.HTTPConnection(HOST, PORT, timeout=2)
            c.request("GET", "/health")
            ok = c.getresponse().status == 200
            c.close()
            if ok:
                return True
        except OSError:
            pass
        if proc.poll() is not None:
            return False
        time.sleep(0.2)
    return False


def probe_wfs():
    print("--- WFS ---")
    s, h, b = req("/wfs?service=WFS&request=GetCapabilities")
    x = b.decode("utf-8", "replace")
    check("GetCapabilities 2.0.0 default", s == 200 and 'version="2.0.0"' in x, f"HTTP {s}")
    check("caps lists places FeatureType", "<wfs:Name>places</wfs:Name>" in x)
    check("caps WGS84 bounds", "<ows:WGS84BoundingBox>" in x)
    check("caps GetFeature operation", '<ows:Operation name="GetFeature">' in x)

    s, h, b = req("/wfs?service=WFS&request=GetCapabilities&acceptVersions=1.1.0,2.0.0")
    check("AcceptVersions picks 1.1.0", s == 200 and 'version="1.1.0"' in b.decode())
    s, h, b = req("/wfs?service=WFS&request=GetCapabilities&version=1.0.0")
    check("version=1.0.0 legacy caps", s == 200 and 'version="1.0.0"' in b.decode())

    s, h, b = req("/wfs?service=WFS&request=DescribeFeatureType&version=2.0.0&typeName=places")
    x = b.decode("utf-8", "replace")
    check("DescribeFeatureType XSD", s == 200 and "xsd:schema" in x and 'name="places"' in x)
    check("DFT geometry typed", "gml:PointPropertyType" in x)
    check("DFT attributes typed", 'name="name" type="xs:string"' in x and 'name="pop" type="xs:integer"' in x)

    # GML (default output)
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places&count=2")
    x = b.decode("utf-8", "replace")
    check("GetFeature GML 200", s == 200 and "gml+xml" in h.get("content-type", ""), f"HTTP {s}")
    check("GML numberMatched/numberReturned", 'numberMatched=' in x and 'numberReturned="2"' in x)
    check("GML member + gml:id", "<wfs:member>" in x and 'gml:id="places.' in x)
    # Default srsName is urn:ogc:def:crs:EPSG::4326 -> EPSG axis order
    # (lat lon) per EPSG:4326 axis order. CRS:84 would be lon/lat.
    check(
        "GML pos encoding",
        'srsName="urn:ogc:def:crs:EPSG::4326"' in x
        and "<gml:pos>24.8801 102.8329</gml:pos>" in x,
        "urn srsName + lat/lon axis order",
    )
    check("GML attribute element", "<ws:name>Kunming</ws:name>" in x)

    # GeoJSON
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&outputFormat=application/json&count=1")
    try:
        d = json.loads(b)
        ok = s == 200 and d["type"] == "FeatureCollection" and len(d["features"]) == 1 \
            and d["numberMatched"] > 1
    except Exception as e:
        ok, d = False, {"err": str(e)}
    check("GetFeature GeoJSON", ok, json.dumps(d)[:80])

    # CSV
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&outputFormat=text/csv&count=1")
    x = b.decode("utf-8", "replace")
    check("GetFeature CSV", s == 200
          and set(x.splitlines()[0].split(",")) == {"name", "kind", "pop"}
          and "Kunming" in x, x[:40])

    # hits
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&resultType=hits")
    x = b.decode("utf-8", "replace")
    check("resultType=hits empty members", s == 200 and 'numberReturned="0"' in x
          and "<wfs:member>" not in x and 'numberMatched="' in x)

    # bbox filter (Kunming only): 102.0,24.0,103.0,25.0 lat-first for 4326
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&bbox=24.0,102.0,25.0,103.0")
    x = b.decode("utf-8", "replace")
    check("bbox lat-first 4326 filter", '<ws:name>Kunming</ws:name>' in x
          and '<ws:name>Chengdu</ws:name>' not in x and 'numberMatched="3"' in x)

    # CRS:84 lon/lat must NOT swap: 102,24,103,25 -> Kunming in, Chengdu out
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&srsName=CRS:84&bbox=102,24,103,25")
    x = b.decode("utf-8", "replace")
    check("CRS:84 lon/lat not swapped", '<ws:name>Kunming</ws:name>' in x
          and '<ws:name>Chengdu</ws:name>' not in x)

    # featureID
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&featureID=f0")
    x = b.decode("utf-8", "replace")
    check("featureID selects one", '<ws:name>Kunming</ws:name>' in x
          and 'numberReturned="1"' in x)

    # propertyName projection
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                  "&outputFormat=application/json&propertyName=pop&count=1")
    d = json.loads(b)
    props = d["features"][0]["properties"]
    check("propertyName projection", set(props.keys()) == {"pop"}, str(props))

    # startIndex: feature at offset 1 must differ from the one at offset 0.
    s0, _, b0 = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                    "&outputFormat=application/json&count=1")
    s1, _, b1 = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=places"
                    "&outputFormat=application/json&startIndex=1&count=1")
    d0, d1 = json.loads(b0), json.loads(b1)
    n0 = d0["features"][0]["properties"].get("name")
    n1 = d1["features"][0]["properties"].get("name")
    check("startIndex skips first", s0 == 200 and s1 == 200 and n1 is not None and n1 != n0,
          f"{n0} -> {n1}")

    # namespace-prefix TYPENAME tolerated
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=ws:places&count=1")
    check("TYPENAME namespace prefix", s == 200 and "wfs:FeatureCollection" in b.decode())

    # errors
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0&typeName=nope")
    check("unknown typeName 400", s == 400 and "InvalidParameterValue" in b.decode())
    s, h, b = req("/wfs?service=WFS&request=Frobnicate")
    check("unknown request 400", s == 400 and "OperationNotSupported" in b.decode())
    s, h, b = req("/wfs?service=WFS&request=GetFeature&version=2.0.0")
    check("missing typeName 400", s == 400 and "MissingParameterValue" in b.decode())


def probe_oaf():
    print("--- OGC API - Features ---")
    s, h, b = req("/ogc/features/v1")
    d = json.loads(b)
    check("landing page", s == 200 and d["title"].startswith("rust-gisserver"))
    rels = [l["rel"] for l in d["links"]]
    check("landing links", {"self", "conformance", "data", "service-desc"} <= set(rels), str(rels))

    s, h, b = req("/ogc/features/v1/conformance")
    cs = json.loads(b)["conformsTo"]
    check("conformance classes", any(c.endswith("/conf/core") for c in cs)
          and any(c.endswith("/conf/geojson") for c in cs))

    s, h, b = req("/ogc/features/v1/api")
    check("api doc is OpenAPI 3.0", s == 200 and json.loads(b)["openapi"].startswith("3.0"))

    s, h, b = req("/ogc/features/v1/collections")
    d = json.loads(b)
    check("collections list", s == 200 and d["collections"][0]["id"] == "places")
    check("collection extent", d["collections"][0]["extent"]["spatial"]["bbox"][0][0] < 103)

    s, h, b = req("/ogc/features/v1/collections/places")
    d = json.loads(b)
    check("collection metadata", d["id"] == "places" and d["itemType"] == "feature"
          and any("def/crs/EPSG/0/4326" in c for c in d["crs"]))

    s, h, b = req("/ogc/features/v1/collections/places/items")
    d = json.loads(b)
    check("items default limit 10", d["type"] == "FeatureCollection"
          and d["numberReturned"] <= 10 and d["numberMatched"] >= d["numberReturned"])
    check("items links self/collection", {"self", "collection"} <= {l["rel"] for l in d["links"]})
    s, h, b = req("/ogc/features/v1/collections/places/items?limit=1")
    d = json.loads(b)
    check("items next link", d["numberReturned"] == 1
          and any(l["rel"] == "next" for l in d["links"]))

    s, h, b = req("/ogc/features/v1/collections/places/items?limit=2&offset=1")
    d = json.loads(b)
    check("items pagination", d["numberReturned"] == 2 and any(l["rel"] == "prev" for l in d["links"]))

    s, h, b = req("/ogc/features/v1/collections/places/items?bbox=102,24,103,25")
    d = json.loads(b)
    names = {f["properties"].get("name") for f in d["features"]}
    check("items bbox filter", "Kunming" in names and "Chengdu" not in names
          and d["numberMatched"] == 3, str(names))

    s, h, b = req("/ogc/features/v1/collections/places/items/f0")
    d = json.loads(b)
    check("item by id", d["type"] == "Feature" and d["properties"]["name"] == "Kunming")

    s, h, b = req("/ogc/features/v1/collections/nope/items")
    check("unknown collection 404 JSON", s == 404 and json.loads(b)["code"] == "NotFound")
    s, h, b = req("/ogc/features/v1/collections/places/items/zzz")
    check("unknown feature 404 JSON", s == 404 and json.loads(b)["code"] == "NotFound")
    s, h, b = req("/ogc/features/v1/collections/places/items?limit=abc")
    check("bad limit 400", s == 400 and json.loads(b)["code"] == "InvalidParameterValue")

    s, h, b = req("/ogc/features/v1", {"Accept": "text/html"})
    check("HTML landing", s == 200 and "text/html" in h.get("content-type", "")
          and b"<html" in b)
    s, h, b = req("/ogc/features/v1/collections/places/items", {"Accept": "text/html"})
    check("HTML items", s == 200 and "text/html" in h.get("content-type", "")
          and b"Kunming" in b)


def probe_oat():
    print("--- OGC API - Tiles ---")
    s, h, b = req("/ogc/tiles/v1")
    d = json.loads(b)
    check("landing page", s == 200 and "OGC API - Tiles" in d["title"])
    rels = [l["rel"] for l in d["links"]]
    check("landing links", {"tileMatrixSets", "data", "conformance"} <= set(rels))

    s, h, b = req("/ogc/tiles/v1/conformance")
    cs = json.loads(b)["conformsTo"]
    check("conformance tileset class", any(c.endswith("/conf/tileset") for c in cs))

    s, h, b = req("/ogc/tiles/v1/tileMatrixSets")
    ids = [t["id"] for t in json.loads(b)["tileMatrixSets"]]
    check("tileMatrixSets presets", "WebMercatorQuad" in ids and "WorldCRS84Quad" in ids, str(ids))

    s, h, b = req("/ogc/tiles/v1/tileMatrixSets/WebMercatorQuad")
    d = json.loads(b)
    check("TMS definition shape", d["crs"] == "http://www.opengis.net/def/crs/EPSG/0/3857"
          and len(d["tileMatrices"]) == 26
          and d["tileMatrices"][0]["topLeftCorner"] == [-20037508.3427892, 20037508.3427892])
    check("TMS z25 matrix size", d["tileMatrices"][25]["matrixWidth"] == 1 << 25)

    s, h, b = req("/ogc/tiles/v1/tileMatrixSets/NopeQuad")
    check("unknown TMS 404", s == 404)

    s, h, b = req("/ogc/tiles/v1/collections")
    d = json.loads(b)
    ids = {c["id"]: c for c in d["collections"]}
    check("tiled collections", {"places", "world", "test-vector"} <= set(ids), str(set(ids)))
    check("vector collection dataType", ids["places"]["dataType"] == "vector")
    check("map collection dataType", ids["world"]["dataType"] == "map")

    s, h, b = req("/ogc/tiles/v1/collections/places")
    d = json.loads(b)
    check("collection tileset links", any(l["rel"] == "tiles" for l in d["links"]))

    s, h, b = req("/ogc/tiles/v1/collections/places/tiles")
    d = json.loads(b)
    check("tilesets list", d["tiles"][0]["tileMatrixSet"] == "WebMercatorQuad")

    s, h, b = req("/ogc/tiles/v1/collections/places/tiles/WebMercatorQuad")
    d = json.loads(b)
    check("tileset metadata", d["dataType"] == "vector"
          and d["format"] == "application/vnd.mapbox-vector-tile"
          and d["maxZoomLevel"] == 25 and "{tileMatrix}" in d["templates"][0])

    s, h, b = req("/ogc/tiles/v1/collections/world/tiles/WebMercatorQuad")
    check("raster tileset png", json.loads(b)["format"] == "image/png")

    # Real tile bytes: vector (MVT) around Kunming z6, raster (PNG) z3.
    z = 6
    x = int((102.8329 + 180) / 360 * 2 ** z)
    la = math.radians(24.8801)
    y = int((1 - math.log(math.tan(la) + 1 / math.cos(la)) / math.pi) / 2 * 2 ** z)
    s, h, b = req(f"/ogc/tiles/v1/collections/places/tiles/WebMercatorQuad/{z}/{y}/{x}")
    check("OAT vector tile bytes", s == 200 and len(b) > 100
          and ("mapbox-vector-tile" in h.get("content-type", "")
               or "x-protobuf" in h.get("content-type", "")), f"{len(b)}B")
    s, h, b = req("/ogc/tiles/v1/collections/world/tiles/WebMercatorQuad/3/3/6")
    check("OAT raster tile bytes", s == 200 and b[:8] == b"\x89PNG\r\n\x1a\n", f"{len(b)}B")
    s, h, b = req("/ogc/tiles/v1/collections/world/tiles/WebMercatorQuad/3/3/6.png")
    check("OAT tile tolerates .png suffix", s == 200 and b[:8] == b"\x89PNG\r\n\x1a\n")
    s, h, b = req("/ogc/tiles/v1/collections/places/tiles/WebMercatorQuad/3/99/99")
    check("out-of-range tile 404", s == 404)
    s, h, b = req("/ogc/tiles/v1/collections/nope/tiles/WebMercatorQuad/3/3/6")
    check("unknown collection tile 404", s == 404)
    s, h, b = req("/ogc/tiles/v1/collections/places/tiles/NopeQuad/3/3/6")
    check("unpublished grid tile 404", s == 404)


def probe_wms_sld():
    print("--- WMS + SLD ---")
    PNG = b"\x89PNG\r\n\x1a\n"

    s, h, b = req("/wms?service=WMS&request=GetCapabilities&version=1.3.0")
    x = b.decode("utf-8", "replace")
    check("caps GetLegendGraphic request", s == 200 and "<GetLegendGraphic>" in x, f"HTTP {s}")
    check("caps UserDefinedSymbolization", "UserDefinedSymbolization" in x and 'SupportSLD="1"' in x)
    check("caps GetFeatureInfo json format", "<Format>application/json</Format>" in x)

    # GetMap through the SLD styled path (star/circle marks + labels + polygon
    # fill + dashed line all land in one frame).
    q = ("/wms?service=WMS&version=1.1.1&request=GetMap&layers=places"
         "&bbox=100,24,104,26&width=256&height=256&crs=EPSG:4326"
         "&format=image/png&styles=")
    s, h, b = req(q)
    check("GetMap SLD styled PNG", s == 200 and b[:8] == PNG and len(b) > 800, f"HTTP {s} {len(b)}B")
    s2, h2, b2 = req(q)
    check("GetMap deterministic bytes", b == b2 and s2 == 200)
    etag = h.get("etag", "")
    s3, h3, b3 = req(q, headers={"If-None-Match": etag})
    check("GetMap ETag 304", s3 == 304, f"HTTP {s3}")

    # A tile through the same SLD path (z4 x12 y6 covers Kunming).
    s, h, b = req("/tiles/places/4/12/6.png")
    check("SLD tile PNG", s == 200 and b[:8] == PNG, f"HTTP {s}")

    # GetLegendGraphic
    s, h, b = req("/wms?service=WMS&version=1.1.1&request=GetLegendGraphic&layer=places&format=image/png")
    check("GetLegendGraphic PNG", s == 200 and b[:8] == PNG and len(b) > 200, f"HTTP {s} {len(b)}B")
    s, h, b = req("/wms?service=WMS&version=1.1.1&request=GetLegendGraphic&layer=places&rule=big")
    check("GetLegendGraphic rule filter", s == 200 and b[:8] == PNG, f"HTTP {s}")
    s, h, b = req("/wms?service=WMS&version=1.1.1&request=GetLegendGraphic&layer=places&rule=nope")
    check("GetLegendGraphic unknown rule placeholder", s == 200 and b[:8] == PNG, f"HTTP {s}")
    s, h, b = req("/wms?service=WMS&version=1.1.1&request=GetLegendGraphic&layer=places&format=image/jpeg")
    check("GetLegendGraphic bad format 400", s == 400 and "InvalidFormat" in b.decode("utf-8", "replace"), f"HTTP {s}")
    s, h, b = req("/wms?service=WMS&version=1.1.1&request=GetLegendGraphic&layer=nope")
    check("GetLegendGraphic unknown layer 400", s == 400 and "LayerNotDefined" in b.decode("utf-8", "replace"), f"HTTP {s}")

    # Vector GetFeatureInfo: Kunming (102.8329, 24.8801) in frame
    # bbox=100,24,104,26 at 256x256 -> pixel (181, 143).
    base = ("/wms?service=WMS&version=1.1.1&request=GetFeatureInfo&layers=places"
            "&query_layers=places&bbox=100,24,104,26&width=256&height=256"
            "&crs=EPSG:4326&x=181&y=143")
    s, h, b = req(base + "&info_format=text/plain")
    check("GetFeatureInfo vector text/plain", s == 200 and "name = Kunming" in b.decode("utf-8", "replace"), f"HTTP {s}")
    s, h, b = req(base + "&info_format=application/json")
    ok = False
    try:
        d = json.loads(b)
        ok = s == 200 and d.get("name") == "Kunming"
    except Exception:
        pass
    check("GetFeatureInfo vector json", ok, f"HTTP {s}")
    s, h, b = req(base + "&info_format=application/vnd.ogc.gml")
    check("GetFeatureInfo bad INFO_FORMAT 400", s == 400 and "InvalidFormat" in b.decode("utf-8", "replace"), f"HTTP {s}")
    # No feature under the click: 200 with empty content
    # (the fixture polygon covers the whole Kunming frame, so probe an empty
    # bbox east of every feature instead).
    empty = ("/wms?service=WMS&version=1.1.1&request=GetFeatureInfo&layers=places"
             "&query_layers=places&bbox=113,25,117,29&width=256&height=256"
             "&crs=EPSG:4326&x=5&y=5&info_format=text/plain")
    s, h, b = req(empty)
    check("GetFeatureInfo no hit 200 empty", s == 200 and b"name" not in b, f"HTTP {s} {b[:40]!r}")
    # Raster layer keeps the sampling path.
    s, h, b = req("/wms?service=WMS&version=1.1.1&request=GetFeatureInfo&layers=world"
                  "&query_layers=world&bbox=100,36,104,40&width=256&height=256"
                  "&crs=EPSG:4326&x=128&y=128")
    check("GetFeatureInfo raster sampling", s == 200 and "value = " in b.decode("utf-8", "replace"), f"HTTP {s}")


def main():
    if not EXE.exists():
        sys.exit(f"missing {EXE} — build first")
    prepare_catalog()
    proc = subprocess.Popen(
        [str(EXE), "serve", "--config", str(CAT / "mapserver.yaml")],
        stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT,
    )
    try:
        if not wait_up(proc):
            sys.exit("server did not come up")
        probe_wfs()
        probe_oaf()
        probe_oat()
        probe_wms_sld()
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()
    failed = [n for n, ok, _ in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} passed")
    if failed:
        print("FAILED:", ", ".join(failed))
        sys.exit(1)


if __name__ == "__main__":
    main()
