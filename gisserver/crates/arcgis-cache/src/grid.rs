//! `Conf.xml` 的 LOD 阶梯 → **custom grid**（含地图比例尺）推导。
//!
//! # 为什么必须由 Conf.xml 生成网格
//!
//! ArcGIS 预切片缓存的瓦片寻址完全由 Conf.xml 决定：
//! `col = floor((x - TileOrigin.X) / (TileCols * Resolution(z)))`，行同理自左上角
//! 向南。这套参数与任何预设网格（WebMercatorQuad / WorldCRS84Quad）都不等价——
//! 尤其当缓存是 CGCS2000 地理坐标系（EPSG:4490，本仓库 E 盘样例）或高斯-克吕格
//! 投影带（EPSG:4522 等）时，用预设网格去取瓦片会整体错位。因此这里把 Conf.xml
//! 直接摊成一份 custom grid 定义，让浏览端按**数据自身的**分辨率取瓦片。
//!
//! # 范围（extent）的算法
//!
//! 与 GeoWebCache 的 `GridSetBuilder` 一致：左上角固定为 `TileOrigin`，右下按
//! **最细一级**分辨率对齐到整数瓦片（`ceil` 后回推），使自左上与自左下计算的
//! 包络对客户端都成立。`conf.cdi` 给出数据范围；缺失时用 bundle 目录名探测出的
//! 最大行列回推（见 [`crate::bundle::CacheProbe`]）。
//!
//! # 比例尺（ScaleDenominator）
//!
//! `LODInfo/Scale` 是 ArcGIS 按 `像素物理尺寸 = 0.0254 / DPI` 导出的，而 OGC 惯例
//! 用 0.28 mm/像素。DPI=96 时两者相差 `0.00028 / 0.00026458… ≈ 1.0583` 倍——用
//! 0.28 mm 反算会把 z0 的 2.958e8 报成 2.798e8。因此网格必须**携带源 Scale**，
//! WMTS/OGC API 的能力文档按它宣告（见 [`GridSpec::scales`] 与
//! `wms_common::tile::TmLevel::scale_denominator`）。

use std::path::Path;

use wms_common::crs;

use crate::conf::CacheInfo;

/// 一份可直接落到图层 XML `<custom_grids>` 的网格规格。
///
/// 与 `map_api::config::GridDef` 字段一一对应（本 crate 不依赖 map-api，因此
/// 这里是纯数据，由接线层转成 GridDef 或由 [`layer_xml`] 直接输出 XML）。
#[derive(Clone, Debug, PartialEq)]
pub struct GridSpec {
    /// 网格 id（图层 `grids:` 引用的键）。
    pub id: String,
    /// 归一化后的 CRS（`EPSG:4490`）。
    pub crs: String,
    /// 左上角 `[x, y]`（CRS 单位）。
    pub origin: [f64; 2],
    /// 对齐到整数瓦片后的范围 `[minx, miny, maxx, maxy]`。
    pub extent: [f64; 4],
    /// 瓦片像素边长（取自 `TileCols`）。
    pub tile_px: u32,
    /// 分辨率阶梯，索引 = LOD 序号（与 `lods` 同序）。
    pub resolutions: Vec<f64>,
    /// 地图比例尺分母阶梯（源自 `LODInfo/Scale`；为 0 的项表示源未给出）。
    pub scales: Vec<f64>,
    /// 每级矩阵列数（`ceil((extent 宽) / (tile_px * res))`）。
    pub matrix_w: Vec<u32>,
    /// 每级矩阵行数。
    pub matrix_h: Vec<u32>,
    /// 分辨率阶梯对应的层级（`LevelID`，可能不连续）。
    pub levels: Vec<u32>,
}

impl GridSpec {
    /// 层级 → LOD 数组下标（`LevelID` 不连续时用于把请求层级映射到阶梯）。
    pub fn lod_index(&self, level: u32) -> Option<usize> {
        self.levels.iter().position(|l| *l == level)
    }

    /// z 是否落在阶梯内（缓存只切了部分层级时为 false）。
    pub fn has_level(&self, z: u32) -> bool {
        self.lod_index(z).is_some()
    }

