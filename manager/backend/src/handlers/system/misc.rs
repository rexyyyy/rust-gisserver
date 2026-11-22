//! 岗位、字典（类型/数据）、参数设置、通知公告。
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::state::{execute, query, AppState};
use crate::util::{Qb, jopt_i32, jopt_i64, jopt_str, jstr, page_of, qopt};

// ------------------------------ 岗位 ------------------------------

/// GET /system/post/list
pub async fn post_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("post_code", qopt(&q, "postCode").as_deref());
    qb.like("post_name", qopt(&q, "postName").as_deref());
    if let Some(s) = qopt(&q, "status") {
        qb.when("status = {}", Some(s));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_post{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_post{where_sql} ORDER BY post_sort LIMIT {lph} OFFSET {oph}",
        crate::models::POST_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::PostVO> = rows.iter().map(crate::models::post_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

pub async fn post_get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_post WHERE post_id = $1", crate::models::POST_COLS);
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("岗位不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::post_from_row(r) })))
}

pub async fn post_add(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let sql = "INSERT INTO sys_post (post_id, dept_id, post_code, post_name, post_sort, status, remark, create_time) \
               VALUES (COALESCE((SELECT max(post_id) FROM sys_post),0) + $1, $2, $3, $4, $5, $6, $7, NOW()) RETURNING post_id";
    let pv = crate::pv![
        1i64,
        jopt_i64(&body, "deptId").unwrap_or(0),
        jstr(&body, "postCode"),
        jstr(&body, "postName"),
        jopt_i32(&body, "postSort").unwrap_or(1),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "remark"),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("post_id").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

pub async fn post_update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "postId").ok_or(AppError::Param("postId 缺失".into()))?;
    let sql = "UPDATE sys_post SET dept_id = $1, post_code = $2, post_name = $3, post_sort = $4, status = $5, remark = $6, update_time = NOW() WHERE post_id = $7";
    let pv = crate::pv![
        jopt_i64(&body, "deptId").unwrap_or(0),
        jstr(&body, "postCode"),
        jstr(&body, "postName"),
        jopt_i32(&body, "postSort").unwrap_or(1),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "remark"),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

pub async fn post_delete(State(state): State<Arc<AppState>>, Path(ids): Path<String>) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        let used: i64 = query(&state.pool, "SELECT count(*) FROM sys_user_post WHERE post_id = $1", &[id])
            .await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
        if used > 0 {
            return Err(AppError::bad("岗位已分配用户，不允许删除"));
        }
        execute(&state.pool, "DELETE FROM sys_post WHERE post_id = $1", &[id]).await?;
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

pub async fn post_option_select(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_post WHERE status = '0' ORDER BY post_sort", crate::models::POST_COLS);
    let rows = query(&state.pool, &sql, &[]).await?;
    let vos: Vec<crate::models::PostVO> = rows.iter().map(crate::models::post_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": vos })))
}

// ------------------------------ 字典类型 ------------------------------

pub async fn dict_type_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("dict_name", qopt(&q, "dictName").as_deref());
    qb.like("dict_type", qopt(&q, "dictType").as_deref());
    if let Some(s) = qopt(&q, "status") {
        qb.when("status = {}", Some(s));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_dict_type{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_dict_type{where_sql} ORDER BY dict_id LIMIT {lph} OFFSET {oph}",
        crate::models::DICT_TYPE_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::DictTypeVO> = rows.iter().map(crate::models::dict_type_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

pub async fn dict_type_get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_dict_type WHERE dict_id = $1", crate::models::DICT_TYPE_COLS);
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("字典类型不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::dict_type_from_row(r) })))
}

pub async fn dict_type_add(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let sql = "INSERT INTO sys_dict_type (dict_id, dict_name, dict_type, remark, create_time) \
               VALUES (COALESCE((SELECT max(dict_id) FROM sys_dict_type),0) + $1, $2, $3, $4, NOW()) RETURNING dict_id";
    let pv = crate::pv![
        1i64,
        jstr(&body, "dictName"),
        jstr(&body, "dictType"),
        jstr(&body, "remark"),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("dict_id").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

pub async fn dict_type_update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "dictId").ok_or(AppError::Param("dictId 缺失".into()))?;
    let sql = "UPDATE sys_dict_type SET dict_name = $1, dict_type = $2, remark = $3, update_time = NOW() WHERE dict_id = $4";
    let pv = crate::pv![
        jstr(&body, "dictName"),
        jstr(&body, "dictType"),
        jstr(&body, "remark"),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

pub async fn dict_type_delete(State(state): State<Arc<AppState>>, Path(ids): Path<String>) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        execute(&state.pool, "DELETE FROM sys_dict_type WHERE dict_id = $1", &[id]).await?;
        execute(&state.pool, "DELETE FROM sys_dict_data WHERE dict_type = (SELECT dict_type FROM sys_dict_type WHERE dict_id = $1)", &[id]).await.ok();
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

pub async fn dict_type_option_select(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_dict_type ORDER BY dict_id", crate::models::DICT_TYPE_COLS);
    let rows = query(&state.pool, &sql, &[]).await?;
    let vos: Vec<crate::models::DictTypeVO> = rows.iter().map(crate::models::dict_type_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": vos })))
}

/// DELETE /system/dict/type/refreshCache（本实现无独立缓存，返回成功保证前端流程完整）
pub async fn dict_refresh(State(_state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    Ok(Json(json!({ "code": 200, "msg": "刷新成功" })))
}

// ------------------------------ 字典数据 ------------------------------

pub async fn dict_data_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("dict_label", qopt(&q, "dictLabel").as_deref());
    if let Some(t) = qopt(&q, "dictType") {
        qb.when("dict_type = {}", Some(t));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_dict_data{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_dict_data{where_sql} ORDER BY dict_sort, dict_code LIMIT {lph} OFFSET {oph}",
        crate::models::DICT_DATA_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::DictDataVO> = rows.iter().map(crate::models::dict_data_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

/// GET /system/dict/data/type/{dictType}
pub async fn dict_data_by_type(
    State(state): State<Arc<AppState>>,
    Path(dict_type): Path<String>,
) -> AppResult<Json<Value>> {
    let sql = format!(
        "SELECT {} FROM sys_dict_data WHERE dict_type = $1 ORDER BY dict_sort",
        crate::models::DICT_DATA_COLS
    );
    let rows = query(&state.pool, &sql, &[&dict_type]).await?;
    let vos: Vec<crate::models::DictDataVO> = rows.iter().map(crate::models::dict_data_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": vos })))
}

pub async fn dict_data_get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_dict_data WHERE dict_code = $1", crate::models::DICT_DATA_COLS);
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("字典数据不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::dict_data_from_row(r) })))
}

pub async fn dict_data_add(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let sql = "INSERT INTO sys_dict_data (dict_code, dict_sort, dict_label, dict_value, dict_type, css_class, list_class, is_default, remark, create_time) \
               VALUES (COALESCE((SELECT max(dict_code) FROM sys_dict_data),0) + $1, $2, $3, $4, $5, $6, $7, $8, $9, NOW()) RETURNING dict_code";
    let pv = crate::pv![
        1i64,
        jopt_i32(&body, "dictSort").unwrap_or(0),
        jstr(&body, "dictLabel"),
        jstr(&body, "dictValue"),
        jstr(&body, "dictType"),
        jstr(&body, "cssClass"),
        jstr(&body, "listClass"),
        jopt_str(&body, "isDefault").unwrap_or_else(|| "N".into()),
        jstr(&body, "remark"),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("dict_code").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

pub async fn dict_data_update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "dictCode").ok_or(AppError::Param("dictCode 缺失".into()))?;
    let sql = "UPDATE sys_dict_data SET dict_sort = $1, dict_label = $2, dict_value = $3, dict_type = $4, css_class = $5, list_class = $6, is_default = $7, remark = $8, update_time = NOW() WHERE dict_code = $9";
    let pv = crate::pv![
        jopt_i32(&body, "dictSort").unwrap_or(0),
        jstr(&body, "dictLabel"),
        jstr(&body, "dictValue"),
        jstr(&body, "dictType"),
        jstr(&body, "cssClass"),
        jstr(&body, "listClass"),
        jopt_str(&body, "isDefault").unwrap_or_else(|| "N".into()),
        jstr(&body, "remark"),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

pub async fn dict_data_delete(State(state): State<Arc<AppState>>, Path(ids): Path<String>) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        execute(&state.pool, "DELETE FROM sys_dict_data WHERE dict_code = $1", &[id]).await?;
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

// ------------------------------ 参数设置 ------------------------------

pub async fn config_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("config_name", qopt(&q, "configName").as_deref());
    qb.like("config_key", qopt(&q, "configKey").as_deref());
    if let Some(t) = qopt(&q, "configType") {
        qb.when("config_type = {}", Some(t));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_config{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_config{where_sql} ORDER BY config_id LIMIT {lph} OFFSET {oph}",
        crate::models::CONFIG_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::ConfigVO> = rows.iter().map(crate::models::config_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

/// GET /system/config/configKey/{key}
pub async fn config_by_key(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> AppResult<Json<Value>> {
    let rows = query(&state.pool, "SELECT config_value FROM sys_config WHERE config_key = $1", &[&key]).await?;
    let value: String = rows.first().and_then(|r| r.try_get("config_value").ok()).unwrap_or_default();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": value })))
}

pub async fn config_get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_config WHERE config_id = $1", crate::models::CONFIG_COLS);
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("参数不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::config_from_row(r) })))
}

pub async fn config_add(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let sql = "INSERT INTO sys_config (config_id, config_name, config_key, config_value, config_type, remark, create_time) \
               VALUES (COALESCE((SELECT max(config_id) FROM sys_config),0) + $1, $2, $3, $4, $5, $6, NOW()) RETURNING config_id";
    let pv = crate::pv![
        1i64,
        jstr(&body, "configName"),
        jstr(&body, "configKey"),
        jstr(&body, "configValue"),
        jopt_str(&body, "configType").unwrap_or_else(|| "N".into()),
        jstr(&body, "remark"),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("config_id").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

pub async fn config_update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "configId").ok_or(AppError::Param("configId 缺失".into()))?;
    let sql = "UPDATE sys_config SET config_name = $1, config_key = $2, config_value = $3, config_type = $4, remark = $5, update_time = NOW() WHERE config_id = $6";
    let pv = crate::pv![
        jstr(&body, "configName"),
        jstr(&body, "configKey"),
        jstr(&body, "configValue"),
        jopt_str(&body, "configType").unwrap_or_else(|| "N".into()),
        jstr(&body, "remark"),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// PUT /system/config/updateByKey
pub async fn config_update_by_key(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let key = jstr(&body, "configKey");
    let value = jstr(&body, "configValue");
    execute(&state.pool, "UPDATE sys_config SET config_value = $1, update_time = NOW() WHERE config_key = $2", &[&value, &key]).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

pub async fn config_delete(State(state): State<Arc<AppState>>, Path(ids): Path<String>) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        execute(&state.pool, "DELETE FROM sys_config WHERE config_id = $1", &[id]).await?;
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

pub async fn config_refresh(State(_state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    Ok(Json(json!({ "code": 200, "msg": "刷新成功" })))
}

// ------------------------------ 通知公告 ------------------------------

pub async fn notice_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("notice_title", qopt(&q, "noticeTitle").as_deref());
    if let Some(t) = qopt(&q, "noticeType") {
        qb.when("notice_type = {}", Some(t));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_notice{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_notice{where_sql} ORDER BY notice_id DESC LIMIT {lph} OFFSET {oph}",
        crate::models::NOTICE_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::NoticeVO> = rows.iter().map(crate::models::notice_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

pub async fn notice_get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {} FROM sys_notice WHERE notice_id = $1", crate::models::NOTICE_COLS);
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("公告不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::notice_from_row(r) })))
}

pub async fn notice_add(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let content = jstr(&body, "noticeContent");
    let sql = "INSERT INTO sys_notice (notice_id, notice_title, notice_type, notice_content, status, remark, create_time) \
               VALUES (COALESCE((SELECT max(notice_id) FROM sys_notice),0) + $1, $2, $3, $4, $5, $6, NOW()) RETURNING notice_id";
    let pv = crate::pv![
        1i64,
        jstr(&body, "noticeTitle"),
        jopt_str(&body, "noticeType").unwrap_or_else(|| "1".into()),
        content,
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "remark"),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let _ = user;
    let id: i64 = rows.first().and_then(|r| r.try_get("notice_id").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

pub async fn notice_update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "noticeId").ok_or(AppError::Param("noticeId 缺失".into()))?;
    let content = jstr(&body, "noticeContent");
    let sql = "UPDATE sys_notice SET notice_title = $1, notice_type = $2, notice_content = $3, status = $4, remark = $5, update_time = NOW() WHERE notice_id = $6";
    let pv = crate::pv![
        jstr(&body, "noticeTitle"),
        jopt_str(&body, "noticeType").unwrap_or_else(|| "1".into()),
        content,
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "remark"),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

pub async fn notice_delete(State(state): State<Arc<AppState>>, Path(ids): Path<String>) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        execute(&state.pool, "DELETE FROM sys_notice WHERE notice_id = $1", &[id]).await?;
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}
