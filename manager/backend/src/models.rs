//! 视图对象（VO）与 `Row` → Rust 的取值助手。
//!
//! 约定：PostgreSQL 的 `char` 列在 SQL 里统一 `::text` 取值，
//! 避免 rust-postgres 对 bpchar 的类型推断歧义。
use axum::Json;
use chrono::NaiveDateTime;
use serde::Serialize;
use tokio_postgres::Row;

// ---------- Row 取值助手 ----------

pub fn s(row: &Row, name: &str) -> String {
    row.try_get::<_, Option<String>>(name)
        .ok()
        .flatten()
        .unwrap_or_default()
}
pub fn os(row: &Row, name: &str) -> Option<String> {
    row.try_get::<_, Option<String>>(name).ok().flatten()
}
pub fn i64v(row: &Row, name: &str) -> i64 {
    row.try_get::<_, Option<i64>>(name)
        .ok()
        .flatten()
        .unwrap_or(0)
}
pub fn oi64(row: &Row, name: &str) -> Option<i64> {
    row.try_get::<_, Option<i64>>(name).ok().flatten()
}
pub fn i32v(row: &Row, name: &str) -> i32 {
    row.try_get::<_, Option<i32>>(name)
        .ok()
        .flatten()
        .unwrap_or(0)
}
pub fn boolv(row: &Row, name: &str) -> bool {
    row.try_get::<_, Option<bool>>(name)
        .ok()
        .flatten()
        .unwrap_or(false)
}
pub fn dt(row: &Row, name: &str) -> Option<NaiveDateTime> {
    row.try_get::<_, Option<NaiveDateTime>>(name).ok().flatten()
}
pub fn fmt_dt(v: Option<NaiveDateTime>) -> String {
    v.map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

// ---------- 用户 ----------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserVO {
    pub user_id: i64,
    pub dept_id: i64,
    pub user_name: String,
    pub nick_name: String,
    pub user_type: String,
    pub email: String,
    pub phonenumber: String,
    pub sex: String,
    pub avatar: String,
    pub password: Option<String>,
    pub status: String,
    pub del_flag: String,
    pub login_ip: String,
    pub login_date: String,
    pub create_by: i64,
    pub create_time: String,
    pub update_by: i64,
    pub update_time: String,
    pub remark: String,
    pub dept_name: String,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub role_ids: Vec<i64>,
    #[serde(default)]
    pub post_ids: Vec<i64>,
}

pub const USER_COLS: &str = "u.user_id, u.dept_id, u.user_name, u.nick_name, u.user_type, \
     u.email, u.phonenumber, u.sex::text AS sex, u.status::text AS status, \
     u.del_flag::text AS del_flag, u.login_ip, u.create_by, u.update_by, u.login_date, \
     u.create_time, u.update_time, u.remark, u.avatar";

pub fn user_from_row(row: &Row, with_password: bool) -> UserVO {
    UserVO {
        user_id: i64v(row, "user_id"),
        dept_id: i64v(row, "dept_id"),
        user_name: s(row, "user_name"),
        nick_name: s(row, "nick_name"),
        user_type: s(row, "user_type"),
        email: s(row, "email"),
        phonenumber: s(row, "phonenumber"),
        sex: s(row, "sex"),
        avatar: s(row, "avatar"),
        password: if with_password { os(row, "password") } else { None },
        status: s(row, "status"),
        del_flag: s(row, "del_flag"),
        login_ip: s(row, "login_ip"),
        login_date: fmt_dt(dt(row, "login_date")),
        create_by: i64v(row, "create_by"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_by: i64v(row, "update_by"),
        update_time: fmt_dt(dt(row, "update_time")),
        remark: s(row, "remark"),
        dept_name: s(row, "dept_name"),
        admin: i64v(row, "user_id") == 1,
        role_ids: Vec::new(),
        post_ids: Vec::new(),
    }
}

/// 登录成功返回的用户信息 + 角色 + 权限。
#[derive(Debug, Serialize)]
pub struct UserInfoVO {
    pub user: UserVO,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}

// ---------- 角色 ----------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleVO {
    pub role_id: i64,
    pub role_name: String,
    pub role_key: String,
    pub role_sort: i32,
    pub data_scope: String,
    pub menu_check_strictly: bool,
    pub dept_check_strictly: bool,
    pub status: String,
    pub del_flag: String,
    pub create_by: i64,
    pub create_time: String,
    pub update_by: i64,
    pub update_time: String,
    pub remark: String,
    #[serde(default)]
    pub menu_ids: Vec<i64>,
    #[serde(default)]
    pub dept_ids: Vec<i64>,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub flag: bool,
}

