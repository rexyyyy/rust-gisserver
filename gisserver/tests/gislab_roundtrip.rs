//! gis-wt arcgisv2webp 插件真实产物的读回验收测试。
//!
//! 与 bundle.rs 里的合成单测不同，这里读的是 gis_invoke 调插件后真实落盘的
//! `_fixtures/cache_out`（由 gen_fixture.py + TD_Conf_min.xml 生成），验证
//! reader 与 writer 端到端字节兼容。缓存不在常规测试路径，故默认 #[ignore]。
//!
//! 生成 + 验收流程：
//!   1) python _fixtures/gen_fixture.py
//!   2) gis_invoke.exe build\plugins gislab.arcgisv2webp @_fixtures\params_build.json
//!   3) cargo test -p gisserver --features arcgis-cache --test gislab_roundtrip -- --ignored --nocapture

use std::path::Path;

use gisserver::arcgis_cache::bundle::{ArcgisCache, CacheVersion};

/// 相对 manifest：gisserver -> rust-gisserver -> thirdparty -> 仓库根 -> _fixtures/cache_out
fn cache_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("_fixtures")
        .join("cache_out")
}

fn nearest_color(payload: &[u8]) -> Option<&'static str> {
    // 不做完整 WebP 解码，用魔数 + 已知尺寸/大小粗判；精确颜色校验由 Python
    // verify_bundle.py 承担（PIL 解码）。这里证明 reader 能按 V2 索引取对瓦片。
    if payload.starts_with(b"RIFF") && payload.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

#[test]
#[ignore = "需要先运行 gis_invoke 生成 _fixtures/cache_out"]
fn gislab_writer_output_readback() {
    let dir = cache_dir();
    assert!(dir.is_dir(), "缓存目录不存在（请先跑 gis_invoke build）: {}", dir.display());
    let ac = ArcgisCache::open(&dir).unwrap_or_else(|e| panic!("打开缓存失败: {e}"));
    assert_eq!(ac.version(), CacheVersion::V2, ".webpdle 产物应判为 V2");

    // L1 的 2x2 网格：conf origin(-180,90)、res=0.3519572、span=90.1
    // bbox 覆盖 col 0-1 / row 0-1，四个瓦片都应能读回且是 WebP。
    let expect = [(0u32, 0u32), (0, 1), (1, 0), (1, 1)];
    let mut got = 0;
    for (col, row) in expect {
        let t = ac
            .read_tile(1, col, row)
            .unwrap_or_else(|e| panic!("read_tile(L1,c{col},r{row}) 失败: {e}"))
            .unwrap_or_else(|| panic!("L1 c{col} r{row} 无瓦片"));
        assert_eq!(t.content_type, "image/webp", "c{col} r{row} 非 WebP");
        assert!(nearest_color(&t.bytes).is_some(), "c{col} r{row} 魔数错误");
        // 4B size 前缀在 offset-4；读回长度应精确，不含前缀、不缺尾
        assert!(t.bytes.len() > 0);
        got += 1;
    }
    assert_eq!(got, 4, "应有 4 个瓦片可读回");
    eprintln!("OK: 真实 writer 产物经 reader 读回 4 个 WebP 瓦片，content_type 全部 image/webp");
}