    /// 源 Scale 缺失时按 OGC 0.28 mm 规则兜底。
    pub fn scale_or_default(&self, z: u32) -> f64 {
        match self.lod_index(z).map(|i| (self.scales[i], self.resolutions[i])) {
            Some((s, _)) if s > 0.0 => s,
            Some((_, r)) => r * crs::meters_per_unit(&self.crs) / 0.00028,
            None => f64::NAN,
        }
    }
}

/// 由 `Conf.xml` + 数据范围推导网格。`bounds` 为 `conf.cdi` 包络（缓存 CRS 单位）。
pub fn derive(info: &CacheInfo, id: &str, bounds: Option<[f64; 4]>) -> Result<GridSpec, String> {
    derive_inner(info, id, bounds, None)
}

/// `probe`：`conf.cdi` 缺失时，用 bundle 目录名探测出的最大行列回推范围。
pub fn derive_with_probe(
    info: &CacheInfo,
    id: &str,
    bounds: Option<[f64; 4]>,
    probe: Option<(u32, u32, u32)>,
) -> Result<GridSpec, String> {
    derive_inner(info, id, bounds, probe)
}

fn derive_inner(
    info: &CacheInfo,
    id: &str,
    bounds: Option<[f64; 4]>,
    probe: Option<(u32, u32, u32)>,
) -> Result<GridSpec, String> {
    if info.lods.is_empty() {
        return Err("Conf.xml 的 LODInfos 为空，无法生成网格".to_string());
    }
    let [ox, oy] = info.origin;
    let tile_px = info.tile_px() as f64;
    let finest = info.lods.last().unwrap();

    // 右下边界：优先 cdi 包络，其次按探测到的最大行列（在最细层级上）回推。
    // 注意 `bounds` 是 [minx, miny, maxx, maxy]：这里要的右下角是 (maxx, miny)，
    // 误取 bounds[3]（maxy）会把网格下边界收到数据北缘，行矩阵数骤减。
    let (maxx, miny) = match (bounds, probe) {
        (Some([_, mn, mx, _]), _) => (mx, mn),
        (None, Some((z, col, row))) => {
            let Some(i) = info.lods.iter().position(|l| l.level == z) else {
                return Err(format!(
                    "探测层级 L{z:02} 不在 Conf.xml 的 LOD 阶梯内（{}..{}）",
                    info.min_zoom(),
                    info.max_zoom()
                ));
            };
            let span = info.lods[i].resolution * tile_px;
            (ox + (col + 1) as f64 * span, oy - (row + 1) as f64 * span)
        }
        (None, None) => {
            // 兜底：最粗一级按矩阵数 1 张瓦片起步没有意义，直接要求运维补 cdi。
            return Err(
                "缺少 conf.cdi 且未能从 _alllayers 探测到瓦片范围，无法确定网格范围".to_string(),
            );
        }
    };

    // GridSetBuilder 的对齐：以**最细一级**的瓦片跨度向上取整，使网格高度恰为
    // 整数张瓦片（自左上与自左下起算的客户端都能算对瓦片包络）。
    let span_x = finest.resolution * tile_px;
    let span_y = finest.resolution * info.tile_rows.max(1) as f64;
    let tiles_w = (((maxx - ox) / span_x).ceil()).max(1.0);
    let tiles_h = (((oy - miny) / span_y).ceil()).max(1.0);
    let width = tiles_w * span_x;
    let height = tiles_h * span_y;
    let extent = [ox, oy - height, ox + width, oy];

    let mut resolutions = Vec::with_capacity(info.lods.len());
    let mut scales = Vec::with_capacity(info.lods.len());
    let mut levels = Vec::with_capacity(info.lods.len());
    let mut matrix_w = Vec::with_capacity(info.lods.len());
    let mut matrix_h = Vec::with_capacity(info.lods.len());
    for lod in &info.lods {
        let (sw, sh) = (lod.resolution * tile_px, lod.resolution * info.tile_rows.max(1) as f64);
        resolutions.push(lod.resolution);
        scales.push(lod.scale);
        levels.push(lod.level);
        matrix_w.push((((width / sw).ceil()) as u32).max(1));
        matrix_h.push((((height / sh).ceil()) as u32).max(1));
    }

    Ok(GridSpec {
        id: id.to_string(),
        crs: crs::normalize(&info.crs),
        origin: [ox, oy],
        extent,
        tile_px: info.tile_px(),
        resolutions,
        scales,
        matrix_w,
        matrix_h,
        levels,
    })
}

