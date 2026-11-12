//! 权限：从 `sys_role / sys_role_menu / sys_menu` 汇总用户的权限码，
//! 并按「路径 → 权限码」的规则做 URL 级与按钮级校验（替代 casbin）。
use std::collections::HashMap;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 取出某用户的 (角色码集合, 权限码集合)。
pub async fn load(state: &AppState, user_id: i64) -> AppResult<(Vec<String>, Vec<String>)> {
    if let Some(cached) = state.perms.get(user_id) {
        return Ok(cached);
    }
    let sql = r#"
SELECT DISTINCT r.role_key, COALESCE(m.perms, '') AS perms
FROM sys_user_role ur
  JOIN sys_role r ON r.role_id = ur.role_id AND r.del_flag = '0' AND r.status = '0'
  LEFT JOIN sys_role_menu rm ON rm.role_id = r.role_id
  LEFT JOIN sys_menu  m  ON m.menu_id = rm.menu_id AND m.status = '0'
WHERE ur.user_id = $1"#;
    let rows = crate::state::query(&state.pool, sql, &[&user_id]).await?;
    let mut roles: Vec<String> = Vec::new();
    let mut perms: Vec<String> = Vec::new();
    for row in rows {
        let role_key: String = row.try_get("role_key").unwrap_or_default();
        if !role_key.is_empty() && !roles.contains(&role_key) {
            roles.push(role_key);
        }
        let p: String = row.try_get("perms").unwrap_or_default();
        if !p.is_empty() && !perms.contains(&p) {
            perms.push(p);
        }
    }
    state.perms.put(user_id, roles.clone(), perms.clone());
    Ok((roles, perms))
}

/// 由「HTTP 方法 + 路径」查出所需的权限码；无需按钮级校验时返回空串。
///
/// **不做通用推导**：权限码由控制器注解显式声明，
/// 同一资源下不同端点的权限码可能不同（如 `GET /system/user/{id}` 是 `query` 而非 `list`），
/// 且带 id 的路径无法从路径本身反推动作名（`DELETE /system/post/7` 会误推成 `system:post:7`，
/// 导致非超管角色的删除操作全部被拒）。
/// 因此这里用显式模式表逐条匹配，`{x}` 段表示匹配任意单段。
///
/// 模式表按「静态段优先、参数段靠后」排序，保证 `/system/user/list` 不会被
/// `/system/user/{userId}` 抢先匹配。
pub fn required(method: &str, path: &str) -> String {
    for (m, pattern, perm) in PERM_RULES {
        if *m == method && match_pattern(pattern, path) {
            return perm.to_string();
        }
    }
    String::new()
}

