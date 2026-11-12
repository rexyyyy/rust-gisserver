//! map-api 侧的 ArcGIS 缓存接线验收。
//!
//! `arcgis-cache` 自己的测试只证明「Conf.xml → 网格 → bundle 字节」这条链在
//! 数据层是对的；这里要证的是**服务层拿到的是同一条链**：图层文件经
//! `ServerConfig::collect_layers` 注入的 custom grid，其分辨率阶梯、`LevelID`
//! 阶梯与地图比例尺必须与 `Conf.xml` 一致，且按这份网格算出的 (z, col, row)
//! 能在缓存里取到瓦片——用错网格（例如套 WebMercatorQuad）会整体错位而
//! 不报错，只有拿真实数据对齐才能发现。
//!
//! 第一条不需要真实数据（默认执行）；第二条需要 E 盘缓存：
//!
//! ```text
//! cargo test -p map-api --test arcgis_grid -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use map_api::config::ServerConfig;

/// 本机 ArcGIS 缓存根目录（不存在时数据相关测试跳过）。
const ROOT: &str = r"E:\data_km\arcgis_cah";
/// 已实测的样例服务（CGCS2000 地理坐标系、19 级 LOD、compact cache v1）。
const TD: &str = "TD_DZZHYHD_G_2025_GCS2000";

/// 建一个只含 `arcgis/<name>.xml` 的临时 catalog 根目录。
///
/// 目录名带进程号，避免与并行的其它测试或上次残留冲突；用完后由调用方删除。
fn temp_catalog(layer_xml: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "rmap_arcgis_grid_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let svc = dir.join("arcgis");
    std::fs::create_dir_all(&svc).expect("创建临时 catalog 目录");
    std::fs::write(svc.join("td.xml"), layer_xml).expect("写入图层文件");
    dir
}

fn rm_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// 图层文件里唯一必须正确的是 `source.type` 与目录名一致、`path` 指到缓存根。
fn layer_xml(path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<layer name="td" title="ArcGIS 预切片缓存接线验收">
  <source type="arcgis" path="{path}"/>
</layer>"#
    )
}

fn config_with_root(root: &Path) -> ServerConfig {
    let text = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <server host="127.0.0.1" port="8080"/>
  <catalog root="{}" backend="files"/>
</config>"#,
        root.display().to_string().replace('\\', "/")
    );
    let value = map_api::xmlcfg::parse_to_value(&text).expect("server 配置应可解析");
    serde_json::from_value(value).expect("server 配置应可反序列化")
}

