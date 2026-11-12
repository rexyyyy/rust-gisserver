//! `.terrainpack`（GWTQ v1）单文件量化网格地形归档：读取与登记。
//!
//! 数据源由 gis-wt 底座 `gis_slice_quantizedmesh`（G10b）产出，二进制布局
//! （见 gis-wt `docs/11-地形量化网格编码设计.md` §4）：
//!
//! ```text
//! magic "GWTQ"(4) | version u16(=1) | headerLen u32 | JSON 头 | dirCount u32
//! | 目录条目 dirCount×25（z/x/y 各 u32 + offset u64 + length u32 + compressed u8）
//! | gzip 瓦片 blob 顺序追加（offset 为相对文件起点的绝对偏移）
//! ```
//!
//! JSON 头字段（`buildArchiveHeaderJson`）：`format/encoding/crs/tile_matrix/
//! tile_size/minzoom/maxzoom/with_normals/available/bounds/height_range`，
//! 其中 `bounds` 恒为经纬度度 `[west, south, east, north]`。

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gisserver::storage::read_exact_at;

/// 单瓦目录条目（25 字节反序列化结果）。
#[derive(Clone, Copy, Debug)]
pub struct TileEntry {
    pub offset: u64,
    pub length: u32,
    pub compressed: bool,
}

/// 一个已解析的 .terrainpack 归档（头部 + 目录常驻内存，瓦片按需定位读）。
pub struct TerrainPack {
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub header: serde_json::Value,
    /// slippy（XYZ 顶行原点）y 键 —— 与切片网格同口径；TMS 翻转在服务层做。
    pub dir: HashMap<(u32, u32, u32), TileEntry>,
}

impl TerrainPack {
    /// 解析归档头部与目录。瓦片 blob 不读入内存。
    pub fn load(path: PathBuf) -> std::io::Result<TerrainPack> {
        let meta = std::fs::metadata(&path)?;
        let total = meta.len();
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        let f = File::open(&path)?;
        let mut head = [0u8; 10];
        read_exact_at(&f, &mut head, 0)?;
        if &head[0..4] != b"GWTQ" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad magic (expect GWTQ)",
            ));
        }
        // magic(0..4) + version(4..6) + headerLen(6..10) + JSON(10..10+hlen)
        // + dirCount(10+hlen ..) + 条目 + blob。
        let header_len = u32::from_le_bytes(head[6..10].try_into().unwrap()) as usize;
        let mut json = vec![0u8; header_len];
        read_exact_at(&f, &mut json, 10)?;
        let header: serde_json::Value = serde_json::from_slice(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut cnt = [0u8; 4];
        read_exact_at(&f, &mut cnt, (10 + header_len) as u64)?;
        let dir_count = u32::from_le_bytes(cnt) as usize;

        let dir_off = 14 + header_len;
        let mut dir_bytes = vec![0u8; dir_count * 25];
        read_exact_at(&f, &mut dir_bytes, dir_off as u64)?;

        let mut dir = HashMap::with_capacity(dir_count);
        for i in 0..dir_count {
            let b = &dir_bytes[i * 25..(i + 1) * 25];
            let z = u32::from_le_bytes(b[0..4].try_into().unwrap());
            let x = u32::from_le_bytes(b[4..8].try_into().unwrap());
            let y = u32::from_le_bytes(b[8..12].try_into().unwrap());
            let offset = u64::from_le_bytes(b[12..20].try_into().unwrap());
            let length = u32::from_le_bytes(b[20..24].try_into().unwrap());
            // 越界条目直接丢弃（不因单个坏条目拒绝整个归档）。
            if offset.saturating_add(length as u64) > total {
                continue;
            }
            dir.insert(
                (z, x, y),
                TileEntry {
                    offset,
                    length,
                    compressed: b[24] == 1,
                },
            );
        }
        Ok(TerrainPack {
            path,
            mtime,
            header,
            dir,
        })
    }

    /// 读一个瓦片的原始字节。gzip blob 原样返回（服务端按
    /// `Content-Encoding: gzip` 透传，不做二次解压/压缩）。
    pub fn read_tile(&self, z: u32, x: u32, y: u32) -> std::io::Result<Option<Vec<u8>>> {
        let Some(e) = self.dir.get(&(z, x, y)) else {
            return Ok(None);
        };
        let f = File::open(&self.path)?;
        let mut buf = vec![0u8; e.length as usize];
        read_exact_at(&f, &mut buf, e.offset)?;
        Ok(Some(buf))
    }

    pub fn min_zoom(&self) -> u32 {
        self.header["minzoom"].as_u64().unwrap_or(0) as u32
    }
    pub fn max_zoom(&self) -> u32 {
        self.header["maxzoom"].as_u64().unwrap_or(0) as u32
    }
    /// true = 等经纬度网格（layer.json 声明 EPSG:4326），false = Web 墨卡托。
    pub fn geodetic(&self) -> bool {
        self.header["tile_matrix"].as_i64().unwrap_or(0) == 1
    }
    pub fn with_normals(&self) -> bool {
        self.header["with_normals"].as_bool().unwrap_or(false)
    }
    /// `[west, south, east, north]`（度）。
    pub fn bounds(&self) -> [f64; 4] {
        let b = &self.header["bounds"];
        let g = |i: usize| b[i].as_f64().unwrap_or(0.0);
        [g(0), g(1), g(2), g(3)]
    }
}

/// 归档登记簿：`<catalog.root>/terrain/*.terrainpack`，一个文件 = 一个服务
/// （服务名 = 文件去扩展名）。
#[derive(Default)]
pub struct TerrainRegistry {
    map: HashMap<String, Arc<TerrainPack>>,
}

impl TerrainRegistry {
    /// 扫描目录并解析全部归档；单个坏文件只产生一条 warning。
    pub fn load_dir(dir: &Path) -> (TerrainRegistry, Vec<String>) {
        let mut reg = TerrainRegistry::default();
        let mut errors = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return (reg, errors), // 目录不存在 = 尚无地形服务
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("terrainpack") {
                continue;
            }
            let name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            match TerrainPack::load(p.clone()) {
                Ok(pack) => {
                    tracing::debug!(
                        name = %name,
                        tiles = pack.dir.len(),
                        "terrainpack loaded"
                    );
                    reg.map.insert(name, Arc::new(pack));
                }
                Err(e) => errors.push(format!("terrainpack {}: {e}", p.display())),
            }
        }
        (reg, errors)
    }

    pub fn get(&self, name: &str) -> Option<Arc<TerrainPack>> {
        self.map.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.map.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}
