//! E 盘真实 ArcGIS 缓存的验收测试。
//!
//! 数据不在仓库里（体积大、且属于第三方产出），因此默认 `#[ignore]`。有数据的
//! 开发机上执行：
//!
//! ```text
//! cargo test -p arcgis-cache --test real_cache -- --ignored --nocapture
//! ```
//!
//! # 为什么值得留这条
//!
//! 合成 fixture 只能证明「代码与自己写的字节一致」；compact cache 的索引顺序、
//! bundle 命名的十六进制习惯、`conf.cdi` 的实际精度都由 ArcGIS 的产出决定。真实
//! 数据一旦读错，表现是**整幅地图镜像错位而不报任何错**——只有拿真实缓存对齐地理
//! 范围才能发现。这里的期望值来自本机实测：L10 的 `R0100C0600.bundle` 按 v1 列优
//! 先解出的行列（col 1603–1607 / row 370–372）正好落在 `conf.cdi` 的昆明范围内。

use std::path::{Path, PathBuf};

use gisserver::arcgis_cache::bundle::{ArcgisCache, CacheVersion};
use gisserver::arcgis_cache::grid;

/// 本机 ArcGIS 缓存根目录（不存在时整组测试直接跳过）。
const ROOT: &str = r"E:\data_km\arcgis_cah";
/// 已实测的样例服务（CGCS2000 地理坐标系、19 级 LOD、compact cache v1）。
const TD: &str = "TD_DZZHYHD_G_2025_GCS2000";

fn root() -> Option<PathBuf> {
    let p = Path::new(ROOT);
    if p.is_dir() {
        Some(p.to_path_buf())
    } else {
        None
    }
}

/// 打开一个缓存服务：服务根本身或其一级图层子目录都能被 `CacheDocs::load` 找到。
fn open_service(root: &Path, svc: &str) -> ArcgisCache {
    ArcgisCache::open(&root.join(svc))
        .unwrap_or_else(|e| panic!("{svc} 打开失败: {e}"))
}

#[test]
#[ignore = "需要 E 盘真实 ArcGIS 缓存数据"]
fn td_cache_matches_measured_layout() {
    let Some(root) = root() else {
        eprintln!("跳过：{ROOT} 不存在");
        return;
    };
    let ac = open_service(&root, TD);

    // 格式判定：真实产出是 10.x 的 v1（有 .bundlx，且 Conf.xml 里没有 BundleVersion）
    assert_eq!(ac.version(), CacheVersion::V1);
    let info = ac.info();
    assert_eq!(info.crs, "EPSG:4490");
    assert_eq!(info.tile_cols, 256);
    assert_eq!(info.packet_size, 128);
    assert_eq!(info.lods.len(), 19);
    assert_eq!((info.min_zoom(), info.max_zoom()), (0, 18));

    // _alllayers 的层级清单不能超出 Conf.xml 的 LOD 阶梯，否则寻址会取到不存在的级
    let probe = ac.probe();
    assert!(!probe.levels.is_empty());
    for z in &probe.levels {
        assert!(info.lods.iter().any(|l| l.level == *z), "L{z:02} 不在 LOD 阶梯内");
    }

    // conf.cdi 的昆明范围
    let b = ac.bounds().expect("该缓存有 conf.cdi");
    assert!(b[0] > 102.2 && b[0] < 102.3, "minx = {}", b[0]);
    assert!(b[1] > 24.4 && b[1] < 24.5, "miny = {}", b[1]);
    assert!(b[2] > 102.8 && b[2] < 102.9, "maxx = {}", b[2]);
    assert!(b[3] > 24.7 && b[3] < 24.8, "maxy = {}", b[3]);

    // LOD → custom grid：范围必须包住数据，且数据四角都能算出网格内的行列
    let spec = ac.grid_spec("arcgis_td").expect("网格推导");
    assert_eq!(spec.crs, "EPSG:4490");
    assert_eq!(spec.levels.len(), 19);
    assert!(spec.extent[0] <= b[0] && spec.extent[2] >= b[2]);
    assert!(spec.extent[1] <= b[1] && spec.extent[3] >= b[3]);
    let z = 10;
    for (x, y) in [(b[0], b[1]), (b[2], b[3]), (b[0], b[3]), (b[2], b[1])] {
        let (c, r) = grid::tile_indices(&spec, z, x, y).expect("数据角点应能算出行列");
        assert!(
            grid::tile_bounds(&spec, z, c, r).is_some(),
            "({x},{y}) → col {c} row {r} 越出矩阵 {}x{}",
            spec.matrix_w[z as usize],
            spec.matrix_h[z as usize]
        );
    }

    // 实测命中的瓦片：v1 列优先的槽位，取错顺序会在这里拿到 None
    let t = ac
        .read_tile(z, 1603, 370)
        .expect("读 L10/R0100C0600 不应报错")
        .expect("实测该槽位有瓦片");
    assert!(
        t.content_type.starts_with("image/"),
        "Content-Type = {}",
        t.content_type
    );
    assert!(t.bytes.len() > 100, "瓦片仅 {} 字节", t.bytes.len());
    // 缓存区外（非洲）不得报错，应给空瓦片
    assert!(ac.read_tile(z, 0, 0).expect("区外读取").is_none());
}