pub const ROLE_COLS: &str = "role_id, role_name, role_key, role_sort, data_scope::text AS data_scope, \
     menu_check_strictly, dept_check_strictly, status::text AS status, del_flag::text AS del_flag, \
     create_by, create_time, update_by, update_time, remark";

pub fn role_from_row(row: &Row) -> RoleVO {
    RoleVO {
        role_id: i64v(row, "role_id"),
        role_name: s(row, "role_name"),
        role_key: s(row, "role_key"),
        role_sort: i32v(row, "role_sort"),
        data_scope: s(row, "data_scope"),
        menu_check_strictly: boolv(row, "menu_check_strictly"),
        dept_check_strictly: boolv(row, "dept_check_strictly"),
        status: s(row, "status"),
        del_flag: s(row, "del_flag"),
        create_by: i64v(row, "create_by"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_by: i64v(row, "update_by"),
        update_time: fmt_dt(dt(row, "update_time")),
        remark: s(row, "remark"),
        menu_ids: Vec::new(),
        dept_ids: Vec::new(),
        admin: false,
        flag: false,
    }
}

// ---------- 菜单 ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuVO {
    pub menu_id: i64,
    pub menu_name: String,
    pub parent_id: i64,
    pub order_num: i32,
    pub path: String,
    pub component: Option<String>,
    pub query_param: String,
    pub is_frame: String,
    pub is_cache: String,
    pub menu_type: String,
    pub visible: String,
    pub status: String,
    pub perms: String,
    pub icon: String,
    pub remark: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MenuVO>,
}

pub const MENU_COLS: &str = "menu_id, menu_name, parent_id, order_num, path, component, \
     query_param, is_frame::text AS is_frame, is_cache::text AS is_cache, \
     menu_type::text AS menu_type, visible::text AS visible, status::text AS status, \
     perms, icon, create_time, remark";

pub fn menu_from_row(row: &Row) -> MenuVO {
    MenuVO {
        menu_id: i64v(row, "menu_id"),
        menu_name: s(row, "menu_name"),
        parent_id: i64v(row, "parent_id"),
        order_num: i32v(row, "order_num"),
        path: s(row, "path"),
        component: os(row, "component").filter(|v| !v.is_empty()),
        query_param: s(row, "query_param"),
        is_frame: s(row, "is_frame"),
        is_cache: s(row, "is_cache"),
        menu_type: s(row, "menu_type"),
        visible: s(row, "visible"),
        status: s(row, "status"),
        perms: s(row, "perms"),
        icon: s(row, "icon"),
        remark: s(row, "remark"),
        children: Vec::new(),
    }
}

/// `getRouters` 返回的动态路由节点（字段与 plus-ui permission store 对齐）。
///
/// 字段名/省略规则对齐上游 RuoYi-Vue-Plus 5.X 的 `RouterVo`：
/// * `JsonInclude(NON_EMPTY)` → 空值字段不下发
/// * `alwaysShow` 仅在目录节点置 `true`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterVO {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub hidden: bool,
    /// 有子路由时置 `true`（上游 `RouterVo.alwaysShow`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_show: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<RouterVO>>,
    /// 上游 `isMenuFrame` 分支会把父级 meta 置空，此处用 `None` 表达。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<MetaVO>,
}

/// 路由显示信息（对齐上游 `MetaVo`）。
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetaVO {
    pub title: String,
    pub icon: String,
    /// 内链地址（http(s):// 开头）；上游仅在 path 是合法 URL 时赋值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// 是否禁用 keep-alive 缓存；上游取 `isCache == "1"`。
    pub no_cache: bool,
    /// 侧边栏高亮目标菜单；上游取 remark 以 `/` 开头时的值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_menu: Option<String>,
}

// ---------- 部门 / 岗位 ----------

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeptVO {
    pub dept_id: i64,
    pub parent_id: i64,
    pub ancestors: String,
    pub dept_name: String,
    pub dept_category: String,
    pub order_num: i32,
    pub leader: String,
    pub phone: String,
    pub email: String,
    pub status: String,
    pub del_flag: String,
    pub create_by: i64,
    pub create_time: String,
    pub update_by: i64,
    pub update_time: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<DeptVO>,
}

