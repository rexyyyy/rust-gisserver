//! `.tpk`（ZIP 缓存包）解压读取的实测验收。
//!
//! 与 `real_cache.rs`（目录式缓存）分工不同：这里证的是 **TPK 容器层**
//! ——ZIP 中央目录解析、store/deflate 解出、幂等解压落点，以及解开后与目录缓存
//! 完全同形（compact cache v1、`Conf.xml`/`conf.cdi` → custom grid → 真实取瓦）。
//! 样例：本地 TPK 样例，由 `GS_TEST_TPK` 指定（CGCS2000 3°带 GK zone 34，
//! EPSG:4522，8 级 LOD；包内条目全为 store，deflate 分支由 `src/tpk.rs` 的单测
//! 覆盖）。未设置或文件不存在时整组跳过。

use std::path::PathBuf;

use gisserver::arcgis_cache::bundle::CacheVersion;
use gisserver::arcgis_cache::{grid, ArcgisCache};

const TPK_ENV: &str = "GS_TEST_TPK";

fn tpk() -> Option<PathBuf> {
    let Ok(value) = std::env::var(TPK_ENV) else {
        eprintln!("跳过：未设置 {TPK_ENV}");
        return None;
    };
    let p = PathBuf::from(value);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("跳过：{TPK_ENV} 指向的文件不存在");
        None
    }
}

/// 打开 `.tpk`：解压定位 → 判定 v1 → CRS 4522 → LOD 阶梯非空。
#[test]
fn tpk_opens_as_v1_4522_cache() {
    let Some(path) = tpk() else {
        return;
    };
    let ac = ArcgisCache::open(&path)
        .unwrap_or_else(|e| panic!("打开 {} 失败: {e}", path.display()));
    assert_eq!(ac.version(), CacheVersion::V1, "样例 TPK 是 compact cache v1");
    let info = ac.info();
    assert_eq!(info.crs, "EPSG:4522", "应为 CGCS2000 3°带 GK zone 34");
    assert_eq!(info.tile_cols, 256);
    assert!(!info.lods.is_empty(), "LOD 阶梯不应为空");
    let (lo, hi) = (info.min_zoom(), info.max_zoom());
    assert!(hi > lo, "层级跨度 {}..{}", lo, hi);
}

/// 网格由 Conf.xml 推导（米制投影带），按自身分辨率能算出真实瓦片地址。
#[test]
fn tpk_derives_grid_and_reads_real_tile() {
    let Some(path) = tpk() else {
        return;
    };
    let ac = ArcgisCache::open(&path).expect("打开 tpk");
    let spec = ac.grid_spec("arcgis_tpk").expect("推导网格");
    assert_eq!(spec.crs, "EPSG:4522");
    let bounds = ac.bounds().expect("conf.cdi 应给出数据范围");
    let z = ac.probe().max_zoom;
    // 中心点反算 (col,row)，再在 ±1 圈邻域找一个有图的槽位（稀疏层中心可能空白）。
    let cx = (bounds[0] + bounds[2]) / 2.0;
    let cy = (bounds[1] + bounds[3]) / 2.0;
    let (c, r) = grid::tile_indices(&spec, z, cx, cy).unwrap();
    let mut found = None;
    'probe: for dr in -1i64..=1 {
        for dc in -1i64..=1 {
            let cc = c as i64 + dc;
            let rr = r as i64 + dr;
            if cc < 0 || rr < 0 {
                continue;
            }
            if let Some(t) = ac.read_tile(z, cc as u32, rr as u32).unwrap_or(None) {
                found = Some((cc, rr, t.bytes.len(), t.content_type));
                break 'probe;
            }
        }
    }
    let (cc, rr, len, mime) = found.expect("tpk 解压后应能在数据范围内取到真实瓦片");
    assert!(mime.starts_with("image/"), "内容类型应为影像：{mime}");
    assert!(len > 100, "瓦片过小：{len}B");
    println!("{}: v{:?} z{z} 命中 ({cc},{rr}) {len}B {mime}", path.display(), ac.version());
}