/// 缓存不可用（路径写错 / 指到了不是缓存的目录）时：collect_layers 只告警、
/// 不注入网格，`LayerRuntime::open` 给出可执行的错误——不能让一个坏图层拖垮启动。
#[test]
fn unusable_cache_warns_instead_of_breaking_startup() {
    let dir = temp_catalog(&layer_xml("E:/definitely/not/a/cache"));
    let cfg = config_with_root(&dir);
    let (layers, grids, warnings) = cfg.collect_layers();

    assert_eq!(layers.len(), 1, "图层本身仍应被收进目录");
    assert_eq!(layers[0].name, "td");
    assert!(grids.is_empty(), "不该注入网格，实际 {grids:?}");
    assert_eq!(
        warnings.iter().filter(|w| w.contains("arcgis cache")).count(),
        1,
        "应恰有一条 arcgis 告警：{warnings:?}"
    );

    // 路径不存在：指名是「缓存目录」，而不是让人去找一个数据文件。
    let err = match map_api::layer::LayerRuntime::open(layers[0].clone(), None, 1, None) {
        Ok(_) => panic!("不存在的缓存应打不开"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("arcgis cache directory not found"),
        "错误应说清是目录缺失：{err}"
    );

    // 目录存在但不是缓存（没有 Conf.xml）：给出格式层面的原因。
    let mut layer = layers[0].clone();
    layer.source = gisserver::wms_common::layer::SourceKind::Arcgis {
        path: dir.to_string_lossy().replace('\\', "/"),
    };
    let err = match map_api::layer::LayerRuntime::open(layer, None, 1, None) {
        Ok(_) => panic!("非缓存目录应打不开"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("invalid arcgis cache"),
        "错误应指名不是 ArcGIS 缓存：{err}"
    );

    rm_dir(&dir);
}

/// E 盘真实缓存走完整接线：Conf.xml → collect_layers → GridDef → TileMatrixSet
/// → WMTS 能力文档里的 ScaleDenominator → bundle 槽位命中。
#[test]
#[ignore = "需要 E 盘真实 ArcGIS 缓存数据"]
fn real_cache_grid_and_scales_survive_the_service_layer() {
    let cache_dir = Path::new(ROOT).join(TD);
    if !cache_dir.is_dir() {
        eprintln!("跳过：{} 不存在", cache_dir.display());
        return;
    }
    let dir = temp_catalog(&layer_xml(&cache_dir.display().to_string().replace('\\', "/")));
    let cfg = config_with_root(&dir);
    let (layers, grids, warnings) = cfg.collect_layers();
    rm_dir(&dir);
    assert!(warnings.is_empty(), "不该有告警：{warnings:?}");

    let layer = layers.iter().find(|l| l.name == "td").expect("图层 td");
    // 图层文件没写 src_crs/grids/extent，全部由缓存自身覆盖
    assert_eq!(layer.src_crs, "EPSG:4490");
    assert_eq!(layer.grids, vec!["arcgis_td".to_string()]);
    assert_eq!(layer.tile_px, 256);
    assert!(layer.extent.is_some(), "extent 应取 conf.cdi 的数据范围");

    let ac = gisserver::arcgis_cache::ArcgisCache::open(&cache_dir).expect("打开缓存");
    let info = ac.info();
    let grid = grids.get("arcgis_td").expect("应注入 arcgis_td 网格");

    // GridDef 就是 Conf.xml 的 LOD 阶梯：分辨率、LevelID、比例尺一一对应
    assert_eq!(grid.resolutions.len(), info.lods.len());
    assert_eq!(grid.levels.len(), info.lods.len());
    assert_eq!(grid.scales.len(), info.lods.len());
    assert_eq!(grid.crs, info.crs);
    assert_eq!(grid.origin, info.origin, "网格左上角应为 TileOrigin");
    for (i, lod) in info.lods.iter().enumerate() {
        assert_eq!(grid.levels[i], lod.level, "第 {i} 级的 LevelID");
        assert!(
            (grid.resolutions[i] - lod.resolution).abs() <= lod.resolution.abs() * 1e-9,
            "第 {i} 级分辨率 {} vs {}",
            grid.resolutions[i],
            lod.resolution
        );
        assert!(
            (grid.scales[i] - lod.scale).abs() <= lod.scale.abs() * 1e-9,
            "第 {i} 级比例尺 {} vs {}",
            grid.scales[i],
            lod.scale
        );
    }

    // to_tms 把 LevelID 变成 z、把源比例尺挂进 TmLevel：WMTS/OGC API 直接宣告它
    let tms = grid.to_tms("arcgis_td");
    assert_eq!(tms.levels.len(), info.lods.len());
    for (i, lv) in tms.levels.iter().enumerate() {
        assert_eq!(lv.z, info.lods[i].level, "TmLevel.z 应取 LevelID");
        let src = lv.scale_denominator.unwrap_or_else(|| {
            panic!("z{} 应携带源比例尺（ArcGIS 0.0254/DPI 口径）", lv.z)
        });
        assert!((src - info.lods[i].scale).abs() <= info.lods[i].scale.abs() * 1e-9);
        // 源比例尺与 OGC 反算的比 = 0.28mm / (0.0254/DPI)，DPI=96 时≈1.058：
        // 这正是网格必须携带 Scale 的原因（拿 OGC 反算会报小 5.5%）。
        let ogc = lv.resolution * gisserver::wms_common::crs::meters_per_unit(&tms.crs) / 0.00028;
        let expect = 0.00028 / (0.0254 / info.dpi);
        let ratio = src / ogc;
        assert!(
            (ratio - expect).abs() / expect < 0.01,
            "z{} 源/OGC 比例尺比 {ratio} 应≈{expect}",
            lv.z
        );
    }
    let xml = gisserver::wms_protocol::wmts::tile_matrix_set_xml(&tms);
    assert!(
        xml.contains(&format!("<ScaleDenominator>{}</ScaleDenominator>", tms.levels[0].scale_denominator.unwrap())),
        "能力文档应原样宣告 Conf.xml 的 z0 比例尺"
    );

    // 按这份网格算出的地址必须能在 bundle 里取到瓦片（用错网格时这里会全空）
    let b = ac.bounds().expect("conf.cdi 数据范围");
    let levels = ac.probe().levels.clone();
    let z = levels[levels.len() / 2];
    let lv = tms.level(z).expect("网格应有该级");
    let res = lv.resolution;
    let col = (((b[0] + b[2]) / 2.0 - tms.origin_x) / (tms.tile_w as f64 * res)).floor();
    let row = ((tms.origin_y - (b[1] + b[3]) / 2.0) / (tms.tile_h as f64 * res)).floor();
    assert!(col >= 0.0 && row >= 0.0, "数据中心应落在网格内 ({col},{row})");
    let (col, row) = (col as u32, row as u32);
    assert!(col < lv.matrix_w && row < lv.matrix_h, "({col},{row}) 越出 {lv:?}");

    let mut hit = None;
    'probe: for dr in -1i64..=1 {
        for dc in -1i64..=1 {
            let (rr, cc) = (row as i64 + dr, col as i64 + dc);
            if rr < 0 || cc < 0 {
                continue;
            }
            if let Some(t) = ac
                .read_tile(z, cc as u32, rr as u32)
                .unwrap_or_else(|e| panic!("读 z{z} ({cc},{rr}) 失败: {e}"))
            {
                hit = Some((cc, rr, t));
                break 'probe;
            }
        }
    }
    let (cc, rr, t) = hit.expect("网格算出的数据中心邻域应取到瓦片");
    assert!(t.content_type.starts_with("image/"), "Content-Type = {}", t.content_type);
    assert!(t.bytes.len() > 100, "瓦片仅 {} 字节", t.bytes.len());
    println!("接线命中 z{z} ({cc},{rr}) {} 字节", t.bytes.len());

    // 图层能按同一条路径被真正打开（check_source_path + ArcgisCache::open）
    let rt = map_api::layer::LayerRuntime::open(layer.clone(), None, 2, None)
        .expect("真实缓存应可打开为图层");
    assert!(rt.arcgis.is_some(), "LayerRuntime 应持有 arcgis 读取器");
}
