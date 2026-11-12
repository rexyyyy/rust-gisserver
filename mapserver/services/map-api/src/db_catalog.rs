//! 数据库目录后端（`catalog.backend = "db"`）：从共享 PostGIS 的
//! `sys_published_layer.service_doc` 读取图层定义，作为服役引擎的唯一真理源。
//!
//! `service_doc` 以 JSONB 存放与 XML 等价的图层定义（含 `custom_grids`），
//! 复用 `xmlcfg` 同一套解析约定，因此与 `files` 后端零语义差异。manager 是
//! 该表的唯一写入方，map-api 仅读。

use std::collections::{BTreeMap, BTreeSet};

use gisserver::pg_source::PgPool;
use serde_json::Value;

use crate::config::GridDef;
use gisserver::wms_common::layer::Layer;

/// 从 `sys_published_layer` 载入全部 `active` 且未删除的图层定义。
///
/// 返回：`(图层列表, 自定义网格定义, 非致命警告)`。单条解析失败会被跳过并告警，
/// 不影响其余图层服役。
pub async fn load_catalog(
    pool: &PgPool,
) -> anyhow::Result<(Vec<Layer>, BTreeMap<String, GridDef>, Vec<String>)> {
    let client = pool
        .get()
        .await
        .map_err(|e| anyhow::anyhow!("catalog db: {e}"))?;
    let rows = client
        .query(
            "SELECT layer_name, service_doc::text \
             FROM sys_published_layer \
             WHERE del_flag = 0 AND state = 'active' AND service_doc IS NOT NULL \
             ORDER BY layer_name",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("load catalog: {e}"))?;

    let mut layers: Vec<Layer> = Vec::new();
    let mut extra_grids: BTreeMap<String, GridDef> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for r in rows {
        let name: String = r.get(0);
        let doc_text: Option<String> = r.get(1);
        let Some(doc_text) = doc_text else { continue };
        let doc_text = crate::xmlcfg::expand_env(&doc_text);
        let mut val: Value = match serde_json::from_str(&doc_text) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!("{name}: service_doc 解析失败：{e}"));
                continue;
            }
        };
        // custom_grids 不属于 Layer 结构，单独抽取（与 collect_layers 一致）。
        if let Some(g) = val.get("custom_grids").cloned() {
            match serde_json::from_value::<BTreeMap<String, GridDef>>(g) {
                Ok(defs) => {
                    for (id, def) in defs {
                        if extra_grids.contains_key(&id) {
                            warnings.push(format!("{name}: 重复自定义网格 '{id}' 已忽略"));
                        } else {
                            extra_grids.insert(id, def);
                        }
                    }
                }
                Err(e) => warnings.push(format!("{name}: custom_grids: {e}")),
            }
        }
        if let Some(m) = val.as_object_mut() {
            m.remove("custom_grids");
        }
        let layer: Layer = match serde_json::from_value(val) {
            Ok(l) => l,
            Err(e) => {
                warnings.push(format!("{name}: 图层定义反序列化失败：{e}"));
                continue;
            }
        };
        if !seen.insert(layer.name.clone()) {
            warnings.push(format!("重复图层名 '{name}' 已忽略"));
            continue;
        }
        layers.push(layer);
    }
    Ok((layers, extra_grids, warnings))
}