/// 层级在网格内的瓦片跨度 `[span_x, span_y]`（CRS 单位）。
pub fn tile_span(spec: &GridSpec, z: u32) -> Option<(f64, f64)> {
    let i = spec.lod_index(z)?;
    Some((
        spec.resolutions[i] * spec.tile_px as f64,
        spec.resolutions[i] * spec.tile_px as f64,
    ))
}

/// `col/row`（自左上角，层级 `z`）对应的瓦片包络，`None` = 越出网格。
pub fn tile_bounds(spec: &GridSpec, z: u32, col: u32, row: u32) -> Option<[f64; 4]> {
    let i = spec.lod_index(z)?;
    if col >= spec.matrix_w[i] || row >= spec.matrix_h[i] {
        return None;
    }
    let span = spec.resolutions[i] * spec.tile_px as f64;
    let minx = spec.origin[0] + col as f64 * span;
    let maxy = spec.origin[1] - row as f64 * span;
    Some([minx, maxy - span, minx + span, maxy])
}

/// 把包络换算成该层级下的瓦片行列（自左上角）。
pub fn tile_indices(spec: &GridSpec, z: u32, x: f64, y: f64) -> Option<(u32, u32)> {
    let i = spec.lod_index(z)?;
    let span = spec.resolutions[i] * spec.tile_px as f64;
    let col = (x - spec.origin[0]) / span;
    let row = (spec.origin[1] - y) / span;
    if col < 0.0 || row < 0.0 {
        return None;
    }
    Some((col.floor() as u32, row.floor() as u32))
}

// ── 文本产出：让 Conf.xml 能"一步生成图层配置" ─────────────────────────────

/// 数字输出：f64 的 Display 是**最短可回读**十进制（不带指数），因此
/// `0.70391441568404511` 这类 LOD 分辨率原样落纸、被 xmlcfg 的 `parse::<f64>()`
/// 读回后逐位相等——写配置这一步不能引入任何精度损失。
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let s = format!("{v}");
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

fn items(vals: &[String]) -> String {
    vals.iter().map(|v| format!("<item>{v}</item>")).collect::<Vec<_>>().join("")
}

/// `<custom_grids>` 片段（一个 `<item id=…>`），可直接贴进 `config/arcgis/*.xml`。
///
/// 输出遵循 `map-api::xmlcfg` 的约定：带 `id` 属性的子元素 → 以 id 为键的映射，
/// 全 `<item>` 子元素 → 数组（`origin` / `extent` / `resolutions` / `scales` /
/// `levels`）。`levels` 把 `LevelID` 原样落地：图层阶梯若不以 0 起（缓存只从
/// L06 切起），靠数组下标当层级会整体错一级。
pub fn custom_grid_xml(spec: &GridSpec, indent: &str) -> String {
    let res: Vec<String> = spec.resolutions.iter().map(|v| num(*v)).collect();
    let sca: Vec<String> = spec.scales.iter().map(|v| num(*v)).collect();
    let ext: Vec<String> = spec.extent.iter().map(|v| num(*v)).collect();
    let ori: Vec<String> = spec.origin.iter().map(|v| num(*v)).collect();
    let lv: Vec<String> = spec.levels.iter().map(|v| v.to_string()).collect();
    let i = format!("{indent}  ");
    let j = format!("{i}  ");
    format!(
        "<custom_grids>\n\
         {i}<item id=\"{id}\" crs=\"{crs}\" tile_px=\"{px}\">\n\
         {j}<origin>{o}</origin>\n\
         {j}<extent>{e}</extent>\n\
         {j}<levels>{l}</levels>\n\
         {j}<resolutions>{r}</resolutions>\n\
         {j}<scales>{s}</scales>\n\
         {i}</item>\n\
         {indent}</custom_grids>",
        id = spec.id,
        crs = spec.crs,
        px = spec.tile_px,
        o = items(&ori),
        e = items(&ext),
        l = items(&lv),
        r = items(&res),
        s = items(&sca),
    )
}

