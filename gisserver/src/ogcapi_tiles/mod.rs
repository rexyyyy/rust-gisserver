//! OGC API - Tiles - Part 1 (1.0.0): landing page, conformance declaration,
//! tileMatrixSets, tiled collections and tileset metadata.
//!
//! Path layout follows GeoServer's `ogcapi-tiles` module: landing at the mount
//! point (`/ogc/tiles/v1`), `tileMatrixSets[/{id}]`, `collections[/{id}]` and
//! tiles under `collections/{collectionId}/tiles/{tileMatrixSetId}`. The tile
//! bytes themselves are served by the map-api service (which reuses the WMS
//! tile renderer); this crate only builds the metadata documents.

use serde_json::{Value, json};
use crate::wms_common::crs::meters_per_unit;
use crate::wms_common::tile::{TileMatrixSet, is_preset_id, strip_size_suffix};

// ── 分辨率口径：把同一 LOD 的三种数值「分开定义」─────────────────────────────
//
// 一个金字塔层级在不同规范里会导出三个互不相等的「分辨率」：
//   native      — 缓存/网格自身的 LOD 分辨率，单位 = CRS 单位/像素（地理系=度、
//                 投影系=米）。`/tiles/{z}/{col}/{row}` 的行列完全由
//                 origin + resolution * tile_px 决定，只有它不会错位。
//   ogc28mm     — 按 OGC 0.28 mm/像素从 ScaleDenominator 反算；OL 的
//                 `optionsFromCapabilities` 及多数 WMTS 客户端是这个口径。
//   arcgis96dpi — 按 ArcGIS `0.0254 / DPI`（96 DPI）从 ScaleDenominator 反算；
//                 Conf.xml 的 `LODInfo/Scale` 正是这样导出的。
// `0.00028 / (0.0254/96) ≈ 1.0583`：对 ArcGIS 缓存用 ogc28mm 会把分辨率放大
// 5.83%、瓦片行列整体偏移；对普通网格（Scale 本就由 0.28mm 推出）反而是
// arcgis96dpi 偏小。因此三者分开下发、由客户端显式选择，默认恒为 native。
pub const RES_MODE_NATIVE: &str = "native";
pub const RES_MODE_OGC_28MM: &str = "ogc28mm";
pub const RES_MODE_ARCGIS_96DPI: &str = "arcgis96dpi";
pub const OGC_PIXEL_SIZE_M: f64 = 0.00028;
pub const ARCGIS_DPI: f64 = 96.0;
pub const ARCGIS_PIXEL_SIZE_M: f64 = 0.0254 / ARCGIS_DPI;
pub const RES_MODES: &[&str] = &[
    RES_MODE_NATIVE,
    RES_MODE_OGC_28MM,
    RES_MODE_ARCGIS_96DPI,
];

/// 三种口径的分辨率（CRS 单位/像素）。`scale` 为源声明的比例尺分母，
/// `mpu` 为该 CRS 每单位对应的米数（地理系 111319.49…，投影系 1）。
fn resolution_variants(native: f64, scale: f64, mpu: f64) -> Value {
    json!({
        "native": native,
        "ogc28mm": scale * OGC_PIXEL_SIZE_M / mpu,
        "arcgis96dpi": scale * ARCGIS_PIXEL_SIZE_M / mpu,
    })
}

/// Conformance classes this implementation claims (OGC API - Tiles 1.0.0).
pub const CONFORMANCE: &[&str] = &[
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/oas30",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tileset",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tilesets-list",
    "http://www.opengis.net/spec/ogcapi-tiles-1/1.0/conf/tileMatrixSet",
];

/// Map tile media types advertised for map tilesets.
pub const MAP_FORMATS: &[&str] = &["image/png"];
/// Vector tile media types advertised for vector tilesets.
pub const VECTOR_FORMATS: &[&str] = &["application/vnd.mapbox-vector-tile"];

/// A tiled collection: one tile-serving layer (cog / vector / mbtiles / pmtiles).
#[derive(Clone, Debug)]
pub struct TiledCollection {
    pub id: String,
    pub title: String,
    /// "map" (raster tiles) or "vector" (MVT).
    pub data_type: &'static str,
    /// Grid ids the layer publishes (e.g. `WebMercatorQuad`).
    pub tile_matrix_sets: Vec<String>,
    /// Tile formats for the tileset.
    pub formats: &'static [&'static str],
    /// Data extent in WGS84 lon/lat.
    pub extent: Option<[f64; 4]>,
    /// First/last zoom of the default grid.
    pub zoom_range: Option<(u32, u32)>,
}