/// 判断是否为参数段，并返回其类型。
///
/// * `Param`（`{x}` 或旧语法 `:x`）—— 匹配任意**单段**；
/// * `CatchAll`（`{*x}` 或旧语法 `*x`）—— 匹配**剩余所有段**（跨段通配）。
fn param_segment_kind(seg: &str) -> Option<ParamKind> {
    if seg.starts_with(':') {
        return Some(ParamKind::Param);
    }
    if seg.starts_with('{') && seg.ends_with('}') && seg.len() > 2 {
        let inner = &seg[1..seg.len() - 1];
        return if let Some(rest) = inner.strip_prefix('*') {
            if rest.is_empty() {
                None
            } else {
                Some(ParamKind::CatchAll)
            }
        } else if inner.is_empty() {
            None
        } else {
            Some(ParamKind::Param)
        };
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamKind {
    /// 单段通配
    Param,
    /// 跨段通配（贪婪匹配剩余全部段）
    CatchAll,
}

/// 模式匹配：参数段通配，其余段必须字面相等。
///
/// 单段参数写作 `{x}`（axum 0.8 新语法）或 `:x`（0.7 旧语法，保留兼容）；
/// 跨段通配写作 `{*x}`（0.8）或 `*x`（0.7），且只能出现在模式末尾。
fn match_pattern(pattern: &str, path: &str) -> bool {
    let ps: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let xs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    for (i, p) in ps.iter().enumerate() {
        match param_segment_kind(p) {
            Some(ParamKind::CatchAll) => {
                // 跨段通配必须位于末尾，且至少吃掉一段
                if i + 1 != ps.len() || i >= xs.len() {
                    return false;
                }
                return true;
            }
            Some(ParamKind::Param) => {
                if i >= xs.len() || xs[i].is_empty() {
                    return false;
                }
            }
            None => {
                if i >= xs.len() || *p != xs[i] {
                    return false;
                }
            }
        }
    }
    // 没有跨段通配时，段数必须完全一致
    ps.len() == xs.len()
}

/// 权限规则表：(HTTP 方法, 路径模式, 权限码)。
///
/// 逐条匹配权限注解模式。
/// 无权限注解的端点（个人资料、自身会话等）不在表内 —— 只需登录态。
const PERM_RULES: &[(&str, &str, &str)] = &[
    // ---- 用户 ----
    ("GET", "/system/user/list", "system:user:list"),
    ("GET", "/system/user/deptTree", "system:user:list"),
    ("GET", "/system/user/list/dept/{deptId}", "system:user:list"),
    ("GET", "/system/user/optionselect", "system:user:query"),
    ("GET", "/system/user/authRole/{userId}", "system:user:query"),
    ("PUT", "/system/user/changeStatus", "system:user:edit"),
    ("PUT", "/system/user/resetPwd", "system:user:resetPwd"),
    ("PUT", "/system/user/authRole", "system:user:edit"),
    ("POST", "/system/user", "system:user:add"),
    ("PUT", "/system/user", "system:user:edit"),
    ("GET", "/system/user/{userId}", "system:user:query"),
    ("DELETE", "/system/user/{userId}", "system:user:remove"),
    // ---- 角色 ----
    ("GET", "/system/role/list", "system:role:list"),
    ("GET", "/system/role/optionselect", "system:role:query"),
    ("PUT", "/system/role/dataScope", "system:role:edit"),
    ("PUT", "/system/role/changeStatus", "system:role:edit"),
    ("GET", "/system/role/authUser/allocatedList", "system:role:list"),
    ("GET", "/system/role/authUser/unallocatedList", "system:role:list"),
    ("PUT", "/system/role/authUser/cancel", "system:role:edit"),
    ("PUT", "/system/role/authUser/cancelAll", "system:role:edit"),
    ("PUT", "/system/role/authUser/selectAll", "system:role:edit"),
    ("GET", "/system/role/deptTree/{roleId}", "system:role:list"),
    ("POST", "/system/role", "system:role:add"),
    ("PUT", "/system/role", "system:role:edit"),
    ("GET", "/system/role/{roleId}", "system:role:query"),
    ("DELETE", "/system/role/{roleId}", "system:role:remove"),
    // ---- 菜单 ----
    ("GET", "/system/menu/list", "system:menu:list"),
    ("GET", "/system/menu/treeselect", "system:menu:query"),
    ("GET", "/system/menu/roleMenuTreeselect/{roleId}", "system:menu:query"),
    ("DELETE", "/system/menu/cascade/{menuId}", "system:menu:remove"),
    ("POST", "/system/menu", "system:menu:add"),
    ("PUT", "/system/menu", "system:menu:edit"),
    ("GET", "/system/menu/{menuId}", "system:menu:query"),
    ("DELETE", "/system/menu/{menuId}", "system:menu:remove"),
    // ---- 部门 ----
    ("GET", "/system/dept/list", "system:dept:list"),
    ("GET", "/system/dept/list/exclude/{deptId}", "system:dept:list"),
    ("GET", "/system/dept/optionselect", "system:dept:query"),
    ("POST", "/system/dept", "system:dept:add"),
    ("PUT", "/system/dept", "system:dept:edit"),
    ("GET", "/system/dept/{deptId}", "system:dept:query"),
    ("DELETE", "/system/dept/{deptId}", "system:dept:remove"),
    // ---- 岗位 ----
    ("GET", "/system/post/list", "system:post:list"),
    ("GET", "/system/post/optionselect", "system:post:query"),
    ("GET", "/system/post/deptTree", "system:post:list"),
    ("POST", "/system/post", "system:post:add"),
    ("PUT", "/system/post", "system:post:edit"),
    ("GET", "/system/post/{postId}", "system:post:query"),
    ("DELETE", "/system/post/{postId}", "system:post:remove"),
    // ---- 字典（type 与 data 共用 system:dict:* 权限码）----
    ("GET", "/system/dict/type/list", "system:dict:list"),
    ("DELETE", "/system/dict/type/refreshCache", "system:dict:remove"),
    ("POST", "/system/dict/type", "system:dict:add"),
    ("PUT", "/system/dict/type", "system:dict:edit"),
    ("GET", "/system/dict/type/{dictId}", "system:dict:query"),
    ("DELETE", "/system/dict/type/{dictId}", "system:dict:remove"),
    ("GET", "/system/dict/data/list", "system:dict:list"),
    ("POST", "/system/dict/data", "system:dict:add"),
    ("PUT", "/system/dict/data", "system:dict:edit"),
    ("GET", "/system/dict/data/{dictCode}", "system:dict:query"),
    ("DELETE", "/system/dict/data/{dictCode}", "system:dict:remove"),
    // ---- 参数 ----
    ("GET", "/system/config/list", "system:config:list"),
    ("DELETE", "/system/config/refreshCache", "system:config:remove"),
    ("PUT", "/system/config/updateByKey", "system:config:edit"),
    ("POST", "/system/config", "system:config:add"),
    ("PUT", "/system/config", "system:config:edit"),
    ("GET", "/system/config/{configId}", "system:config:query"),
    ("DELETE", "/system/config/{configId}", "system:config:remove"),
    // ---- 通知 ----
    ("GET", "/system/notice/list", "system:notice:list"),
    ("POST", "/system/notice", "system:notice:add"),
    ("PUT", "/system/notice", "system:notice:edit"),
    ("GET", "/system/notice/{noticeId}", "system:notice:query"),
    ("DELETE", "/system/notice/{noticeId}", "system:notice:remove"),
    // ---- 监控 ----
    ("GET", "/monitor/online/list", "monitor:online:list"),
    ("DELETE", "/monitor/online/{tokenId}", "monitor:online:forceLogout"),
    ("GET", "/monitor/logininfor/list", "monitor:logininfor:list"),
    ("GET", "/monitor/logininfor/unlock/{userName}", "monitor:logininfor:unlock"),
    ("DELETE", "/monitor/logininfor/clean", "monitor:logininfor:remove"),
    ("DELETE", "/monitor/logininfor/{infoId}", "monitor:logininfor:remove"),
    ("GET", "/monitor/operlog/list", "monitor:operlog:list"),
    ("DELETE", "/monitor/operlog/clean", "monitor:operlog:remove"),
    ("DELETE", "/monitor/operlog/{operId}", "monitor:operlog:remove"),
    // ---- 地图服务（map-api 管理面代理）----
    ("GET", "/map/services", "map:service:list"),
    ("PUT", "/map/services/{kind}/{name}/state", "map:service:state"),
    // 服务归属引擎：读复用 list；调整/清除归属按服务编辑权限（5 段字面量末段 /engine）
    ("GET", "/map/services/{kind}/{name}/engine", "map:service:list"),
    ("PUT", "/map/services/{kind}/{name}/engine", "map:service:edit"),
    ("DELETE", "/map/services/{kind}/{name}/engine", "map:service:edit"),
    // 服务分组：查复用 list；调整用 edit（5 段字面量末段 /group）
    ("GET", "/map/services/{kind}/{name}/group", "map:service:list"),
    ("PUT", "/map/services/{kind}/{name}/group", "map:service:edit"),
    ("GET", "/map/service-groups", "map:service:list"),
    ("POST", "/map/reload", "map:service:reload"),
    ("GET", "/map/usage", "map:usage:query"),
    ("POST", "/map/services/{kind}/{name}/check", "map:service:check"),
    // 服务配置编辑（读原文 / 回写 XML）：读复用 list，写用独立的 edit 权限码。
    // 两条规则均为 4 段路径，与 5 段的 /state、/check 不冲突（段数不同）。
    ("GET", "/map/services/{kind}/{name}", "map:service:list"),
    ("PUT", "/map/services/{kind}/{name}", "map:service:edit"),
    // 新增服务向导 / 运营仪表盘（复用既有权限，无需改库）
    ("GET", "/map/browse", "map:service:list"),
    ("POST", "/map/services", "map:service:reload"),
    ("POST", "/map/pg-test", "map:service:check"),
    ("GET", "/map/components", "map:service:list"),
    // 缓存管理（查看与清理复用 map:service:list，无需改库）
    ("GET", "/map/cache", "map:service:list"),
    ("POST", "/map/cache/purge", "map:service:list"),
    // 配置/设置页（只读展示 manager 运行配置，复用 map:service:list）
    ("GET", "/map/runtime", "map:service:list"),
    // ---- API Key 管理（菜单 128 / 按钮 1067）----
    // 读用 list；生成 / 重置 / 编辑 / 删除 / 应用到引擎用 edit。
    // 4 段的 rotate/apply 先于 3 段通配登记（段数不同，实际不冲突）。
    ("GET", "/map/apikeys", "map:apikey:list"),
    ("POST", "/map/apikeys", "map:apikey:edit"),
    ("POST", "/map/apikeys/{keyId}/rotate", "map:apikey:edit"),
    ("POST", "/map/apikeys/{keyId}/apply", "map:apikey:edit"),
    ("PUT", "/map/apikeys/{keyId}", "map:apikey:edit"),
    ("DELETE", "/map/apikeys/{keyId}", "map:apikey:edit"),
    // ---- 服务引擎管理（多引擎注册表）----
    // 查看与健康探测用 list；增删改/设默认用 edit。
    // /map/engines/:engineId/health 与 /default 均为 5 段，先于 4 段通配注册。
    ("GET", "/map/engines", "map:engine:list"),
    // 事件日志（静态路径，优先于 :engineId 通配）
    ("GET", "/map/engines/events", "map:engine:list"),
    ("GET", "/map/engines/{engineId}/health", "map:engine:list"),
    ("GET", "/map/engines/{engineId}/detail", "map:engine:list"),
    ("GET", "/map/engines/{engineId}", "map:engine:list"),
    ("POST", "/map/engines", "map:engine:edit"),
    ("PUT", "/map/engines/{engineId}", "map:engine:edit"),
    ("PUT", "/map/engines/{engineId}/default", "map:engine:edit"),
    ("DELETE", "/map/engines/{engineId}", "map:engine:edit"),
    // 访问日志与审计
    ("GET", "/map/logs", "map:log:list"),
    // 样式库（静态段在前，:name 通配在后）
    ("GET", "/map/styles", "map:style:list"),
    ("GET", "/map/styles/{name}/content", "map:style:list"),
    ("GET", "/map/styles/{name}/legend.png", "map:style:list"),
    ("POST", "/map/styles/{name}", "map:style:upload"),
    ("DELETE", "/map/styles/{name}", "map:style:remove"),
    // ---- 数据目录（Workspace / Store / Resource / 已发布服务）----
    // 权限码对齐 sql/08-catalog.sql 菜单：
    //   122 map:catalog:list（目录查看）、1060 map:catalog:edit（目录维护：
    //   工作区与 Store 增删改、连接测试）、1061 map:catalog:publish（数据发布：
    //   发现、发布、改发布信息、资源软删除与垃圾箱恢复）。
    ("GET", "/map/catalog/capabilities", "map:catalog:list"),
    ("GET", "/map/catalog/workspaces", "map:catalog:list"),
    ("GET", "/map/catalog/dirs", "map:catalog:list"),
    ("GET", "/map/catalog/stores", "map:catalog:list"),
    ("GET", "/map/catalog/resources", "map:catalog:list"),
    ("GET", "/map/catalog/published", "map:catalog:list"),
    ("GET", "/map/catalog/published/{id}/endpoints", "map:catalog:list"),
    ("POST", "/map/catalog/stores/{id}/discover", "map:catalog:publish"),
    ("POST", "/map/catalog/stores/{id}/import-shp", "map:catalog:edit"),
    ("POST", "/map/catalog/stores/{id}/connect", "map:catalog:edit"),
    ("POST", "/map/catalog/stores/connect", "map:catalog:edit"),
    ("POST", "/map/catalog/stores/{id}/restore", "map:catalog:edit"),
    ("POST", "/map/catalog/resources/{id}/restore", "map:catalog:publish"),
    ("POST", "/map/catalog/resources/{id}/refresh", "map:catalog:publish"),
    ("GET", "/map/catalog/resources/{id}/query", "map:catalog:list"),
    ("PUT", "/map/catalog/published/{id}/state", "map:catalog:publish"),
    ("POST", "/map/catalog/workspaces", "map:catalog:edit"),
    ("PUT", "/map/catalog/workspaces/{id}", "map:catalog:edit"),
    ("DELETE", "/map/catalog/workspaces/{id}", "map:catalog:edit"),
    ("POST", "/map/catalog/stores", "map:catalog:edit"),
    ("PUT", "/map/catalog/stores/{id}", "map:catalog:edit"),
    ("DELETE", "/map/catalog/stores/{id}", "map:catalog:edit"),
    ("POST", "/map/catalog/publish", "map:catalog:publish"),
    ("PUT", "/map/catalog/published/{id}", "map:catalog:publish"),
    ("DELETE", "/map/catalog/resources/{id}", "map:catalog:publish"),
];

/// 不需要按钮级校验、只需登录态即可访问的读端点（前端强依赖，否则页面打不开）。
fn is_open_read(path: &str) -> bool {
    [
        "/system/user/getInfo",
        "/system/menu/getRouters",
        "/system/user/profile",
        "/system/user/deptTree",
        "/system/post/deptTree",
        "/system/menu/treeselect",
        "/system/dept/optionselect",
        "/system/post/optionselect",
        "/system/role/optionselect",
        "/system/dict/type/optionselect",
        "/system/dict/data/type/",
        "/system/config/configKey/",
    ]
    .iter()
    .any(|p| path.starts_with(p))
        || path.contains("/treeselect")
        || path.contains("/optionselect")
}

/// **没有**权限注解的端点：只需登录态，不做按钮级校验。
///
/// 无需按钮级校验的端点：`SysProfileController`（个人资料/改密/头像）、`SysUserController.getInfo`、
/// `SysUserOnlineController`（自身会话）、`SysConfigController.getConfigKey`、
/// `SysDictTypeController.optionselect`、`SysDictDataController.dictType`。
const NO_PERM_PATTERNS: &[(&str, &str)] = &[
    ("GET", "/system/user/profile"),
    ("PUT", "/system/user/profile"),
    ("PUT", "/system/user/profile/updatePwd"),
    ("POST", "/system/user/profile/avatar"),
    ("GET", "/monitor/online"),
    ("DELETE", "/monitor/online/myself"),
    ("DELETE", "/monitor/online/myself/{tokenId}"),
    ("GET", "/system/config/configKey/{configKey}"),
    ("GET", "/system/dict/data/type/{dictType}"),
];

/// 鉴权核心：超级管理员放行；其余按 `PERM_RULES` 查出的权限码匹配。
pub fn allows(method: &str, path: &str, admin: bool, perms: &[String]) -> Result<(), AppError> {
    if admin {
        return Ok(());
    }
    if method == "GET" && is_open_read(path) {
        return Ok(());
    }
    // 无注解端点：仅需登录态
    if NO_PERM_PATTERNS
        .iter()
        .any(|(m, p)| *m == method && match_pattern(p, path))
    {
        return Ok(());
    }
    let need = required(method, path);
    if need.is_empty() {
        // 规则表未覆盖的路径：读操作放行（依赖菜单可见性约束），写操作一律拒绝。
        if method == "GET" {
            return Ok(());
        }
        return Err(AppError::Forbidden(path.to_string()));
    }
    if perms.iter().any(|p| p == &need) {
        return Ok(());
    }
    Err(AppError::Forbidden(need))
}

/// 把扁平菜单列表转成 `getRouters` 需要的树。
pub fn build_tree(items: Vec<crate::models::MenuVO>) -> Vec<crate::models::MenuVO> {
    let mut by_parent: HashMap<i64, Vec<crate::models::MenuVO>> = HashMap::new();
    for it in items {
        by_parent.entry(it.parent_id).or_default().push(it);
    }
    fn walk(id: i64, map: &HashMap<i64, Vec<crate::models::MenuVO>>) -> Vec<crate::models::MenuVO> {
        let mut out = Vec::new();
        if let Some(children) = map.get(&id) {
            for c in children {
                let mut node = c.clone();
                let sub = walk(node.menu_id, map);
                node.children = sub;
                out.push(node);
            }
        }
        out.sort_by_key(|n| n.order_num);
        out
    }
    walk(0, &by_parent)
}

/// 菜单 → 前端动态路由节点。
///
/// 路由构建规则（`buildMenus` / `getRouterPath` / `getComponentInfo`）：
///
/// * `is_frame`：`"0"`=是外链（YES_FRAME，新窗口打开），`"1"`=非外链（NO_FRAME，站内嵌）
/// * 一级目录（parent_id=0 且 type=M 且 is_frame="1"）→ path 补前导 `/`，component=`Layout`
/// * 一级菜单（parent_id=0 且 type=C 且 is_frame="1"）→ path=`/`，component=`Layout`，并派生出唯一子路由
/// * 真外链（is_frame="0" 且 path 是 http 开头）→ path 保持原始 http 地址，前端按外链渲染不注册路由
/// * 内链（is_frame="1" 且 path 是 http 开头）→ path 经 `inner_link_replace_each` 转义，component=`InnerLink`
/// * 目录节点带子菜单时 `redirect = noRedirect` 且 `alwaysShow = true`
/// * `meta.noCache` 取 `is_cache == "1"`（`MetaVo` 构造器语义）
pub fn to_routers(items: Vec<crate::models::MenuVO>) -> Vec<crate::models::RouterVO> {
    items
        .into_iter()
        .filter(|m| m.menu_type == "M" || m.menu_type == "C")
        .map(|m| {
            let is_top = m.parent_id == 0;
            let raw_component = m.component.as_deref().unwrap_or("").trim();
            let has_component = !raw_component.is_empty();
            // isMenuFrame：顶级 + 菜单类型 + 非外链
            let is_menu_frame = is_top && m.menu_type == "C" && m.is_frame == "1";
            // isInnerLink：非外链标记 + http(s) 地址
            let is_inner_link = m.is_frame == "1" && is_http(&m.path);

            // name（getRouteName + menuId）：isMenuFrame 时 routeName 为空串
            let name = if is_menu_frame {
                m.menu_id.to_string()
            } else {
                format!("{}{}", capitalize(&m.path), m.menu_id)
            };

            // path（getRouterPath）
            let path = if !is_top && is_inner_link {
                inner_link_replace_each(&m.path)
            } else if is_top && m.menu_type == "M" && m.is_frame == "1" {
                format!("/{}", m.path.trim_start_matches('/'))
            } else if is_menu_frame {
                "/".to_string()
            } else {
                m.path.clone()
            };

            // component（getComponentInfo）
            let component = if has_component && !is_menu_frame {
                raw_component.to_string()
            } else if !has_component && !is_top && is_inner_link {
                "InnerLink".to_string()
            } else if !has_component && is_parent_view(&m) {
                "ParentView".to_string()
            } else {
                "Layout".to_string()
            };

            let children = m.children.clone();
            let has_child = !children.is_empty();

            let query = if m.query_param.is_empty() {
                None
            } else {
                Some(m.query_param.clone())
            };

            // meta（MetaVo 5 参构造器：link 仅当 http 时赋值，activeMenu 仅当以 / 开头时赋值）
            let meta = crate::models::MetaVO {
                title: m.menu_name.clone(),
                icon: m.icon.clone(),
                link: if is_http(&m.path) {
                    Some(m.path.clone())
                } else {
                    None
                },
                no_cache: m.is_cache == "1",
                active_menu: if m.remark.starts_with('/') {
                    Some(m.remark.clone())
                } else {
                    None
                },
            };

            // 一级菜单（isMenuFrame）：父级 meta 置空，派生出唯一子路由
            if is_menu_frame {
                let child = crate::models::RouterVO {
                    name: format!("{}{}", capitalize(&m.path), m.menu_id),
                    path: m.path.clone(),
                    redirect: None,
                    component: if has_component {
                        Some(raw_component.to_string())
                    } else {
                        None
                    },
                    query: query.clone(),
                    hidden: m.visible == "1",
                    always_show: None,
                    children: None,
                    meta: Some(meta),
                };
                return crate::models::RouterVO {
                    name,
                    path,
                    redirect: None,
                    component: Some(component),
                    query,
                    hidden: m.visible == "1",
                    always_show: None,
                    children: Some(vec![child]),
                    meta: None,
                };
            }

            // 顶级内链：外层 path="/" 且 meta 只留标题图标，内层为 InnerLink 子路由
            if is_top && is_inner_link {
                let inner_path = inner_link_replace_each(&m.path);
                let child = crate::models::RouterVO {
                    name: format!("{}{}", capitalize(&inner_path), m.menu_id),
                    path: inner_path,
                    redirect: None,
                    component: Some("InnerLink".to_string()),
                    query: None,
                    hidden: m.visible == "1",
                    always_show: None,
                    children: None,
                    meta: Some(crate::models::MetaVO {
                        title: m.menu_name.clone(),
                        icon: m.icon.clone(),
                        link: Some(m.path.clone()),
                        no_cache: m.is_cache == "1",
                        active_menu: None,
                    }),
                };
                return crate::models::RouterVO {
                    name,
                    path: "/".to_string(),
                    redirect: None,
                    component: Some(component),
                    query: None,
                    hidden: m.visible == "1",
                    always_show: None,
                    children: Some(vec![child]),
                    meta: Some(crate::models::MetaVO {
                        title: m.menu_name.clone(),
                        icon: m.icon.clone(),
                        link: None,
                        no_cache: m.is_cache == "1",
                        active_menu: None,
                    }),
                };
            }

            // 普通节点：目录带子菜单时 redirect=noRedirect 且 alwaysShow=true
            let (children_vo, redirect, always_show) = if has_child && m.menu_type == "M" {
                (
                    Some(to_routers(children)),
                    Some("noRedirect".to_string()),
                    Some(true),
                )
            } else if has_child {
                (Some(to_routers(children)), None, None)
            } else {
                (None, None, None)
            };

            crate::models::RouterVO {
                name,
                path,
                redirect,
                component: Some(component),
                query,
                hidden: m.visible == "1",
                always_show,
                children: children_vo,
                meta: Some(meta),
            }
        })
        .collect()
}

/// `SysMenu.isParentView`：非顶级 + 目录类型。
fn is_parent_view(m: &crate::models::MenuVO) -> bool {
    m.parent_id != 0 && m.menu_type == "M"
}

/// 是否是 http(s) 地址（`StringUtils.ishttp` 的实用近似）。
fn is_http(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

/// `SysMenu.innerLinkReplaceEach`：把协议与域名分隔符换成 `/`。
fn inner_link_replace_each(path: &str) -> String {
    path.replace("http://", "")
        .replace("https://", "")
        .replace("www.", "")
        .replace('.', "/")
        .replace(':', "/")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_segment_matching_supports_both_syntaxes() {
        // axum 0.8 新语法 {x}：当前 PERM_RULES 使用的形式
        assert!(match_pattern("/system/user/{userId}", "/system/user/7"));
        assert!(match_pattern("/system/user/{userId}", "/system/user/abc"));
        // axum 0.7 旧语法 :x：保留兼容，防止遗漏的历史条目静默失效
        assert!(match_pattern("/system/user/:userId", "/system/user/7"));
        // 通配捕获 {*rest}
        assert!(match_pattern("/profile/avatar/{*path}", "/profile/avatar/a/b/c.png"));
        // 普通段不得被当作参数
        assert!(!match_pattern("/system/user/list", "/system/user/7"));
        // 段数不同不匹配
        assert!(!match_pattern("/system/user/{userId}", "/system/user/7/extra"));
        assert!(!match_pattern("/system/user/{userId}", "/system/user"));
    }

    /// 路由语法迁移的守卫：PERM_RULES 里不得残留 axum 0.7 的 `:param` 旧写法。
    ///
    /// 0.8 的 matchit 在注册路由时会对 `:` 段直接 panic（见 matchit 0.8 changelog），
    /// 而权限表是纯数据、不会 panic，只会让鉴权静默失配。
    /// 这里锁死规则表统一使用 `{param}` 语法。
    #[test]
    fn perm_rules_use_axum08_brace_syntax() {
        for (method, pattern, perm) in PERM_RULES {
            assert!(
                !pattern.contains("/:"),
                "权限规则仍用 0.7 旧语法 :param —— {method} {pattern} ({perm})"
            );
            assert!(
                !pattern.contains("/*"),
                "权限规则仍用 0.7 旧通配 *rest —— {method} {pattern} ({perm})"
            );
        }
    }

    #[test]
    fn map_routes_resolve_expected_perms() {
        // 每个 /map/* 路由都应查到正确的权限码（与 PERM_RULES 一致）
        assert_eq!(required("GET", "/map/services"), "map:service:list");
        assert_eq!(required("PUT", "/map/services/cog/world/state"), "map:service:state");
        assert_eq!(required("POST", "/map/reload"), "map:service:reload");
        assert_eq!(required("GET", "/map/usage"), "map:usage:query");
        assert_eq!(
            required("POST", "/map/services/postgis/wuhua_buildings/check"),
            "map:service:check"
        );
        assert_eq!(required("GET", "/map/browse"), "map:service:list");
        assert_eq!(required("POST", "/map/services"), "map:service:reload");
        assert_eq!(required("POST", "/map/pg-test"), "map:service:check");
        assert_eq!(required("GET", "/map/components"), "map:service:list");
        assert_eq!(required("GET", "/map/cache"), "map:service:list");
        assert_eq!(required("POST", "/map/cache/purge"), "map:service:list");
        assert_eq!(required("GET", "/map/logs"), "map:log:list");
        assert_eq!(required("GET", "/map/styles"), "map:style:list");
        // 静态段优先于 :name 通配：legend.png 必须命中 /map/styles/:name/legend.png
        assert_eq!(required("GET", "/map/styles/my/legend.png"), "map:style:list");
        assert_eq!(required("POST", "/map/styles/my"), "map:style:upload");
        assert_eq!(required("DELETE", "/map/styles/my"), "map:style:remove");
        assert_eq!(required("GET", "/map/runtime"), "map:service:list");
        // 配置编辑：读原文用 list，回写用独立的 edit 权限码
        assert_eq!(required("GET", "/map/services/cog/world"), "map:service:list");
        assert_eq!(required("PUT", "/map/services/cog/world"), "map:service:edit");
        // 4 段规则不得遮蔽 5 段的 /state、/check
        assert_eq!(
            required("PUT", "/map/services/cog/world/state"),
            "map:service:state"
        );
        assert_eq!(
            required("POST", "/map/services/cog/world/check"),
            "map:service:check"
        );
        // 服务归属引擎：读 = list；调整/清除 = edit（5 段字面量末段 /engine）
        assert_eq!(
            required("GET", "/map/services/cog/world/engine"),
            "map:service:list"
        );
        assert_eq!(
            required("PUT", "/map/services/cog/world/engine"),
            "map:service:edit"
        );
        assert_eq!(
            required("DELETE", "/map/services/cog/world/engine"),
            "map:service:edit"
        );
        // 引擎监控与详情
        assert_eq!(required("GET", "/map/engines/events"), "map:engine:list");
        assert_eq!(
            required("GET", "/map/engines/map-api/detail"),
            "map:engine:list"
        );
        // 服务分组
        assert_eq!(required("GET", "/map/service-groups"), "map:service:list");
        assert_eq!(
            required("GET", "/map/services/cog/world/group"),
            "map:service:list"
        );
        assert_eq!(
            required("PUT", "/map/services/cog/world/group"),
            "map:service:edit"
        );
    }

    #[test]
    fn apikey_routes_resolve_expected_perms() {
        assert_eq!(required("GET", "/map/apikeys"), "map:apikey:list");
        assert_eq!(required("POST", "/map/apikeys"), "map:apikey:edit");
        assert_eq!(required("PUT", "/map/apikeys/abc"), "map:apikey:edit");
        assert_eq!(required("DELETE", "/map/apikeys/abc"), "map:apikey:edit");
        assert_eq!(required("POST", "/map/apikeys/abc/rotate"), "map:apikey:edit");
        assert_eq!(required("POST", "/map/apikeys/abc/apply"), "map:apikey:edit");
        // 读操作不能因为 4 段规则被误判成参数段
        assert_eq!(required("GET", "/map/apikeys/abc/rotate"), "");
    }

    #[test]
    fn apikey_write_routes_are_all_covered() {
        // 规则表漏登记的写操作会被 allows() 直接拒绝（非超管角色）
        let writes = [
            ("POST", "/map/apikeys"),
            ("PUT", "/map/apikeys/7"),
            ("DELETE", "/map/apikeys/7"),
            ("POST", "/map/apikeys/7/rotate"),
            ("POST", "/map/apikeys/7/apply"),
        ];
        for (m, p) in writes {
            assert_eq!(required(m, p), "map:apikey:edit", "{m} {p} 未登记权限码");
        }
        let read_only = vec!["map:apikey:list".to_string()];
        assert!(allows("GET", "/map/apikeys", false, &read_only).is_ok());
        assert!(allows("POST", "/map/apikeys", false, &read_only).is_err());
        let with_edit = vec!["map:apikey:list".to_string(), "map:apikey:edit".to_string()];
        for (m, p) in writes {
            assert!(allows(m, p, false, &with_edit).is_ok(), "{m} {p} 应放行");
        }
    }

    #[test]
    fn engine_routes_resolve_expected_perms() {
        // 引擎：查看/探测 = list；增删改/设默认 = edit
        assert_eq!(required("GET", "/map/engines"), "map:engine:list");
        assert_eq!(required("POST", "/map/engines"), "map:engine:edit");
        assert_eq!(required("GET", "/map/engines/map-api"), "map:engine:list");
        assert_eq!(
            required("GET", "/map/engines/map-api/health"),
            "map:engine:list"
        );
        assert_eq!(
            required("PUT", "/map/engines/map-api/default"),
            "map:engine:edit"
        );
        assert_eq!(
            required("PUT", "/map/engines/map-api"),
            "map:engine:edit"
        );
        assert_eq!(
            required("DELETE", "/map/engines/map-api"),
            "map:engine:edit"
        );
    }

    #[test]
    fn catalog_routes_resolve_expected_perms() {
        // 目录读：全部落 list
        assert_eq!(required("GET", "/map/catalog/capabilities"), "map:catalog:list");
        assert_eq!(required("GET", "/map/catalog/workspaces"), "map:catalog:list");
        assert_eq!(required("GET", "/map/catalog/stores"), "map:catalog:list");
        assert_eq!(required("GET", "/map/catalog/resources"), "map:catalog:list");
        assert_eq!(required("GET", "/map/catalog/published"), "map:catalog:list");
        assert_eq!(
            required("GET", "/map/catalog/published/7/endpoints"),
            "map:catalog:list"
        );
        // 目录维护：工作区 / Store 增删改、连接测试
        assert_eq!(required("POST", "/map/catalog/workspaces"), "map:catalog:edit");
        assert_eq!(
            required("PUT", "/map/catalog/workspaces/7"),
            "map:catalog:edit"
        );
        assert_eq!(
            required("DELETE", "/map/catalog/workspaces/7"),
            "map:catalog:edit"
        );
        assert_eq!(required("POST", "/map/catalog/stores"), "map:catalog:edit");
        assert_eq!(
            required("PUT", "/map/catalog/stores/7"),
            "map:catalog:edit"
        );
        assert_eq!(
            required("DELETE", "/map/catalog/stores/7"),
            "map:catalog:edit"
        );
        assert_eq!(
            required("POST", "/map/catalog/stores/7/connect"),
            "map:catalog:edit"
        );
        assert_eq!(
            required("POST", "/map/catalog/stores/7/restore"),
            "map:catalog:edit"
        );
        // 数据发布：发现、发布、改发布信息、软删除与垃圾箱恢复
        assert_eq!(
            required("POST", "/map/catalog/stores/7/discover"),
            "map:catalog:publish"
        );
        assert_eq!(required("POST", "/map/catalog/publish"), "map:catalog:publish");
        assert_eq!(
            required("PUT", "/map/catalog/published/7"),
            "map:catalog:publish"
        );
        assert_eq!(
            required("PUT", "/map/catalog/published/7/state"),
            "map:catalog:publish"
        );
        assert_eq!(
            required("DELETE", "/map/catalog/resources/7"),
            "map:catalog:publish"
        );
        assert_eq!(
            required("POST", "/map/catalog/resources/7/restore"),
            "map:catalog:publish"
        );
        assert_eq!(
            required("POST", "/map/catalog/resources/7/refresh"),
            "map:catalog:publish"
        );
    }

    #[test]
    fn catalog_write_routes_are_not_uncovered() {
        // 规则表若漏登记，POST/PUT/DELETE 会被当成「未覆盖的写」直接拒绝；
        // 这里用与 catalog::routes() 一致的清单兜住，防止新增端点忘记补权限码。
        let writes = [
            ("POST", "/map/catalog/workspaces"),
            ("PUT", "/map/catalog/workspaces/7"),
            ("DELETE", "/map/catalog/workspaces/7"),
            ("POST", "/map/catalog/stores"),
            ("PUT", "/map/catalog/stores/7"),
            ("DELETE", "/map/catalog/stores/7"),
            ("POST", "/map/catalog/stores/7/connect"),
            ("POST", "/map/catalog/stores/7/discover"),
            ("POST", "/map/catalog/stores/7/restore"),
            ("GET", "/map/catalog/capabilities"),
            ("GET", "/map/catalog/workspaces"),
            ("GET", "/map/catalog/stores"),
            ("GET", "/map/catalog/resources"),
            ("GET", "/map/catalog/published"),
            ("GET", "/map/catalog/published/7/endpoints"),
            ("POST", "/map/catalog/publish"),
            ("PUT", "/map/catalog/published/7"),
            ("PUT", "/map/catalog/published/7/state"),
            ("DELETE", "/map/catalog/resources/7"),
            ("POST", "/map/catalog/resources/7/restore"),
            ("POST", "/map/catalog/resources/7/refresh"),
        ];
        for (m, p) in writes {
            assert!(
                !required(m, p).is_empty(),
                "{m} {p} 未在 PERM_RULES 登记目录权限码"
            );
        }
    }

    #[test]
    fn role3_perms_cover_all_map_routes() {
        // 模拟 role 3 的权限集合（与 DB sys_role_menu 实查一致）
        let role3: Vec<String> = [
            "map:apikey:edit",
            "map:apikey:list",
            "map:catalog:edit",
            "map:catalog:list",
            "map:catalog:publish",
            "map:engine:edit",
            "map:engine:list",
            "map:log:list",
            "map:service:check",
            "map:service:edit",
            "map:service:list",
            "map:service:reload",
            "map:service:state",
            "map:style:list",
            "map:style:remove",
            "map:style:upload",
            "map:usage:query",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let routes = [
            ("GET", "/map/services"),
            ("PUT", "/map/services/cog/world/state"),
            ("POST", "/map/reload"),
            ("GET", "/map/usage"),
            ("POST", "/map/services/postgis/wuhua_buildings/check"),
            ("GET", "/map/browse"),
            ("POST", "/map/services"),
            ("POST", "/map/pg-test"),
            ("GET", "/map/components"),
            ("GET", "/map/cache"),
            ("POST", "/map/cache/purge"),
            ("GET", "/map/logs"),
            ("GET", "/map/styles"),
            ("POST", "/map/styles/my"),
            ("DELETE", "/map/styles/my"),
            ("GET", "/map/runtime"),
            // API Key 管理（菜单 128 / 按钮 1067 已授角色 3）
            ("GET", "/map/apikeys"),
            ("POST", "/map/apikeys"),
            ("PUT", "/map/apikeys/k1"),
            ("POST", "/map/apikeys/k1/rotate"),
            ("POST", "/map/apikeys/k1/apply"),
            ("DELETE", "/map/apikeys/k1"),
            ("GET", "/map/services/cog/world"),
            ("PUT", "/map/services/cog/world"),
            // 服务归属引擎
            ("GET", "/map/services/cog/world/engine"),
            ("PUT", "/map/services/cog/world/engine"),
            ("DELETE", "/map/services/cog/world/engine"),
            // 引擎管理
            ("GET", "/map/engines"),
            ("GET", "/map/engines/map-api/health"),
            ("POST", "/map/engines"),
            ("PUT", "/map/engines/map-api"),
            ("PUT", "/map/engines/map-api/default"),
            ("DELETE", "/map/engines/map-api"),
            // 数据目录（菜单 122/1060/1061 已授角色 3）
            ("GET", "/map/catalog/capabilities"),
            ("POST", "/map/catalog/workspaces"),
            ("PUT", "/map/catalog/workspaces/7"),
            ("POST", "/map/catalog/stores"),
            ("POST", "/map/catalog/stores/7/connect"),
            ("POST", "/map/catalog/stores/7/discover"),
            ("POST", "/map/catalog/stores/7/restore"),
            ("GET", "/map/catalog/resources"),
            ("POST", "/map/catalog/publish"),
            ("GET", "/map/catalog/published"),
            ("GET", "/map/catalog/published/7/endpoints"),
            ("PUT", "/map/catalog/published/7/state"),
            ("DELETE", "/map/catalog/resources/7"),
            ("POST", "/map/catalog/resources/7/restore"),
            ("POST", "/map/catalog/resources/7/refresh"),
        ];
        for (m, p) in routes {
            assert!(
                allows(m, p, false, &role3).is_ok(),
                "role3 应放行 {m} {p}，但实际被拒"
            );
        }
    }

    #[test]
    fn admin_always_allowed() {
        let empty: Vec<String> = vec![];
        assert!(allows("POST", "/map/reload", true, &empty).is_ok());
        assert!(allows("DELETE", "/system/user/1", true, &empty).is_ok());
    }

    #[test]
    fn uncovered_get_allowed_uncovered_post_denied() {
        let perms: Vec<String> = vec![];
        // 规则表未覆盖的 GET 读操作放行（依赖菜单可见性约束）
        assert!(allows("GET", "/some/new/read", false, &perms).is_ok());
        // 规则表未覆盖的 POST 写操作一律拒绝
        assert!(allows("POST", "/some/new/write", false, &perms).is_err());
    }

    #[test]
    fn config_edit_requires_edit_perm() {
        // 仅有 list 权限（能读配置原文、能看服务列表）的角色
        let read_only: Vec<String> = vec!["map:service:list".to_string()];
        // 读配置原文：放行
        assert!(allows("GET", "/map/services/cog/world", false, &read_only).is_ok());
        // 回写配置：拒绝（缺 map:service:edit）
        assert!(allows("PUT", "/map/services/cog/world", false, &read_only).is_err());
        // 补上 edit 权限后放行
        let with_edit = vec![
            "map:service:list".to_string(),
            "map:service:edit".to_string(),
        ];
        assert!(allows("PUT", "/map/services/cog/world", false, &with_edit).is_ok());
    }
}