/// 完整图层 XML（`config/arcgis/<name>.xml` 的形状）。
///
/// `cache_path` 指向缓存服务目录（含 `Conf.xml`，或它的一级图层子目录）。
/// `grids:` 引用 [`custom_grid_xml`] 生成的 id，因此该文件落地即可服务。
pub fn layer_xml(name: &str, title: &str, cache_path: &Path, spec: &GridSpec) -> String {
    let p = cache_path.to_string_lossy().replace('\\', "/");
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <layer name=\"{name}\" title=\"{title}\" src_crs=\"{crs}\" tile_px=\"{px}\">\n\
         \x20 <source type=\"arcgis\" path=\"{p}\"/>\n\
         \x20 <grids><item>{gid}</item></grids>\n\
         {}\n\
         </layer>\n",
        custom_grid_xml(spec, "  "),
        name = name,
        title = title.replace('"', "&quot;"),
        crs = spec.crs,
        px = spec.tile_px,
        gid = spec.id,
        p = p,
    )
}

/// [`custom_grid_xml`] 的回读结果（与 `map_api::config::GridDef` 同形）。
#[derive(Clone, Debug, PartialEq)]
pub struct EmittedGrid {
    pub id: String,
    pub crs: String,
    pub origin: [f64; 2],
    pub extent: [f64; 4],
    pub tile_px: u32,
    pub resolutions: Vec<f64>,
    pub scales: Vec<f64>,
    /// `LevelID` 阶梯；空 = 旧配置（按下标 0..n-1 理解）。
    pub levels: Vec<u32>,
}

