//! 目录导入器：把 `files` 后端的 XML 目录灌入共享库 `sys_published_layer`。
//!
//! 目的：让 manager 成为唯一写入方之后，map-api 能以 `catalog.backend = "db"`
//! 服役（唯一真理源 = 共享库），而不再依赖 `<catalog.root>/<kind>/*.xml`。
//!
//! 语义与 `files` 路径**完全一致**：两者都用同一个 `config::parse_layer_doc`
//! 解析 XML，因此导入库的 `service_doc` 与 XML 文档描述的是同一个图层。
//!
//! 一次性迁移工具：
//!
//! ```text
//! map-api catalog-import --config config/mapserver.xml \
//!     --dsn "postgres://postgres:123456@localhost:5434/mapserver_manager" \
//!     --workspace mapserver [--dry-run]
//! ```
//!
//! 幂等：按 `layer_name`（未删除行）upsert，重复执行结果一致。

use std::collections::{BTreeMap, BTreeSet};

use gisserver::pg_source::PgPool;
use serde_json::Value;
use gisserver::wms_common::layer::Layer;

use crate::config::{parse_layer_doc, CatalogOpts, GridDef, ServerConfig};

/// 导入结果摘要。
#[derive(Debug, Default)]
pub struct ImportReport {
    /// 归属工作区 id（`sys_workspace.id`）。
    pub workspace_id: i64,
    /// 扫描到的图层文件数。
    pub scanned: usize,
    /// 新插入的行数。
    pub inserted: usize,
    /// 覆盖更新的行数。
    pub updated: usize,
    /// 跳过的文件及原因。
    pub skipped: Vec<String>,
    /// 是否为只读演练（未写库）。
    pub dry_run: bool,
}

/// 导入器。
pub struct Importer {
    pool: PgPool,
    workspace_id: i64,
    dry_run: bool,
}

/// 协议集合：按数据源类型推导 manager 侧要勾选的发布协议。
fn service_types(kind: &str) -> &'static str {
    match kind {
        "cog" => "wms,wmts,xyz",
        "vector" => "wms,wfs,mvt,oaf",
        "pmtiles" => "xyz,mvt,wmts",
        "mbtiles" => "xyz,mvt,wmts",
        // ArcGIS 预切片缓存只出瓦片：无栅格波段不参与 WMS GetMap，也不是矢量。
        "arcgis" => "wmts,xyz",
        "postgis" => "wms,wfs,mvt,oaf",
        _ => "",
    }
}