/// EPSG code as an OGC CRS URI (`http://www.opengis.net/def/crs/EPSG/0/3857`).
pub fn crs_uri(crs: &str) -> String {
    let code = crs.trim_start_matches("EPSG:");
    format!("http://www.opengis.net/def/crs/EPSG/0/{code}")
}

/// 网格的 CRS URI，需与 WMTS 能力文档的 `<SupportedCRS>` 同口径（见
/// `wms-protocol::wmts::supported_crs`）：`WorldCRS84Quad` 的 `topLeftCorner`
/// 与 `orderedAxes` 都是经度在前，那就不该挂 EPSG:4326（注册表轴序=纬度在
/// 前），只看 `crs` 不读 `orderedAxes` 的客户端会据此交换坐标。
/// 投影网格与自定义地理网格照旧给 EPSG def-URI。
pub fn grid_crs_uri(g: &TileMatrixSet) -> String {
    if g.id.starts_with("WorldCRS84Quad") {
        return "http://www.opengis.net/def/crs/OGC/1.3/CRS84".into();
    }
    crs_uri(&g.crs)
}

/// OGC 正式登记的 TileMatrixSet URI，仅对标准/内置网格给出（去尺寸尾缀）。
/// 自定义网格（`arcgis_*`、`custom_grids:` 等）返回 `None`——服务端不拥有其
/// OGC 注册，硬拼一个 `.../spec/tms/2.0/.../<id>` 会指向 404，属「说谎元数据」。
/// 与 WMTS 能力文档只在内置网格写 `<TileMatrixSetURI>` 保持同一口径。
fn preset_ogc_uri(id: &str) -> Option<String> {
    if is_preset_id(id) {
        Some(format!(
            "http://www.opengis.net/def/tilematrixset/OGC/1.0/{}",
            strip_size_suffix(id)
        ))
    } else {
        None
    }
}

fn links(base: &str, extra: Vec<(&str, String)>) -> Vec<Value> {
    let mut v = vec![json!({"rel": "self", "href": base, "type": "application/json"})];
    for (rel, href) in extra {
        v.push(json!({"rel": rel, "href": href, "type": "application/json"}));
    }
    v
}

