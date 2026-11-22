//! ArcGIS 缓存包导入（`unpack_to` → 扁平化 → 可发布缓存目录）的实测验收。
//!
//! 与 `tpk.rs`（`ArcgisCache::open` 直接打开 `.tpk`，走 `ensure_dir` 幂等解压）
//! 分工不同：这里证的是**导入链路**——
////!
//! * `.tpk`（`v101/图层/conf.xml` + `_alllayers/`）解开后落点目录**本身就是**
//!   一份标准 ArcGIS 缓存目录（缓存内容提升到根，`conf.xml` 与 `_alllayers/`
//!   同级），而不是还留着 `v101/图层` 两层；
//! * `.tpkx`（`root.json` + `tile/`）原样落在根，瓦片根目录名由定义记录给出；
//! * 两者的网格、坐标、LOD 都从各自配置文件读出，归一成同一条定义记录；
//! * 内容完全一致的缓存**引用同一条**定义记录，不新建。
//!
//! 样例：本地 TPK/TPKX 样例，分别由 `GS_TEST_TPK`、`GS_TEST_TPKX` 指定；未设置
//! 或文件不存在时整组跳过。

use std::path::PathBuf;

use gisserver::arcgis_cache::bundle::CacheVersion;
use gisserver::arcgis_cache::definition::{CacheDefinition, DefinitionKind};
use gisserver::arcgis_cache::{conf::CacheDocs, grid, tpk::unpack_to, ArcgisCache};

const TPK_ENV: &str = "GS_TEST_TPK";
const TPKX_ENV: &str = "GS_TEST_TPKX";

fn sample(env_key: &str) -> Option<PathBuf> {
    let Ok(value) = std::env::var(env_key) else {
        eprintln!("跳过：未设置 {env_key}");
        return None;
    };
    let p = PathBuf::from(value);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("跳过：{env_key} 指向的文件不存在");
        None
    }
}

/// `.tpk` 导入后：落点根下直接是 `conf.xml` + `_alllayers/`，且能被打开读瓦。
#[test]
fn tpk_import_produces_flat_cache_dir() {
    let Some(src) = sample(TPK_ENV) else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("tpk_import");
    let r = unpack_to(&src, &dest).expect("导入应成功");

    // 落点根下直接是 conf.xml（不再是 v101/图层/conf.xml）。
    assert!(
        dest.join("conf.xml").is_file(),
        "conf.xml 应提升到落点根：{:?}",
        std::fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>()
    );
    // 瓦片根目录与 conf.xml 同级。
    assert!(dest.join("_alllayers").is_dir(), "_alllayers 应与 conf.xml 同级");
    // 包元数据被收进 _package/，不干扰缓存目录识别。
    assert!(dest.join("_package").is_dir(), "包元数据应收进 _package/");
    assert_eq!(r.config_file, dest.join("conf.xml"));

    // 打开即读：定义记录来自 Conf.xml，瓦片可取。
    let ac = ArcgisCache::open(&dest).expect("导入后的目录应能作为缓存打开");
    assert_eq!(ac.info().kind, DefinitionKind::ConfXml);
    assert_eq!(ac.info().tile_root, "_alllayers");
    assert_eq!(ac.version(), CacheVersion::V1, "TPK 样例带 .bundlx，是 compact v1");

    // 网格由定义记录推导，取到真实瓦片。
    let spec = ac.grid_spec("arcgis_tpk").expect("推导网格");
    let bounds = ac.bounds().expect("conf.cdi 应给出数据范围");
    let z = ac.probe().max_zoom;
    let (c, r) = grid::tile_indices(&spec, z, (bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0).unwrap();
    let mut hit = None;
    'probe: for dr in -1i64..=1 {
        for dc in -1i64..=1 {
            let cc = c as i64 + dc;
            let rr = r as i64 + dr;
            if cc < 0 || rr < 0 {
                continue;
            }
            if let Some(t) = ac.read_tile(z, cc as u32, rr as u32).unwrap_or(None) {
                hit = Some((cc, rr, t.bytes.len()));
                break 'probe;
            }
        }
    }
    assert!(hit.is_some(), "导入后应能取到真实瓦片");
    let (cc, rr, len) = hit.unwrap();
    assert!(len > 100, "瓦片过小：{len}B");
    println!("TPK 导入 → {:?} v{:?} z{z} 命中 ({cc},{rr}) {len}B", ac.info().crs, ac.version());
}