impl Importer {
    /// 连接目标库并解析（get-or-create）归属工作区。
    pub async fn open(
        dsn: &str,
        workspace: &str,
        dry_run: bool,
    ) -> anyhow::Result<(Importer, Vec<String>)> {
        let warnings: Vec<String> = Vec::new();
        // 不做 PostGIS 探针：manager 库可能没装扩展，目录导入只需要普通表。
        let pool = PgPool::new_plain(dsn, 2)
            .await
            .map_err(|e| anyhow::anyhow!("连接目录库失败：{e}"))?;
        let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        // 表不存在时给出明确指引（先执行 manager 的 map.sql）。
        let exists: bool = c
            .query_one(
                "select exists (
                     select 1 from information_schema.tables
                     where table_schema = current_schema()
                       and table_name = 'sys_published_layer'
                 )",
                &[],
            )
            .await
            .map_err(|e| anyhow::anyhow!("检查 sys_published_layer 失败：{e}"))?
            .get(0);
        if !exists {
            anyhow::bail!(
                "目标库缺少 sys_published_layer 表：请先执行 manager 的 backend/sql/map.sql"
            );
        }
        // 工作区：不存在则创建（同名未删除工作区唯一）。
        let workspace_id: i64 = c
            .query_one(
                "insert into sys_workspace (name, title, del_flag) values ($1, $1, 0)
                 on conflict (name) where del_flag = 0 do update
                 set title = excluded.title, update_time = now()
                 returning id",
                &[&workspace],
            )
            .await
            .map_err(|e| anyhow::anyhow!("准备工作区 '{workspace}' 失败：{e}"))?
            .get(0);
        drop(c);
        Ok((
            Importer {
                pool,
                workspace_id,
                dry_run,
            },
            warnings,
        ))
    }

    /// 把配置目录下的图层 XML 全部导入 `sys_published_layer`。
    pub async fn import(&self, config: &ServerConfig) -> anyhow::Result<ImportReport> {
        let mut report = ImportReport {
            workspace_id: self.workspace_id,
            dry_run: self.dry_run,
            ..Default::default()
        };
        let root = std::path::Path::new(&config.catalog.root);
        let mut dirs: Vec<std::path::PathBuf> = match std::fs::read_dir(root) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(e) => {
                anyhow::bail!("读取目录 {} 失败：{e}", root.display());
            }
        };
        dirs.sort();

        let mut seen: BTreeSet<String> = BTreeSet::new();
        for dir in dirs {
            let dir_name = dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let want_type = match CatalogOpts::dir_source_type(&dir_name) {
                Some(t) => t,
                None => continue, // 3dtiles / terrain / styles 等不是图层目录
            };
            let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(&dir) {
                Ok(it) => it
                    .flatten()
                    .map(|f| f.path())
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xml"))
                    .collect(),
                Err(e) => {
                    report
                        .skipped
                        .push(format!("读取目录 {} 失败：{e}", dir.display()));
                    continue;
                }
            };
            files.sort();
            for file in files {
                report.scanned += 1;
                let text = match std::fs::read_to_string(&file) {
                    Ok(t) => t,
                    Err(e) => {
                        report
                            .skipped
                            .push(format!("读取 {} 失败：{e}", file.display()));
                        continue;
                    }
                };
                let parsed = match parse_layer_doc(&text) {
                    Ok(p) => p,
                    Err(e) => {
                        report
                            .skipped
                            .push(format!("解析 {} 失败：{e}", file.display()));
                        continue;
                    }
                };
                let mut layer = parsed.layer;
                if let Some(w) = parsed.grids_warning {
                    report.skipped.push(format!("{}: {w}", file.display()));
                }
                let got_type = crate::state::source_kind(&layer.source);
                if got_type != want_type {
                    report.skipped.push(format!(
                        "{}: source 类型 '{got_type}' 与目录 '{want_type}' 不符",
                        file.display()
                    ));
                    continue;
                }
                if !seen.insert(layer.name.clone()) {
                    report
                        .skipped
                        .push(format!("{}: 图层名重复 '{}'", file.display(), layer.name));
                    continue;
                }
                // 向量图层：未显式指定 sld 时，探测同目录同名的 .sld 兄弟文件
                // （与 files 后端规则一致）。
                if layer.sld.is_none() && want_type == "vector" {
                    let sld = file.with_extension("sld");
                    if sld.exists() {
                        layer.sld = Some(sld.to_string_lossy().replace('\\', "/"));
                    }
                }
                let doc = service_doc(&layer, &parsed.grids)?;
                let kind = crate::state::source_kind(&layer.source);
                if self.dry_run {
                    report.updated += 1;
                    continue;
                }
                let inserted = self
                    .upsert(&layer, &doc, kind)
                    .await
                    .map_err(|e| anyhow::anyhow!("写入 {} 失败：{e}", file.display()))?;
                if inserted {
                    report.inserted += 1;
                } else {
                    report.updated += 1;
                }
            }
        }
        Ok(report)
    }

    /// upsert 一行 `sys_published_layer`；返回 true = 新插入，false = 覆盖更新。
    ///
    /// 冲突时**只同步定义**（title/src_crs/bbox/协议/类型/service_doc），不动
    /// `state` / `del_flag` / `workspace_id` / `resource_id`：这四项归 manager
    /// 的生命周期与归属管理所有。若导入器把 `state` 覆写成 'active'，已被
    /// manager 删除（`state='deleted'`）的服务会被「复活」，在管理界面上重新
    /// 出现——属于越权操作。
    async fn upsert(&self, layer: &Layer, doc: &Value, kind: &str) -> anyhow::Result<bool> {
        let bbox: Option<Vec<f64>> = layer.extent.map(|e| vec![e[0], e[1], e[2], e[3]]);
        let doc_text = serde_json::to_string(doc)?;
        let c = self.pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        // service_doc 以 text 传入再转 jsonb：tokio-postgres 未启用
        // serde_json 特性，用 `$n::text::jsonb` 显式指定参数类型。
        let inserted: bool = c
            .query_one(
                "insert into sys_published_layer
                   (workspace_id, resource_id, layer_name, title, src_crs, bbox,
                    service_types, mapserver_kind, state, del_flag, remark, service_doc)
                 values ($1, null, $2, $3, $4, $5, $6, $7, 'active', 0, $8, $9::text::jsonb)
                 on conflict (layer_name) where del_flag = 0 do update set
                   title = excluded.title,
                   src_crs = excluded.src_crs,
                   bbox = excluded.bbox,
                   service_types = excluded.service_types,
                   mapserver_kind = excluded.mapserver_kind,
                   remark = excluded.remark,
                   service_doc = excluded.service_doc,
                   update_time = now()
                 returning (xmax = 0) as inserted",
                &[
                    &self.workspace_id,
                    &layer.name,
                    &layer.title,
                    &layer.src_crs,
                    &bbox,
                    &service_types(kind),
                    &kind,
                    &"XML 目录导入（files → db 迁移）",
                    &doc_text,
                ],
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .get(0);
        Ok(inserted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 往返不变式：XML → service_doc → Layer 必须还原出同一个图层。
    ///
    /// 这是 db 后端能等价替换 files 后端的核心保证——若两者对同一份 XML
    /// 的理解出现偏差，导入后的服务集合就会与迁移前不一致。
    #[test]
    fn service_doc_round_trips_to_same_layer() {
        let xml = r#"<layer name="world" title="World" src_crs="EPSG:4326" tile_px="256">
              <source type="cog" path="fixtures/sample.tif"/>
              <grids><item>WebMercatorQuad</item></grids>
            </layer>"#;
        let parsed = parse_layer_doc(xml).expect("XML 应可解析");
        let doc = service_doc(&parsed.layer, &parsed.grids).expect("应可序列化");

        // 还原路径与 db_catalog::load_catalog 完全一致：先摘 custom_grids，
        // 剩余部分按 Layer 反序列化。
        let mut val = doc.clone();
        let grids = val
            .get("custom_grids")
            .cloned()
            .map(|g| serde_json::from_value::<BTreeMap<String, GridDef>>(g).unwrap())
            .unwrap_or_default();
        if let Some(m) = val.as_object_mut() {
            m.remove("custom_grids");
        }
        let restored: Layer = serde_json::from_value(val).expect("应可反序列化为 Layer");

        assert_eq!(restored.name, parsed.layer.name);
        assert_eq!(restored.title, parsed.layer.title);
        assert_eq!(restored.src_crs, parsed.layer.src_crs);
        assert_eq!(restored.tile_px, parsed.layer.tile_px);
        assert_eq!(restored.grids, parsed.layer.grids);
        assert_eq!(restored.source, parsed.layer.source);
        assert!(grids.is_empty(), "本例未定义 custom_grids");
    }

    /// 自定义网格必须随图层一起进入 service_doc（读取方按图层取用）。
    #[test]
    fn service_doc_carries_custom_grids() {
        let xml = r#"<layer name="g" src_crs="EPSG:4326">
              <source type="cog" path="a.tif"/>
              <grids><item>mygrid</item></grids>
              <custom_grids>
                <item id="mygrid" crs="EPSG:4326" tile_px="256">
                  <origin><item>-180</item><item>90</item></origin>
                  <extent><item>-180</item><item>-90</item><item>180</item><item>90</item></extent>
                  <resolutions><item>0.703125</item></resolutions>
                </item>
              </custom_grids>
            </layer>"#;
        let parsed = parse_layer_doc(xml).expect("XML 应可解析");
        assert_eq!(parsed.grids.len(), 1, "应解出 1 个自定义网格");
        assert!(parsed.grids.contains_key("mygrid"));

        let doc = service_doc(&parsed.layer, &parsed.grids).expect("应可序列化");
        let grids = serde_json::from_value::<BTreeMap<String, GridDef>>(
            doc.get("custom_grids").cloned().expect("doc 应含 custom_grids"),
        )
        .expect("custom_grids 应可反序列化");
        assert_eq!(grids.len(), 1);
        assert_eq!(grids["mygrid"].crs, "EPSG:4326");
        assert_eq!(grids["mygrid"].resolutions, vec![0.703125]);
    }

    /// 协议集合按数据源类型推导（manager 侧发布勾选项）。
    #[test]
    fn service_types_depend_on_source_kind() {
        assert_eq!(service_types("cog"), "wms,wmts,xyz");
        assert_eq!(service_types("arcgis"), "wmts,xyz");
        assert_eq!(service_types("postgis"), "wms,wfs,mvt,oaf");
        assert_eq!(service_types("3dtiles"), "");
    }
}

/// 构造 `service_doc`：图层本体 + 该图层自带的 `custom_grids`。
///
/// 与 `db_catalog::load_catalog` 的读取约定严格对应：读取方先摘出
/// `custom_grids` 合并为全局网格表，再把剩余部分当作 `Layer` 反序列化。
pub fn service_doc(layer: &Layer, grids: &BTreeMap<String, GridDef>) -> anyhow::Result<Value> {
    let mut doc = serde_json::to_value(layer)?;
    if let Some(obj) = doc.as_object_mut() {
        if !grids.is_empty() {
            obj.insert(
                "custom_grids".to_string(),
                serde_json::to_value(grids)?,
            );
        }
    }
    Ok(doc)
}
