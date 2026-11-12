//! `.tpk` 解压目录在运行期被删掉后的「缺失即重解」自愈验收。
//!
//! 单开一个测试文件（`cargo test` 按二进制串行执行）：本测试会真删解压目录，放在
//! `tpk.rs` 里会和同文件并发运行的取瓦用例互相踩踏。样例同 `tpk.rs`，缺失即跳过。

use std::path::{Path, PathBuf};

use gisserver::arcgis_cache::{grid, ArcgisCache};

const TPK: &str = r"E:\data_km\longjie2en.tpk";

/// 两个测试都要真删同一个解压目录：Windows 下别人正打开着 bundle 文件时删除会
/// `PermissionDenied`，所以让它们串行（同二进制内默认并发）。
static EXTRACT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tpk() -> Option<PathBuf> {
    let p = Path::new(TPK);
    p.is_file().then(|| p.to_path_buf())
}

/// 从 `_alllayers` 逐级向上找到解压根（带 `.extract_ok` 的那一层）。
fn extract_root(allayers: &Path) -> PathBuf {
    let mut cur = allayers.to_path_buf();
    loop {
        if cur.join(".extract_ok").is_file() {
            return cur;
        }
        assert!(cur.pop(), "未找到解压根：{}", allayers.display());
    }
}

/// 在数据中心 ±1 圈邻域找一个真有图的瓦片地址（稀疏层中心槽可能空白）。
fn find_tile(ac: &ArcgisCache) -> Option<(u32, u32, u32)> {
    let spec = ac.grid_spec("arcgis_longjie2en").ok()?;
    let b = ac.bounds()?;
    let z = ac.probe().max_zoom;
    let (c, r) = grid::tile_indices(&spec, z, (b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)?;
    for dr in -1i64..=1 {
        for dc in -1i64..=1 {
            let (cc, rr) = (c as i64 + dc, r as i64 + dr);
            if cc < 0 || rr < 0 {
                continue;
            }
            if ac.read_tile(z, cc as u32, rr as u32).ok().flatten().is_some() {
                return Some((z, cc as u32, rr as u32));
            }
        }
    }
    None
}

/// 删掉整个解压目录后，下一次取瓦应自行重解并恢复到原来的字节。
#[test]
fn tile_reads_recover_after_extraction_dir_deleted() {
    let Some(path) = tpk() else {
        eprintln!("跳过：{TPK} 不存在");
        return;
    };
    let _serial = EXTRACT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ac = ArcgisCache::open(&path).expect("打开 tpk");
    let root = extract_root(ac.allayers_dir());
    // 只允许删临时目录下的解压产物，绝不能碰真实目录缓存。
    assert!(
        root.starts_with(std::env::temp_dir()),
        "解压目录不在临时目录内，拒绝删除：{}",
        root.display()
    );

    let (z, col, row) = find_tile(&ac).expect("解压后应能取到真实瓦片");
    let before = ac
        .read_tile(z, col, row)
        .unwrap()
        .expect("首次读取")
        .bytes
        .len();

    std::fs::remove_dir_all(&root).expect("删除解压目录");
    assert!(!root.exists(), "解压目录应已消失");

    let after = ac
        .read_tile(z, col, row)
        .unwrap()
        .unwrap_or_else(|| panic!("目录被删后 z{z} ({col},{row}) 应重解恢复，而不是持续 404"));
    assert!(root.is_dir(), "重解应重建解压目录：{}", root.display());
    assert!(after.content_type.starts_with("image/"));
    assert_eq!(
        after.bytes.len(),
        before,
        "重解后的瓦片字节数应与删除前一致（同一源包）"
    );
    println!("{TPK}: 删目录后自愈成功，z{z} ({col},{row}) = {}B", after.bytes.len());
}

/// 并发请求下只重解一次，且所有请求都拿到图（不能有兄弟请求因节流而 404）。
#[test]
fn concurrent_reads_all_recover() {
    let Some(path) = tpk() else {
        eprintln!("跳过：{TPK} 不存在");
        return;
    };
    let _serial = EXTRACT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ac = std::sync::Arc::new(ArcgisCache::open(&path).expect("打开 tpk"));
    let root = extract_root(ac.allayers_dir());
    assert!(root.starts_with(std::env::temp_dir()), "解压目录不在临时目录内：{}", root.display());

    let (z, col, row) = find_tile(&ac).expect("解压后应能取到真实瓦片");
    let expect = ac.read_tile(z, col, row).unwrap().expect("首次读取").bytes.len();
    // 邻域瓦片：删除后并发重读，可得性集合应与删除前逐一对齐（稀疏层的空槽仍为空）。
    let addrs: Vec<(u32, u32, u32)> = (0..8).map(|i| (z, col + i, row)).collect();
    let before: Vec<Option<usize>> = addrs
        .iter()
        .map(|a| ac.read_tile(a.0, a.1, a.2).unwrap().map(|t| t.bytes.len()))
        .collect();
    assert!(before.iter().any(|b| b.is_some()), "邻域里至少要有一张有图的瓦片");

    std::fs::remove_dir_all(&root).expect("删除解压目录");
    let got = std::sync::Mutex::new(vec![None; addrs.len()]);
    let got = &got;
    std::thread::scope(|s| {
        for (i, a) in addrs.iter().enumerate() {
            let ac = std::sync::Arc::clone(&ac);
            s.spawn(move || {
                let n = ac.read_tile(a.0, a.1, a.2).ok().flatten().map(|t| t.bytes.len());
                got.lock().unwrap()[i] = n;
            });
        }
    });
    let after = got.lock().unwrap().clone();
    for i in 0..addrs.len() {
        assert_eq!(
            after[i],
            before[i],
            "并发自愈后 z{} 第 {i} 个邻域瓦片的可得性/大小应与删除前一致（节流让兄弟请求 404 就是回归）",
            z
        );
    }
    assert!(root.is_dir());
    assert_eq!(
        ac.read_tile(z, col, row).unwrap().expect("重解后仍要能取瓦").bytes.len(),
        expect,
        "重解后同一瓦片字节数应一致"
    );
}

/// 缓存区外 / 未切层级的请求不应触发重解（那是正常空瓦片，不是目录丢失）。
#[test]
fn out_of_coverage_reads_stay_empty() {
    let Some(path) = tpk() else {
        eprintln!("跳过：{TPK} 不存在");
        return;
    };
    let ac = ArcgisCache::open(&path).expect("打开 tpk");
    let z = ac.info().max_zoom();
    // 数据外的地址：bundle 不存在但 _alllayers 还在 → 走不到重解分支，返回空瓦片。
    assert!(ac.read_tile(z, u32::MAX, u32::MAX).unwrap().is_none());
    // 层级本身不存在时也不应触发任何解压。
    assert!(ac.read_tile(u32::MAX - 1, 0, 0).unwrap().is_none());
}