/// 三个样例都要能打开并推出网格；任何一个环节（编码、大小写、子目录、LOD）
/// 在真实产出上翻车，这里会指名是哪个服务。
#[test]
#[ignore = "需要 E 盘真实 ArcGIS 缓存数据"]
fn every_sample_cache_opens_and_derives_a_grid() {
    let Some(root) = root() else {
        eprintln!("跳过：{ROOT} 不存在");
        return;
    };
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let ac = open_service(&root, &name);
        let spec = ac
            .grid_spec(&format!("arcgis_{name}"))
            .unwrap_or_else(|e| panic!("{name}: 网格推导失败: {e}"));
        assert!(spec.resolutions.len() >= 2, "{name}: LOD 太少");
        assert!(spec.extent[2] > spec.extent[0] && spec.extent[3] > spec.extent[1]);
        // 比例尺阶梯必须递减（ArcGIS 的 Scale 与 Resolution 同向）
        for w in spec.scales.windows(2) {
            if w[0] > 0.0 && w[1] > 0.0 {
                assert!(w[1] < w[0], "{name}: 比例尺阶梯非单调 {w:?}");
            }
        }
        // 中心点在实际切过的层级上应当取到图（数据范围中心不应是空白）；
        // 取实际级数的中间一级，太细一级可能落在空白区。
        // 取样点用**数据范围**中心：网格 extent 的左上角是 TileOrigin（常为
        // -180/90），它的“中心”会落在没切片的空白区，而不是数据上。
        let z = { let lv = &ac.probe().levels; lv[lv.len() / 2] };
        let b = ac.bounds().unwrap_or(spec.extent);
        let cx = (b[0] + b[2]) / 2.0;
        let cy = (b[1] + b[3]) / 2.0;
        let (c, r) = grid::tile_indices(&spec, z, cx, cy).unwrap();
        // 中心 ±1 圈邻域内找一个有图的槽位（稀疏图层中心恰好空白时也能确认真的切过片）
        let mut found = None;
        'probe: for dr in -1i64..=1 {
            for dc in -1i64..=1 {
                let (rr, cc) = (r as i64 + dr, c as i64 + dc);
                if rr < 0 || cc < 0 {
                    continue;
                }
                if let Some(t) = ac.read_tile(z, cc as u32, rr as u32).unwrap_or_else(|e| {
                    panic!("{name}: 读 z{z} ({cc},{rr}) 失败: {e}")
                }) {
                    found = Some((cc, rr, t.bytes.len()));
                    break 'probe;
                }
            }
        }
        let (cc, rr, len) =
            found.unwrap_or_else(|| panic!("{name}: z{z} 中心 ({c},{r}) 邻域取不到任何瓦片"));
        println!("{name}: {} 级, v{:?}, z{z} 命中 ({cc},{rr}) {len}B", spec.levels.len(), ac.version());
        checked += 1;
    }
    assert!(checked >= 3, "样例缓存应有 3 个，实际检查 {checked} 个");
}
