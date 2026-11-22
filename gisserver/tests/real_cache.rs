//! 本地真实 ArcGIS 缓存的验收测试。
//!
//! 数据不在仓库里（体积大、且属于第三方产出），因此默认 `#[ignore]`。缓存根目录
//! 由环境变量 `GS_TEST_ARCGIS_CACHE` 指定，未设置或目录不存在时整组跳过。有数据的
//! 环境执行：
//!
//! ```text
//! GS_TEST_ARCGIS_CACHE=/path/to/cache cargo test --features arcgis-cache --test real_cache -- --ignored --nocapture
//! ```
//!
//! # 为什么值得留这条
//!
//! 合成 fixture 只能证明「代码与自己写的字节一致」；compact cache 的索引顺序、
//! bundle 命名的十六进制习惯、`conf.cdi` 的实际精度都由 ArcGIS 的产出决定。真实
//! 数据一旦读错，表现是**整幅地图镜像错位而不报任何错**——只有拿真实缓存对齐地理
//! 范围才能发现。

use std::path::{Path, PathBuf};

use gisserver::arcgis_cache::bundle::{ArcgisCache, CacheVersion};
use gisserver::arcgis_cache::grid;

/// 缓存根目录的环境变量名（未设置或目录不存在时整组测试直接跳过）。
const ROOT_ENV: &str = "GS_TEST_ARCGIS_CACHE";

fn root() -> Option<PathBuf> {
    let Ok(value) = std::env::var(ROOT_ENV) else {
        eprintln!("跳过：未设置 {ROOT_ENV}");
        return None;
    };
    let p = PathBuf::from(value);
    if p.is_dir() {
        Some(p)
    } else {
        eprintln!("跳过：{ROOT_ENV} 指向的目录不存在");
        None
    }
}

/// 枚举缓存根目录下的一级服务目录（排序后返回，保证测试结果可复现）。
fn service_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// 打开一个缓存服务：服务根本身或其一级图层子目录都能被 `CacheDocs::load` 找到。
fn open_service(root: &Path, svc: &str) -> ArcgisCache {
    ArcgisCache::open(&root.join(svc))
        .unwrap_or_else(|e| panic!("{svc} 打开失败: {e}"))
}

#[test]
#[ignore = "需要本地真实 ArcGIS 缓存（GS_TEST_ARCGIS_CACHE）"]
fn sample_cache_conf_and_probe_agree() {
    let Some(root) = root() else {
        return;
    };
    let mut checked = 0usize;
    for dir in service_dirs(&root) {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let ac = open_service(&root, &name);

        // 格式判定：真实目录缓存是 v1（有 .bundlx）或 v2（索引内嵌）
        assert!(
            matches!(ac.version(), CacheVersion::V1 | CacheVersion::V2),
            "{name}: 非预期格式 {:?}",
            ac.version()
        );
        let info = ac.info();
        assert!(!info.crs.is_empty() && info.crs != "EPSG:0", "{name}: 缺少 CRS");
        assert_eq!(info.tile_cols, 256, "{name}: 瓦片边长应为 256");
        assert_eq!(info.tile_rows, 256, "{name}: 瓦片边长应为 256");
        assert!(info.packet_size > 0, "{name}: PacketSize 应为正");
        assert!(!info.lods.is_empty(), "{name}: LOD 阶梯不应为空");

        // _alllayers 的层级清单不能超出 Conf.xml 的 LOD 阶梯，否则寻址会取到不存在的级
        let probe = ac.probe();
        assert!(!probe.levels.is_empty(), "{name}: _alllayers 未探测到层级");
        for z in &probe.levels {
            assert!(
                info.lods.iter().any(|l| l.level == *z),
                "{name}: L{z:02} 不在 LOD 阶梯内"
            );
        }

        // LOD → custom grid：范围必须包住数据，且数据四角都能算出网格内的行列
        let spec = ac.grid_spec(&format!("arcgis_{name}")).unwrap_or_else(|e| {
            panic!("{name}: 网格推导失败: {e}")
        });
        assert_eq!(spec.crs, info.crs, "{name}: 网格 CRS 与 Conf.xml 不一致");
        assert_eq!(spec.levels.len(), info.lods.len(), "{name}: 网格层级数不一致");
        if let Some(b) = ac.bounds() {
            assert!(
                spec.extent[0] <= b[0] && spec.extent[2] >= b[2],
                "{name}: 网格东西范围未包住数据"
            );
            assert!(
                spec.extent[1] <= b[1] && spec.extent[3] >= b[3],
                "{name}: 网格南北范围未包住数据"
            );
            let z = probe.levels[probe.levels.len() / 2];
            for (x, y) in [(b[0], b[1]), (b[2], b[3]), (b[0], b[3]), (b[2], b[1])] {
                let (c, r) =
                    grid::tile_indices(&spec, z, x, y).expect("数据角点应能算出行列");
                assert!(
                    grid::tile_bounds(&spec, z, c, r).is_some(),
                    "{name}: ({x},{y}) → col {c} row {r} 越出矩阵 {}x{}",
                    spec.matrix_w[z as usize],
                    spec.matrix_h[z as usize]
                );
            }
        }

        // 层级最深一级的区外读取不得报错，应给空瓦片
        let z = info.max_zoom();
        assert!(
            ac.read_tile(z, u32::MAX, u32::MAX).expect("区外读取").is_none(),
            "{name}: 区外应返回空瓦片"
        );
        checked += 1;
    }
    assert!(checked > 0, "缓存根目录下没有可识别的服务目录");
}

/// 每个样例都要能打开并推出网格，且在数据中心邻域取到真实瓦片；任何一个环节
/// （编码、大小写、子目录、LOD）在真实产出上翻车，这里会指名是哪个服务。
#[test]
#[ignore = "需要本地真实 ArcGIS 缓存（GS_TEST_ARCGIS_CACHE）"]
fn every_sample_cache_opens_and_derives_a_grid() {
    let Some(root) = root() else {
        return;
    };
    let mut checked = 0usize;
    for dir in service_dirs(&root) {
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
    assert!(checked > 0, "缓存根目录下没有可识别的服务目录");
}