pub const DEPT_COLS: &str = "dept_id, parent_id, ancestors, dept_name, dept_category, order_num, \
     leader, phone, email, status::text AS status, del_flag::text AS del_flag, create_by, \
     create_time, update_by, update_time";

pub fn dept_from_row(row: &Row) -> DeptVO {
    DeptVO {
        dept_id: i64v(row, "dept_id"),
        parent_id: i64v(row, "parent_id"),
        ancestors: s(row, "ancestors"),
        dept_name: s(row, "dept_name"),
        dept_category: s(row, "dept_category"),
        order_num: i32v(row, "order_num"),
        leader: s(row, "leader"),
        phone: s(row, "phone"),
        email: s(row, "email"),
        status: s(row, "status"),
        del_flag: s(row, "del_flag"),
        create_by: i64v(row, "create_by"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_by: i64v(row, "update_by"),
        update_time: fmt_dt(dt(row, "update_time")),
        children: Vec::new(),
    }
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PostVO {
    pub post_id: i64,
    pub dept_id: i64,
    pub post_code: String,
    pub post_name: String,
    pub post_sort: i32,
    pub status: String,
    pub remark: String,
    pub create_by: i64,
    pub create_time: String,
    pub update_by: i64,
    pub update_time: String,
    #[serde(default)]
    pub flag: bool,
}

pub const POST_COLS: &str = "post_id, dept_id, post_code, post_name, post_sort, \
     status::text AS status, remark, create_by, create_time, update_by, update_time";

pub fn post_from_row(row: &Row) -> PostVO {
    PostVO {
        post_id: i64v(row, "post_id"),
        dept_id: i64v(row, "dept_id"),
        post_code: s(row, "post_code"),
        post_name: s(row, "post_name"),
        post_sort: i32v(row, "post_sort"),
        status: s(row, "status"),
        remark: s(row, "remark"),
        create_by: i64v(row, "create_by"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_by: i64v(row, "update_by"),
        update_time: fmt_dt(dt(row, "update_time")),
        flag: false,
    }
}

// ---------- 字典 / 参数 / 通知 ----------

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DictTypeVO {
    pub dict_id: i64,
    pub dict_name: String,
    pub dict_type: String,
    pub remark: String,
    pub create_time: String,
    pub update_time: String,
}

pub const DICT_TYPE_COLS: &str =
    "dict_id, dict_name, dict_type, remark, create_time, update_time";

pub fn dict_type_from_row(row: &Row) -> DictTypeVO {
    DictTypeVO {
        dict_id: i64v(row, "dict_id"),
        dict_name: s(row, "dict_name"),
        dict_type: s(row, "dict_type"),
        remark: s(row, "remark"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_time: fmt_dt(dt(row, "update_time")),
    }
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DictDataVO {
    pub dict_code: i64,
    pub dict_sort: i32,
    pub dict_label: String,
    pub dict_value: String,
    pub dict_type: String,
    pub css_class: String,
    pub list_class: String,
    pub is_default: String,
    pub remark: String,
    pub create_time: String,
    pub update_time: String,
}

pub const DICT_DATA_COLS: &str = "dict_code, dict_sort, dict_label, dict_value, dict_type, \
     css_class, list_class, is_default::text AS is_default, remark, create_time, update_time";

pub fn dict_data_from_row(row: &Row) -> DictDataVO {
    DictDataVO {
        dict_code: i64v(row, "dict_code"),
        dict_sort: i32v(row, "dict_sort"),
        dict_label: s(row, "dict_label"),
        dict_value: s(row, "dict_value"),
        dict_type: s(row, "dict_type"),
        css_class: s(row, "css_class"),
        list_class: s(row, "list_class"),
        is_default: s(row, "is_default"),
        remark: s(row, "remark"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_time: fmt_dt(dt(row, "update_time")),
    }
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfigVO {
    pub config_id: i64,
    pub config_name: String,
    pub config_key: String,
    pub config_value: String,
    pub config_type: String,
    pub remark: String,
    pub create_time: String,
    pub update_time: String,
}

pub const CONFIG_COLS: &str =
    "config_id, config_name, config_key, config_value, config_type::text AS config_type, \
     remark, create_time, update_time";

pub fn config_from_row(row: &Row) -> ConfigVO {
    ConfigVO {
        config_id: i64v(row, "config_id"),
        config_name: s(row, "config_name"),
        config_key: s(row, "config_key"),
        config_value: s(row, "config_value"),
        config_type: s(row, "config_type"),
        remark: s(row, "remark"),
        create_time: fmt_dt(dt(row, "create_time")),
        update_time: fmt_dt(dt(row, "update_time")),
    }
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NoticeVO {
    pub notice_id: i64,
    pub notice_title: String,
    pub notice_type: String,
    pub notice_content: String,
    pub status: String,
    pub remark: String,
    pub create_by: String,
    pub create_time: String,
    pub update_time: String,
}

pub const NOTICE_COLS: &str = "notice_id, notice_title, notice_type::text AS notice_type, \
     notice_content, status::text AS status, remark, create_time, update_time";

pub fn notice_from_row(row: &Row) -> NoticeVO {
    let content = s(row, "notice_content");
    NoticeVO {
        notice_id: i64v(row, "notice_id"),
        notice_title: s(row, "notice_title"),
        notice_type: s(row, "notice_type"),
        notice_content: content,
        status: s(row, "status"),
        remark: s(row, "remark"),
        create_by: String::new(),
        create_time: fmt_dt(dt(row, "create_time")),
        update_time: fmt_dt(dt(row, "update_time")),
    }
}

// ---------- 监控 ----------

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogininforVO {
    pub info_id: i64,
    pub tenant_id: String,
    pub user_name: String,
    pub client_key: String,
    pub device_type: String,
    pub ipaddr: String,
    pub login_location: String,
    pub browser: String,
    pub os: String,
    pub status: String,
    pub msg: String,
    pub login_time: String,
}

pub const LOGININFOR_COLS: &str = "info_id, user_name, client_key, device_type, ipaddr, \
     login_location, browser, os, status::text AS status, msg, login_time";

pub fn logininfor_from_row(row: &Row) -> LogininforVO {
    LogininforVO {
        info_id: i64v(row, "info_id"),
        tenant_id: String::new(),
        user_name: s(row, "user_name"),
        client_key: s(row, "client_key"),
        device_type: s(row, "device_type"),
        ipaddr: s(row, "ipaddr"),
        login_location: s(row, "login_location"),
        browser: s(row, "browser"),
        os: s(row, "os"),
        status: s(row, "status"),
        msg: s(row, "msg"),
        login_time: fmt_dt(dt(row, "login_time")),
    }
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperlogVO {
    pub oper_id: i64,
    pub title: String,
    pub business_type: i32,
    pub method: String,
    pub request_method: String,
    pub operator_type: String,
    pub oper_name: String,
    pub dept_name: String,
    pub oper_url: String,
    pub oper_ip: String,
    pub oper_location: String,
    pub oper_param: String,
    pub json_result: String,
    pub status: String,
    pub error_msg: String,
    pub oper_time: String,
    pub cost_time: i64,
}

pub const OPERLOG_COLS: &str = "oper_id, title, business_type, method, request_method, \
     operator_type::text AS operator_type, oper_name, dept_name, oper_url, oper_ip, \
     oper_location, oper_param, json_result, status::text AS status, error_msg, \
     oper_time, cost_time";

pub fn operlog_from_row(row: &Row) -> OperlogVO {
    OperlogVO {
        oper_id: i64v(row, "oper_id"),
        title: s(row, "title"),
        business_type: i32v(row, "business_type"),
        method: s(row, "method"),
        request_method: s(row, "request_method"),
        operator_type: s(row, "operator_type"),
        oper_name: s(row, "oper_name"),
        dept_name: s(row, "dept_name"),
        oper_url: s(row, "oper_url"),
        oper_ip: s(row, "oper_ip"),
        oper_location: s(row, "oper_location"),
        oper_param: s(row, "oper_param"),
        json_result: s(row, "json_result"),
        status: s(row, "status"),
        error_msg: s(row, "error_msg"),
        oper_time: fmt_dt(dt(row, "oper_time")),
        cost_time: i64v(row, "cost_time"),
    }
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OnlineVO {
    pub token_id: String,
    pub user_name: String,
    pub nick_name: String,
    pub dept_name: String,
    pub ipaddr: String,
    pub browser: String,
    pub os: String,
    pub login_time: String,
}

/// 树形下拉统一结构（deptTree / treeselect 返回 `data` 为数组，元素含 id/label/children）。
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TreeSelectVO {
    pub id: i64,
    pub label: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeSelectVO>,
}

/// 成功响应的便捷写法占位：让 handlers 少写一层泛型。
pub type JsonVoid = Json<crate::resp::Resp<()>>;
