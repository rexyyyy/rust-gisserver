//! 系统管理处理器。
pub mod avatar;
pub mod dept;
pub mod menu;
pub mod misc;
pub mod role;
pub mod user;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

/// 系统管理域路由（用户 / 角色 / 菜单 / 部门 / 岗位 / 字典 / 参数 / 通知）。
///
/// 由 [`crate::handlers::router`] 统一装配：各域 `routes()` 只管注册，
/// 鉴权中间件与 `with_state` 折叠在装配处统一完成。静态路径先于动态路径。
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // 用户（静态优先）
        .route("/system/user/list", get(user::list))
        .route("/system/user/deptTree", get(user::dept_tree))
        .route("/system/user/optionselect", get(user::option_select))
        .route("/system/user/profile", get(user::profile).put(user::update_profile))
        .route("/system/user/profile/updatePwd", put(user::update_pwd))
        .route("/system/user/profile/avatar", post(avatar::upload))
        .route("/system/user/changeStatus", put(user::change_status))
        .route("/system/user/resetPwd", put(user::reset_pwd))
        .route("/system/user/authRole", put(user::auth_role))
        .route("/system/user/authRole/{userId}", get(user::auth_role_get))
        .route("/system/user/list/dept/{deptId}", get(user::list_by_dept))
        .route("/system/user", post(user::add).put(user::update))
        .route("/system/user/{userId}", get(user::get).delete(user::delete))
        // 角色
        .route("/system/role/list", get(role::list))
        .route("/system/role/optionselect", get(role::option_select))
        .route("/system/role/changeStatus", put(role::change_status))
        .route("/system/role/dataScope", put(role::data_scope))
        .route("/system/role/authUser/allocatedList", get(role::allocated))
        .route("/system/role/authUser/unallocatedList", get(role::unallocated))
        .route("/system/role/authUser/selectAll", put(role::select_all))
        .route("/system/role/authUser/cancel", put(role::cancel))
        .route("/system/role/authUser/cancelAll", put(role::cancel_all))
        .route("/system/role/deptTree/{roleId}", get(role::dept_tree))
        .route("/system/role", post(role::add).put(role::update))
        .route("/system/role/{ids}", get(role::get).delete(role::delete))
        // 菜单
        .route("/system/menu/list", get(menu::list))
        .route("/system/menu/treeselect", get(menu::treeselect))
        .route("/system/menu/roleMenuTreeselect/{roleId}", get(menu::role_menu_treeselect))
        .route("/system/menu/cascade/{menuId}", delete(menu::cascade_delete))
        .route("/system/menu", post(menu::add).put(menu::update))
        .route("/system/menu/{menuId}", get(menu::get).delete(menu::delete))
        // 部门
        .route("/system/dept/list", get(dept::list))
        .route("/system/dept/optionselect", get(dept::option_select))
        .route("/system/dept/list/exclude/{deptId}", get(dept::exclude))
        .route("/system/dept", post(dept::add).put(dept::update))
        .route("/system/dept/{deptId}", get(dept::get).delete(dept::delete))
        // 岗位
        .route("/system/post/list", get(misc::post_list))
        .route("/system/post/optionselect", get(misc::post_option_select))
        .route("/system/post/deptTree", get(dept::dept_tree))
        .route("/system/post", post(misc::post_add).put(misc::post_update))
        .route("/system/post/{ids}", get(misc::post_get).delete(misc::post_delete))
        // 字典
        .route("/system/dict/type/list", get(misc::dict_type_list))
        .route("/system/dict/type/optionselect", get(misc::dict_type_option_select))
        .route("/system/dict/type/refreshCache", delete(misc::dict_refresh))
        .route("/system/dict/type", post(misc::dict_type_add).put(misc::dict_type_update))
        .route("/system/dict/type/{ids}", get(misc::dict_type_get).delete(misc::dict_type_delete))
        .route("/system/dict/data/list", get(misc::dict_data_list))
        .route("/system/dict/data/type/{dictType}", get(misc::dict_data_by_type))
        .route("/system/dict/data", post(misc::dict_data_add).put(misc::dict_data_update))
        .route("/system/dict/data/{ids}", get(misc::dict_data_get).delete(misc::dict_data_delete))
        // 参数
        .route("/system/config/list", get(misc::config_list))
        .route("/system/config/configKey/{key}", get(misc::config_by_key))
        .route("/system/config/refreshCache", delete(misc::config_refresh))
        .route("/system/config/updateByKey", put(misc::config_update_by_key))
        .route("/system/config", post(misc::config_add).put(misc::config_update))
        .route("/system/config/{id}", get(misc::config_get).delete(misc::config_delete))
        // 通知
        .route("/system/notice/list", get(misc::notice_list))
        .route("/system/notice", post(misc::notice_add).put(misc::notice_update))
        .route("/system/notice/{ids}", get(misc::notice_get).delete(misc::notice_delete))
}