/// `GET {base}` — the landing page.
pub fn landing(base: &str) -> String {
    let doc = json!({
        "title": "rust-gisserver OGC API - Tiles",
        "description": "Map and vector tile access over the rust-gisserver catalog.",
        "links": links(base, vec![
            ("service-desc", format!("{base}/api")),
            ("conformance", format!("{base}/conformance")),
            ("tileMatrixSets", format!("{base}/tileMatrixSets")),
            ("data", format!("{base}/collections")),
        ]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/conformance`.
pub fn conformance() -> String {
    let doc = json!({ "conformsTo": CONFORMANCE });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/tileMatrixSets` — the list of advertised grids.
pub fn tile_matrix_sets(base: &str, grids: &[TileMatrixSet]) -> String {
    let items: Vec<Value> = grids
        .iter()
        .map(|g| {
            json!({
                "id": g.id,
                "title": g.id,
                // 列表项也带类型标记，客户端不必逐个拉取即可区分标准/自定义。
                "standard": is_preset_id(&g.id),
                // `links()` 自带 self，extra 里不能再写同 rel 链接（OGC API 要求
                // 同一文档内每个 rel 唯一，重复 self 会让客户端取到不确定的一条）
                "links": links(&format!("{base}/tileMatrixSets/{}", g.id), vec![]),
            })
        })
        .collect();
    let doc = json!({
        "tileMatrixSets": items,
        "links": links(base, vec![]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/tileMatrixSets/{tileMatrixSetId}` — the full grid definition.
pub fn tile_matrix_set(base: &str, g: &TileMatrixSet, crs_proj4: Option<&str>) -> String {
    let mpu = meters_per_unit(&g.crs);
    let matrices: Vec<Value> = g
        .levels
        .iter()
        .map(|l| {
            let scale = g.scale_denominator(l.z);
            json!({
                "id": l.z.to_string(),
                // 兼容旧客户端：顶层 resolution 恒等于 native（CRS 单位/像素）。
                // 客户端应直接用它建网格，切忌用 scaleDenominator×0.28mm 反算
                // （ArcGIS 0.0254/DPI 与 OGC 0.28mm 口径差 ~5.8%，地理系下还会
                //  把「度」当「米」，瓦片寻址整体错位）。
                "resolution": l.resolution,
                // 三种口径分开下发，供前端按 ?mode= / 面板配置选用（默认 native）。
                "resolutions": resolution_variants(l.resolution, scale, mpu),
                "scaleDenominator": scale,
                "topLeftCorner": [g.origin_x, g.origin_y],
                "tileWidth": g.tile_w,
                "tileHeight": g.tile_h,
                "matrixWidth": l.matrix_w,
                "matrixHeight": l.matrix_h,
            })
        })
        .collect();
    let doc = json!({
        "id": g.id,
        "title": g.id,
        // 显式类型标记：标准/内置网格 = true；自定义网格（ArcGIS Conf.xml 派生
        // 或 config `custom_grids:`，如 EPSG:4522 投影带）= false。客户端/管理端
        // 据此分得清类型，无需再从 id 名字猜。uri 见下方按类型补。
        "standard": is_preset_id(&g.id),
        "crs": grid_crs_uri(g),
        // 本服务的网格一律按 (easting, northing) 书写，与 XML 的 TopLeftCorner 同序
        "orderedAxes": ["X", "Y"],
        // 把 CRS 单位→米的换算一并下发：客户端不必硬编码 111319.49，
        // 也可据此自行按任意 DPI/像素尺寸再推一种口径。
        "metersPerUnit": mpu,
        "resolutionConventions": {
            "default": RES_MODE_NATIVE,
            "modes": RES_MODES,
            "native": {
                "unit": "CRS units / px",
                "pixelSizeM": Value::Null,
                "description": "源 LOD 分辨率，瓦片寻址唯一正确口径",
            },
            "ogc28mm": {
                "pixelSizeM": OGC_PIXEL_SIZE_M,
                "description": "ScaleDenominator × 0.00028 / metersPerUnit（OL / WMTS 客户端口径）",
            },
            "arcgis96dpi": {
                "dpi": ARCGIS_DPI,
                "pixelSizeM": ARCGIS_PIXEL_SIZE_M,
                "description": "ScaleDenominator × (0.0254/96) / metersPerUnit（ArcGIS LODInfo/Scale 口径；实测源 Scale 与 Resolution 有约 0.1% 圆整差，仅近似）",
            },
        },
        "tileMatrices": matrices,
        // `links()` 已经给了一条 self，这里不能再补（上一版会输出两条完全相同的
        // self 链接，违反 OGC API “每个 rel 唯一”的链接约定）
        "links": links(&format!("{base}/tileMatrixSets/{}", g.id), vec![]),
    });
    // 有 PROJ 后端时下发该 CRS 的 proj4 定义串，供前端为任意投影带（如
    // EPSG:4522/45xx/UTM）动态注册坐标系，无需硬编码；无后端则省略此字段，
    // 客户端回退到内置定义。几何(origin/resolutions/matrix)本就在 tileMatrices。
    let mut doc = doc;
    // uri 只给标准/内置网格挂官方 OGC 登记 URI；自定义网格省略（与 WMTS XML 面
    // 只在内置写 <TileMatrixSetURI> 一致），不再对所有网格伪造 spec/tms 命名空间。
    if let Some(uri) = preset_ogc_uri(&g.id) {
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("uri".to_string(), json!(uri));
        }
    }
    if let Some(pj) = crs_proj4.filter(|s| !s.trim().is_empty()) {
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("proj4".to_string(), json!(pj));
        }
    }
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

// 独立命名：本文件末尾已有一个 `mod tests`（landing/conformance/文档形状），
// 同名会触发 E0428；按主题拆成第二个测试模块更清晰。
#[cfg(test)]
mod resolution_mode_tests {
    use super::*;

    /// ArcGIS 口径的 Scale 只能用 arcgis96dpi 反算回原生分辨率；ogc28mm 会
    /// 偏大 5.83% —— 这正是瓦片错位的根源，固化成守卫，防止有人「统一口径」。
    #[test]
    fn arcgis_scale_only_reconstructs_with_arcgis_dpi() {
        let mpu = meters_per_unit("EPSG:4490");
        let native = 0.00137483;
        let scale = native * mpu / ARCGIS_PIXEL_SIZE_M;
        let v = resolution_variants(native, scale, mpu);
        assert!((v["arcgis96dpi"].as_f64().unwrap() - native).abs() < 1e-12);
        let ogc = v["ogc28mm"].as_f64().unwrap();
        assert!((ogc / native - OGC_PIXEL_SIZE_M / ARCGIS_PIXEL_SIZE_M).abs() < 1e-9);
        // 0.00028 × 96 / 0.0254 = 1.0582677165…（≈+5.83%），固化数值防口径被误统一
        assert!((ogc / native - 1.0582677165).abs() < 1e-9);
        // 普通网格的 Scale 本就由 0.28mm 推出 → ogc28mm 还原，arcgis96dpi 偏小
        let scale_ogc = native * mpu / OGC_PIXEL_SIZE_M;
        let v2 = resolution_variants(native, scale_ogc, mpu);
        assert!((v2["ogc28mm"].as_f64().unwrap() - native).abs() < 1e-12);
        assert!(v2["arcgis96dpi"].as_f64().unwrap() < native);
    }

    #[test]
    fn tile_matrix_set_advertises_all_resolution_modes() {
        let mut g = TileMatrixSet::web_mercator_quad(256);
        g.id = "arcgis_probe".into();
        let native = g.levels[9].resolution;
        // 伪装成 ArcGIS 源 Scale（3857 下 metersPerUnit = 1）
        g.levels[9].scale_denominator = Some(native / ARCGIS_PIXEL_SIZE_M);
        let doc: Value =
            serde_json::from_str(&tile_matrix_set("http://x/ogc/tiles/v1", &g, None)).unwrap();
        assert_eq!(
            doc["resolutionConventions"]["default"].as_str(),
            Some(RES_MODE_NATIVE)
        );
        assert_eq!(
            doc["resolutionConventions"]["modes"].as_array().unwrap().len(),
            RES_MODES.len()
        );
        let m = &doc["tileMatrices"][9];
        assert_eq!(m["resolution"], m["resolutions"]["native"]);
        for mode in RES_MODES {
            assert!(m["resolutions"][mode].is_number(), "缺少 {mode} 口径");
        }
        assert!(
            (m["resolutions"][RES_MODE_ARCGIS_96DPI].as_f64().unwrap() - native).abs() < 1e-9
        );
        assert!(m["resolutions"][RES_MODE_OGC_28MM].as_f64().unwrap() > native);
        assert!(doc["metersPerUnit"].as_f64().unwrap() > 0.0);
    }
}

/// `GET {base}/collections` — the tiled collection list.
pub fn collections(base: &str, cols: &[TiledCollection]) -> String {
    let items: Vec<Value> = cols
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "title": c.title,
                "dataType": c.data_type,
                "tileMatrixSets": c.tile_matrix_sets,
                "links": links(&format!("{base}/collections/{}", c.id), vec![
                    ("tiles", format!("{base}/collections/{}/tiles", c.id)),
                ]),
            })
        })
        .collect();
    let doc = json!({
        "collections": items,
        "links": links(base, vec![]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/collections/{collectionId}` — one tiled collection.
pub fn collection(base: &str, c: &TiledCollection) -> String {
    let self_url = format!("{base}/collections/{}", c.id);
    let extent = c.extent.map(|e| json!({
        "spatial": { "bbox": [e], "crs": "http://www.opengis.net/def/crs/OGC/1.3/CRS84" }
    }));
    let mut extra: Vec<(&str, String)> = vec![
        ("collection", format!("{base}/collections")),
        ("tiles", format!("{self_url}/tiles")),
    ];
    for tms in &c.tile_matrix_sets {
        extra.push((
            "tileset",
            format!("{self_url}/tiles/{tms}"),
        ));
    }
    let doc = json!({
        "id": c.id,
        "title": c.title,
        "description": format!("Tiled collection '{}' ({}) served by rust-gisserver.", c.id, c.data_type),
        "dataType": c.data_type,
        "tileMatrixSets": c.tile_matrix_sets,
        "extent": extent,
        "links": links(&self_url, extra),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/collections/{collectionId}/tiles` — the tilesets of a collection.
pub fn tilesets(base: &str, c: &TiledCollection) -> String {
    let items: Vec<Value> = c
        .tile_matrix_sets
        .iter()
        .map(|tms| {
            json!({
                "tileMatrixSet": tms,
                "dataType": c.data_type,
                "links": links(&format!("{base}/collections/{}/tiles/{}", c.id, tms), vec![
                    ("tiles", format!("{base}/collections/{}/tiles/{}/{{tileMatrix}}/{{tileRow}}/{{tileCol}}", c.id, tms)),
                ]),
            })
        })
        .collect();
    let doc = json!({
        "tiles": items,
        "links": links(&format!("{base}/collections/{}/tiles", c.id), vec![
            ("collection", format!("{base}/collections/{}", c.id)),
        ]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/collections/{collectionId}/tiles/{tileMatrixSetId}` — one tileset.
pub fn tileset(base: &str, c: &TiledCollection, g: &TileMatrixSet) -> String {
    let self_url = format!("{base}/collections/{}/tiles/{}", c.id, g.id);
    let tile_url = format!("{self_url}/{{tileMatrix}}/{{tileRow}}/{{tileCol}}");
    let zs: Vec<u32> = g.levels.iter().map(|l| l.z).collect();
    let minzoom = *zs.first().unwrap_or(&0);
    let maxzoom = *zs.last().unwrap_or(&0);
    let extent = c.extent.map(|e| json!({
        "spatial": { "bbox": [e], "crs": "http://www.opengis.net/def/crs/OGC/1.3/CRS84" }
    }));
    let mut doc = json!({
        "tileMatrixSet": g.id,
        // 类型标记：标准/内置=true。tileMatrixSetURI 只在内置给官方 OGC 登记 URI
        // （见下方补写），与 tileMatrixSets/{id} 文档、WMTS XML 面同口径。
        "standard": is_preset_id(&g.id),
        "dataType": c.data_type,
        "title": format!("{} ({})", c.title, g.id),
        "description": format!("Tiles of '{}' on grid '{}'.", c.id, g.id),
        "crs": grid_crs_uri(g),
        "epoch": 0,
        "minZoomLevel": minzoom,
        "maxZoomLevel": maxzoom,
        "tileMatrix": g.levels.iter().map(|l| l.z.to_string()).collect::<Vec<_>>(),
        "format": c.formats.first().copied().unwrap_or("image/png"),
        "templates": [tile_url],
        "extent": extent,
        "links": links(&self_url, vec![
            ("collection", format!("{base}/collections/{}", c.id)),
            ("tiles", tile_url),
        ]),
    });
    if let Some(uri) = preset_ogc_uri(&g.id) {
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("tileMatrixSetURI".to_string(), json!(uri));
        }
    }
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> TileMatrixSet {
        TileMatrixSet::web_mercator_quad(256)
    }

    fn coll() -> TiledCollection {
        TiledCollection {
            id: "wh".into(),
            title: "wh imagery".into(),
            data_type: "map",
            tile_matrix_sets: vec!["WebMercatorQuad".into()],
            formats: MAP_FORMATS,
            extent: Some([102.55, 25.02, 102.76, 25.43]),
            zoom_range: Some((0, 22)),
        }
    }

    #[test]
    fn landing_and_conformance() {
        let d: Value = serde_json::from_str(&landing("http://h/ogc/tiles/v1")).unwrap();
        let rels: Vec<&str> = d["links"].as_array().unwrap().iter().map(|l| l["rel"].as_str().unwrap()).collect();
        assert!(rels.contains(&"tileMatrixSets"));
        let d: Value = serde_json::from_str(&conformance()).unwrap();
        assert!(d["conformsTo"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().ends_with("/conf/tileset")));
    }

    #[test]
    fn tile_matrix_set_definition_shape() {
        let d: Value = serde_json::from_str(&tile_matrix_set("http://h", &grid(), None)).unwrap();
        assert_eq!(d["id"], "WebMercatorQuad");
        assert_eq!(d["crs"], "http://www.opengis.net/def/crs/EPSG/0/3857");
        let m = d["tileMatrices"].as_array().unwrap();
        assert_eq!(m.len(), 26); // z0..25
        assert_eq!(m[0]["tileWidth"], 256);
        assert_eq!(m[0]["matrixWidth"], 1);
        assert_eq!(m[25]["matrixWidth"], 1 << 25);
        assert!(m[0]["scaleDenominator"].as_f64().unwrap() > 5.5e8);
    }

    /// 类型标记：内置标准网格 `standard=true` 且给官方 OGC `uri`；自定义网格
    /// `standard=false` 且不伪造 `uri`（与 WMTS XML 面只在内置写 TileMatrixSetURI 一致）。
    /// 验证 tile_matrix_set / tileset / tile_matrix_sets 三个文档同口径。
    #[test]
    fn standard_vs_custom_type_marker_and_honest_uri() {
        let d: Value =
            serde_json::from_str(&tile_matrix_set("http://h", &grid(), None)).unwrap();
        assert_eq!(d["standard"], true);
        assert_eq!(
            d["uri"],
            "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad"
        );

        let mut custom = TileMatrixSet::web_mercator_quad(256);
        custom.id = "arcgis_km_td".into();
        let d: Value =
            serde_json::from_str(&tile_matrix_set("http://h", &custom, None)).unwrap();
        assert_eq!(d["standard"], false);
        assert!(d.get("uri").is_none(), "自定义网格不应伪造 OGC uri");

        // tileset 文档同口径：内置有 tileMatrixSetURI，自定义省略。
        let c = coll();
        let d: Value = serde_json::from_str(&tileset("http://h", &c, &grid())).unwrap();
        assert_eq!(d["standard"], true);
        assert_eq!(
            d["tileMatrixSetURI"],
            "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad"
        );
        let d: Value = serde_json::from_str(&tileset("http://h", &c, &custom)).unwrap();
        assert_eq!(d["standard"], false);
        assert!(d.get("tileMatrixSetURI").is_none());

        // 列表项也带类型标记。
        let list: Value =
            serde_json::from_str(&tile_matrix_sets("http://h", &[grid(), custom])).unwrap();
        let items = list["tileMatrixSets"].as_array().unwrap();
        let flag = |id: &str| {
            items
                .iter()
                .find(|x| x["id"] == id)
                .unwrap()["standard"]
                .as_bool()
                .unwrap()
        };
        assert!(flag("WebMercatorQuad"));
        assert!(!flag("arcgis_km_td"));
    }

    #[test]
    fn collection_and_tileset_documents() {
        let c = coll();
        let d: Value = serde_json::from_str(&collection("http://h", &c)).unwrap();
        assert_eq!(d["dataType"], "map");
        assert_eq!(d["tileMatrixSets"][0], "WebMercatorQuad");
        assert_eq!(d["extent"]["spatial"]["bbox"][0][0], 102.55);
        let d: Value = serde_json::from_str(&tilesets("http://h", &c)).unwrap();
        assert_eq!(d["tiles"][0]["tileMatrixSet"], "WebMercatorQuad");
        let d: Value = serde_json::from_str(&tileset("http://h", &c, &grid())).unwrap();
        assert_eq!(d["minZoomLevel"], 0);
        assert_eq!(d["maxZoomLevel"], 25);
        assert_eq!(d["format"], "image/png");
        assert!(d["templates"][0].as_str().unwrap().contains("{tileMatrix}"));
    }

    #[test]
    fn vector_collection_advertises_mvt() {
        let c = TiledCollection {
            id: "places".into(),
            title: "places".into(),
            data_type: "vector",
            tile_matrix_sets: vec!["WebMercatorQuad".into()],
            formats: VECTOR_FORMATS,
            extent: None,
            zoom_range: None,
        };
        let d: Value = serde_json::from_str(&tileset("http://h", &c, &grid())).unwrap();
        assert_eq!(d["dataType"], "vector");
        assert_eq!(d["format"], "application/vnd.mapbox-vector-tile");
    }

    /// `WorldCRS84Quad` 的 `topLeftCorner`/`orderedAxes` 都是经度在前，`crs`
    /// 必须落到 CRS84 而不是 EPSG:4326，否则只看 `crs` 的客户端会按
    /// 注册表轴序（纬度在前）交换坐标、整层镜像。与 WMTS XML 的
    /// `<SupportedCRS>`（`wms-protocol::wmts::supported_crs`）保持同口径。
    #[test]
    fn world_crs84_quad_declares_crs84() {
        let g = TileMatrixSet::world_crs84_quad(256);
        assert_eq!(g.id, "WorldCRS84Quad");
        let d: Value = serde_json::from_str(&tile_matrix_set("http://h", &g, None)).unwrap();
        assert_eq!(d["crs"], "http://www.opengis.net/def/crs/OGC/1.3/CRS84");
        assert_eq!(d["orderedAxes"][0], "X");
        assert_eq!(d["orderedAxes"][1], "Y");
        assert_eq!(d["tileMatrices"][0]["topLeftCorner"][0], -180.0); // 经度在前
        assert!(!d["crs"].as_str().unwrap().contains("EPSG"));
        // tileset 文档同一网格同样给 CRS84；投影网格仍给 EPSG def-URI
        let c = coll();
        let t: Value = serde_json::from_str(&tileset("http://h", &c, &g)).unwrap();
        assert_eq!(t["crs"], "http://www.opengis.net/def/crs/OGC/1.3/CRS84");
        assert_eq!(t["epoch"], 0);
        let w: Value = serde_json::from_str(&tileset("http://h", &c, &grid())).unwrap();
        assert_eq!(w["crs"], "http://www.opengis.net/def/crs/EPSG/0/3857");
    }

    /// 文档内至多一条 `self`（`links()` 已自带，extra 不能再补）；非 `tileset` 的
    /// rel 不得重复。`rel="tileset"` 允许多条 —— OGC API - Tiles 给集合的每个
    /// tileMatrixSet 各指一条，但 href 必须可区分。
    #[test]
    fn links_are_well_formed() {
        let grids = vec![
            grid(),
            TileMatrixSet::world_crs84_quad(256),
            {
                // 尺寸后缀（@512）是 WMTS 层的命名约定，这里给个独立 id 以免
                // 自己造出两个同名网格（那不是服务能出现的形态）
                let mut g = TileMatrixSet::web_mercator_quad(512);
                g.id = "WebMercatorQuad512".into();
                g
            },
        ];
        let c = TiledCollection {
            id: "wh".into(),
            title: "wh imagery".into(),
            data_type: "map",
            tile_matrix_sets: grids.iter().map(|g| g.id.clone()).collect(),
            formats: MAP_FORMATS,
            extent: Some([102.55, 25.02, 102.76, 25.43]),
            zoom_range: Some((0, 22)),
        };
        for (name, doc) in [
            ("landing", landing("http://h")),
            ("tileMatrixSets", tile_matrix_sets("http://h", &grids)),
            ("tileMatrixSet", tile_matrix_set("http://h", &grids[1], None)),
            ("collections", collections("http://h", &[c.clone()])),
            ("collection", collection("http://h", &c)),
            ("tilesets", tilesets("http://h", &c)),
            ("tileset", tileset("http://h", &c, &grids[0])),
        ] {
            let v: Value = serde_json::from_str(&doc).unwrap();
            check_links(name, &v["links"]);
            for key in ["tileMatrixSets", "collections", "tiles"] {
                if let Some(items) = v[key].as_array() {
                    for it in items {
                        // collection/tileset 文档里的 `tileMatrixSets` 是网格 id 字符串
                        // 数组（规范如此），只有对象型条目才带 links
                        if it.is_object() {
                            check_links(&format!("{name}[{key}]"), &it["links"]);
                        }
                    }
                }
            }
        }
    }

    fn check_links(name: &str, links: &Value) {
        let items: Vec<&Value> = links
            .as_array()
            .unwrap_or_else(|| panic!("{name} 文档缺 links 数组"))
            .iter()
            .collect();
        let rels: Vec<&str> = items.iter().filter_map(|l| l["rel"].as_str()).collect();
        assert_eq!(
            rels.iter().filter(|r| **r == "self").count(),
            1,
            "{name} 的 self 链接应为 1 条，实际 {rels:?}"
        );
        let others: Vec<&str> = rels.iter().copied().filter(|r| *r != "tileset").collect();
        let mut sorted = others.clone();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "{name} 有重复 rel：{others:?}");
        let hrefs: Vec<&str> = items.iter().filter_map(|l| l["href"].as_str()).collect();
        let mut hs = hrefs.clone();
        hs.sort_unstable();
        hs.dedup();
        assert_eq!(hrefs.len(), hs.len(), "{name} 有完全相同的链接：{hrefs:?}");
    }
}