/// 解析一份 `<custom_grids>` 片段（根元素或 `<item>` 均可）。
///
/// 存在的理由不是「再实现一个 GridDef 读取器」，而是让运维工具能**反向校验**
/// 已落地的图层 XML：配置里的阶梯与缓存 `Conf.xml` 当前阶梯不一致（数据重切、
/// 手工改过数字）时，瓦片会整体错位而不报错。
pub fn parse_custom_grid_xml(text: &str) -> Result<EmittedGrid, String> {
    let root = crate::xmlscan::parse(text)?;
    // 三种根形态：`<item>` 本体、`<custom_grids>` 片段、完整 `<layer>` 文档，
    // 统一收敛到同一个 `<item>`，回读与校验只走一条路径。
    let item: &crate::xmlscan::Elem = if root.name.eq_ignore_ascii_case("item") {
        &root
    } else if root.name.eq_ignore_ascii_case("custom_grids") {
        root.children_named("item")
            .first()
            .copied()
            .ok_or("custom_grids 下没有 <item>")?
    } else {
        // <layer> 根：往下找 custom_grids/item
        root.find("custom_grids")
            .and_then(|cg| cg.child("item"))
            .ok_or("未找到 <custom_grids><item …>")?
    };
    let attrs = |n: &str| {
        item.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.clone())
    };
    let id = attrs("id").ok_or("custom grid 缺少 id")?;
    let crs_raw = attrs("crs").ok_or("custom grid 缺少 crs")?;
    let tile_px = attrs("tile_px")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .ok_or("custom grid 缺少 tile_px")?;
    let nums = |name: &str| -> Option<Vec<f64>> {
        let el = item.child(name)?;
        let vals: Vec<f64> = el
            .children_named("item")
            .iter()
            .filter_map(|i| i.text.trim().parse::<f64>().ok())
            .collect();
        if vals.is_empty() {
            None
        } else {
            Some(vals)
        }
    };
    let need = |v: Option<Vec<f64>>, name: &str| -> Result<Vec<f64>, String> {
        v.ok_or_else(|| format!("custom grid '{id}' 缺少 {name}"))
    };
    let origin = need(nums("origin"), "origin")?;
    let extent = need(nums("extent"), "extent")?;
    let resolutions = need(nums("resolutions"), "resolutions")?;
    let scales = nums("scales").unwrap_or_default();
    // levels 可缺失（旧配置）；存在时必与 resolutions 同长，否则阶梯与层级对不上。
    let levels: Vec<u32> = nums("levels")
        .unwrap_or_default()
        .iter()
        .map(|v| *v as u32)
        .collect();
    if !levels.is_empty() && levels.len() != resolutions.len() {
        return Err(format!(
            "custom grid '{id}' 的 levels({}) 与 resolutions({}) 个数不一致",
            levels.len(),
            resolutions.len()
        ));
    }
    let (origin, extent) = match (origin, extent) {
        (o, e) if o.len() == 2 && e.len() == 4 => ([o[0], o[1]], [e[0], e[1], e[2], e[3]]),
        _ => return Err(format!("custom grid '{id}' 的 origin/extent 元素个数不对")),
    };
    Ok(EmittedGrid {
        id,
        crs: crs::normalize(&crs_raw),
        origin,
        extent,
        tile_px,
        resolutions,
        scales,
        levels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conf::CacheInfo;

    /// E 盘真实缓存（TD_DZZHYHD_G_2025_GCS2000）的 Conf.xml 关键参数固化。
    fn real_info() -> CacheInfo {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/TD_Conf.xml")).unwrap();
        CacheInfo::from_xml(&text).unwrap()
    }

    const CDI: [f64; 4] = [102.23285257581954, 24.438884054211364, 102.83157693, 24.757753353357];

    #[test]
    fn lod_ladder_becomes_grid_with_scales() {
        let info = real_info();
        let spec = derive(&info, "arcgis_td", Some(CDI)).unwrap();
        assert_eq!(spec.crs, "EPSG:4490");
        assert_eq!(spec.origin, [-180.0, 90.0]);
        assert_eq!(spec.tile_px, 256);
        assert_eq!(spec.levels.len(), 19);
        assert_eq!(*spec.levels.last().unwrap(), 18);
        // 比例尺原样携带源 Scale（0.28mm 反算会差 5.8%）
        assert!((spec.scales[0] - 295829355.454566).abs() < 1e-3);
        assert!((spec.scale_or_default(0) - 295829355.454566).abs() < 1e-3);
        let ogc = spec.resolutions[0] * 111319.4907932736 / 0.00028;
        assert!((ogc / spec.scales[0] - 0.9459).abs() < 0.001, "两口径差异应约 5.8%");
        // 范围左上角为 origin，右下角按最细级对齐到整瓦
        assert!((spec.extent[0] + 180.0).abs() < 1e-12);
        assert!((spec.extent[3] - 90.0).abs() < 1e-12);
        // 对齐后的范围必须仍包住数据四角（把 maxy 当成 miny 用会让南界上提）
        assert!(spec.extent[1] <= CDI[1] + 1e-12, "南界 {}/{}", spec.extent[1], CDI[1]);
        assert!(spec.extent[2] >= CDI[2] - 1e-12, "东界 {}/{}", spec.extent[2], CDI[2]);
        let finest = spec.resolutions[18];
        let span = finest * 256.0;
        let tiles_w = (spec.extent[2] - spec.extent[0]) / span;
        assert!((tiles_w - tiles_w.round()).abs() < 1e-6, "宽度必须是整瓦");
        // 层级矩阵数随深度 4 倍增长（同一 extent 下 ceil）
        assert!(spec.matrix_w[0] < spec.matrix_w[18] && spec.matrix_h[0] < spec.matrix_h[18]);
        assert!(spec.matrix_w[18] >= ((CDI[2] - CDI[0]) / span).ceil() as u32);
    }

    /// 数据必须落在网格内，且行列换算与包络换算互为逆运算（瓦片寻址正确性的核心）。
    #[test]
    fn tile_index_roundtrip_covers_data_bounds() {
        let info = real_info();
        let spec = derive(&info, "g", Some(CDI)).unwrap();
        let z = 10;
        // 数据四角所在瓦片
        for (x, y) in [(CDI[0], CDI[1]), (CDI[2], CDI[3]), (CDI[0], CDI[3]), (CDI[2], CDI[1])] {
            let (c, r) = tile_indices(&spec, z, x, y).unwrap();
            let b = tile_bounds(&spec, z, c, r).unwrap_or_else(|| {
                panic!(
                    "({x},{y}) → col {c} row {r} 越出矩阵 {}x{}; extent {:?} res {}",
                    spec.matrix_w[z as usize],
                    spec.matrix_h[z as usize],
                    spec.extent,
                    spec.resolutions[z as usize]
                )
            });
            assert!(b[0] <= x && x <= b[2] && b[1] <= y && y <= b[3], "({x},{y}) 应在 {b:?} 内");
            // 越出网格的行列必须被拒绝
            assert!(tile_bounds(&spec, z, spec.matrix_w[10], 0).is_none());
        }
        assert_eq!(spec.lod_index(0), Some(0));
        assert_eq!(spec.lod_index(19), None);
        assert!(!spec.has_level(19));
        let (sx, sy) = tile_span(&spec, 18).unwrap();
        assert!(sx > 0.0 && (sx - sy).abs() < 1e-18);
    }

    /// `conf.cdi` 缺失时用探测行列回推（真实数据没有 cdi 的产出并不少见）。
    #[test]
    fn probe_fallback_when_no_cdi() {
        let info = real_info();
        let spec = derive_with_probe(&info, "g", None, Some((18, 599, 300))).unwrap();
        let span = info.lods[18].resolution * 256.0;
        assert!((spec.extent[2] - (-180.0 + 600.0 * span)).abs() < 1e-9);
        assert!((spec.extent[1] - (90.0 - 301.0 * span)).abs() < 1e-9);
        // 探测层级不在阶梯内要报错（Conf.xml 与 _alllayers 不一致）
        assert!(derive_with_probe(&info, "g", None, Some((99, 1, 1))).is_err());
        assert!(derive(&info, "g", None).is_err());
    }

    #[test]
    fn emits_layer_xml_with_lod_scales() {
        let info = real_info();
        let spec = derive(&info, "arcgis_td_dzzhyhd", Some(CDI)).unwrap();
        let xml = layer_xml("td_dzzhyhd", "专题图层", Path::new("E:/data_km/arcgis_cah/TD_DZZHYHD_G_2025_GCS2000"), &spec);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<layer name=\"td_dzzhyhd\" title=\"专题图层\" src_crs=\"EPSG:4490\" tile_px=\"256\">"));
        assert!(xml.contains("<source type=\"arcgis\" path=\"E:/data_km/arcgis_cah/TD_DZZHYHD_G_2025_GCS2000\"/>"));
        assert!(xml.contains("<item>arcgis_td_dzzhyhd</item>"));
        assert!(xml.contains("<custom_grids>"));
        assert!(xml.contains("id=\"arcgis_td_dzzhyhd\""));
        // 19 级分辨率 + 19 级比例尺，且数字可被原样读回（不丢精度）
        assert!(xml.contains(&format!("<item>{}</item>", info.lods[0].resolution)));
        assert!(xml.contains(&format!("<item>{}</item>", info.lods[0].scale)));
        assert!(xml.contains(&format!("<item>{}</item>", info.lods[18].resolution)));
        // LevelID 阶梯也要原样落地（不连续时下标当层级会整体错级）
        assert!(xml.contains("<levels><item>0</item>"));
        assert!(xml.contains("<item>18</item></levels>"));
        let round = parse_custom_grid_xml(&custom_grid_xml(&spec, "")).unwrap();
        assert_eq!(round.resolutions, spec.resolutions, "XML 往返必须逐位相等");
        assert_eq!(round.scales, spec.scales);
        assert_eq!(round.levels, spec.levels);
        assert_eq!(round.origin, spec.origin);
        assert_eq!(round.extent, spec.extent);
        assert_eq!(round.tile_px, spec.tile_px);
        assert_eq!(round.id, "arcgis_td_dzzhyhd");
        assert_eq!(round.crs, "EPSG:4490");
        // 图层整文档形式也能回读（运维反向校验走同一条路径）
        assert_eq!(parse_custom_grid_xml(&xml).unwrap().resolutions, spec.resolutions);
        // 缺 scales 是老配置（GridDef 无该字段），应视为空而非报错
        let no_scale = xml.replace("<scales>", "<other>").replace("</scales>", "</other>");
        assert!(parse_custom_grid_xml(&no_scale).unwrap().scales.is_empty());
    }
}