/// `.tpkx` 导入后：`root.json` + `tile/` 在根，瓦片根目录名由定义记录给出。
#[test]
fn tpkx_import_keeps_root_json_layout() {
    let Some(src) = sample(TPKX_ENV) else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("tpkx_import");
    let r = unpack_to(&src, &dest).expect("导入应成功");

    assert!(dest.join("root.json").is_file(), "root.json 应在落点根");
    assert!(dest.join("tile").is_dir(), "tile/ 应在落点根");
    assert_eq!(r.config_file, dest.join("root.json"));

    let ac = ArcgisCache::open(&dest).expect("导入后的目录应能作为缓存打开");
    assert_eq!(ac.info().kind, DefinitionKind::RootJson);
    assert_eq!(ac.info().tile_root, "tile", "瓦片根目录名来自 root.json 的 tileBundlesPath");
    assert_eq!(ac.version(), CacheVersion::V2, "tpkx 是 compact v2");
    assert!(!ac.info().lods.is_empty(), "LOD 应从 root.json 读出");
    println!(
        "TPKX 导入 → {} z{}..{} tile_root={}",
        ac.info().crs,
        ac.info().min_zoom(),
        ac.info().max_zoom(),
        ac.info().tile_root
    );
}

/// 两种形态的配置归一到同一形状：字段名不同，网格语义一致。
#[test]
fn both_package_shapes_map_to_one_definition_shape() {
    let (Some(tpk), Some(tpkx)) = (sample(TPK_ENV), sample(TPKX_ENV)) else {
        eprintln!("跳过：样本未设置或不存在");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let d1 = tmp.path().join("a_tpk");
    let d2 = tmp.path().join("b_tpk");
    unpack_to(&tpk, &d1).unwrap();
    unpack_to(&tpkx, &d2).unwrap();

    let x = CacheDefinition::from_xml(&std::fs::read_to_string(d1.join("conf.xml")).unwrap()).unwrap();
    let y = CacheDefinition::from_root_json(&std::fs::read_to_string(d2.join("root.json")).unwrap()).unwrap();
    // 同一形状的字段：CRS/原点/瓦片尺寸/DPI/LOD 阶梯/PacketSize 都有值。
    for d in [&x, &y] {
        assert!(!d.crs.is_empty() && d.crs != "EPSG:0");
        assert!(d.tile_cols > 0 && d.tile_rows > 0);
        assert!(d.dpi > 0.0);
        assert!(!d.lods.is_empty());
        assert!(d.packet_size > 0);
        assert!(!d.tile_root.is_empty());
    }
    // 两份样本网格不同（不同坐标系）→ 定义记录不得复用。
    assert_ne!(x.fingerprint(), y.fingerprint());
}

/// 同一份缓存导入两次 → 引用同一条定义记录，不新建。
#[test]
fn reimport_reuses_definition_record() {
    let Some(src) = sample(TPK_ENV) else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a_tpk");
    let b = tmp.path().join("b_tpk");
    unpack_to(&src, &a).unwrap();
    unpack_to(&src, &b).unwrap();

    let da = CacheDocs::load(&a).unwrap();
    let db = CacheDocs::load(&b).unwrap();
    assert_eq!(
        da.def.id, db.def.id,
        "内容完全一致的缓存应引用同一条定义记录"
    );
    assert!(db.reused, "第二次应标记为「引用已有记录」");
}

/// 目录缓存（不经包）同样归一到定义记录，且与包形态字段同形。
#[test]
fn directory_cache_and_package_share_definition_shape() {
    let Some(src) = sample(TPK_ENV) else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("tpk_import");
    unpack_to(&src, &dest).unwrap();

    let docs = CacheDocs::load(&dest).expect("目录形态应能装载");
    let def = docs.definition();
    assert_eq!(def.kind, DefinitionKind::ConfXml);
    assert_eq!(def.tile_root, "_alllayers");
    // 范围来自 conf.cdi（数据属性，不是网格属性）。
    assert!(docs.cdi.is_some(), "conf.cdi 应给出数据范围");
}
