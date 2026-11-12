# r-manager 后端（manager）

r-mapserver 的系统管理后端：axum 0.8 + tokio-postgres（PostgreSQL 单库）。负责
登录鉴权（JWT + 验证码）、RBAC（用户/角色/菜单/部门/岗位）、字典/参数/通知、
监控审计（在线用户/登录日志/操作日志），以及对 r-mapserver `map-api` 管理面
（`/admin/*`、`/api/usage`）的统一代理。前端见 `../ui`（Vue3 + Vite）。

## 模块职责

| 模块 | 职责 |
| --- | --- |
| `main.rs` | 组合根：加载配置 → 建连接池 → 构造 `AppState` → 起服务 + 引擎巡检后台任务 |
| `config.rs` / `xmlcfg.rs` | XML 配置（`config/manager.xml`）桥接为 JSON 后反序列化；`--config <path>` 可覆盖 |
| `state.rs` | `AppState`（连接池/配置/在线会话表/权限缓存/头像根目录/map-api HTTP 客户端）与 `query`/`execute` 帮助函数 |
| `auth.rs` | JWT 签发/校验、鉴权中间件 `auth_mw`、白名单判定 |
| `perm.rs` | 按钮级权限判定（含单测） |
| `captcha.rs` | 验证码（自绘 SVG）与内存会话表 |
| `error.rs` / `resp.rs` | 统一错误 `AppError` 与 `{code,msg,data}` 响应契约 |
| `models.rs` / `util.rs` | VO / 列清单 与 分页、查询构造工具 |
| `engine_monitor.rs` | 引擎存活后台巡检（30s），状态变化写 `sys_engine_event` |
| `handlers/` | 路由与处理器，按业务域分模块（见下） |
| `staticfiles.rs` | 前端静态资源托管（SPA 同源部署）：`ServeDir` + SPA fallback + 缓存头 + 路径穿越防护 |

## handlers 路由装配（各司其职）

路由注册下放到各业务域模块的 `routes()` 构造器（返回 `Router<Arc<AppState>>`），
`handlers/mod.rs` 只做装配，不注册业务路由：

```
auth::routes()      认证：/auth/*、captchaImage、/system/user/getInfo、/system/menu/getRouters
system::routes()    系统管理：/system/*（用户/角色/菜单/部门/岗位/字典/参数/通知）
engine::routes()    服务引擎：/map/engines*、服务归属引擎 /map/services/{kind}/{name}/engine
map::routes()       地图服务：/map/services*、分组、样式库、缓存、用量、reload 等
monitor::routes()   监控审计：/monitor/*
```

装配链：`merge` 各域路由 → 统一套鉴权中间件（白名单内放行）→ `with_state`
折叠为 `Router<()>` → `nest` 到 `base_api`。顶层另挂头像静态回显（不进
base_api、不挂鉴权——浏览器 `<img>` 不带 Authorization 头）与 `/health`。

注意（axum type-state）：带 `State` 提取器的路由构成 `Router<Arc<AppState>>`；
新增业务域时在自己的 `routes()` 里注册即可，不要用 `Router::new()`（即 `Router<()>`）
去 merge 带状态的子路由，类型不匹配会编译失败。

### 路由参数语法：必须用 `{param}`（axum 0.8）

axum 0.8 的 matchit 0.8 **不再支持** 0.7 的 `:param` / `*rest` 写法，
遇到旧语法会在**注册路由时直接 panic**：

```text
Path segments must not start with `:`. For capture groups, use `{capture}`.
```

| 用途 | 0.7 旧写法（已废弃） | 0.8 正确写法 |
| --- | --- | --- |
| 单段参数 | `/system/user/:userId` | `/system/user/{userId}` |
| 跨段通配 | `/profile/avatar/*path` | `/profile/avatar/{*path}` |

⚠️ **权限表必须同步迁移**：`src/perm.rs` 的 `PERM_RULES` 是**纯数据**，
用旧语法不会 panic，只会让 `match_pattern` 静默失配（鉴权全线拒绝或误放行）。
`perm::tests::perm_rules_use_axum08_brace_syntax` 是这条约束的守卫测试。

`perm.rs` 的 `match_pattern` 同时兼容两种写法（`{x}` / `:x`），但规则表本身
统一用 `{param}`。

迁移脚本见 `tools/migrate_axum08_routes.py`（按路径段切分后逐段判定，
避免把普通段 `/map` 误改成 `/{map}`）。

## 构建 / 运行 / 测试

```bash
cargo build            # 需 MSVC 工具链（jsonwebtoken→ring 等需要链接）
cargo test             # 单测：perm.rs、xmlcfg.rs（bin crate 内联 #[cfg(test)]，cargo test 直跑）
cargo run              # 默认读 config/manager.xml
```

运行依赖：PostgreSQL（`database.url`）；可选 r-mapserver map-api（`mapsvc.base_url`，
默认 `http://127.0.0.1:8095`，多引擎部署时以 `sys_engine` 注册表为准）。

## 前端部署：同端口一体化（static_dir）

后端可选地**同端口同源**托管前端产物 `ui/dist`，实现单进程部署：

```xml
<server addr="0.0.0.0:6180" base_api="/manager-api" static_dir="../ui/dist">
```

* **相对路径按后端进程工作目录解析**（与 `storage.avatar_dir` 一致）；
  生产部署建议用绝对路径，避免工作目录变化导致找不到。
* **留空 = 不启用**，本进程只提供 API —— 此时前端由 Vite dev server 或
  独立静态服务器提供。两种模式可共存，切换只需改配置。
* 前端 API 用的是**相对路径** `/manager-api`（`VITE_APP_BASE_API`），
  同源部署天然**零 CORS**，无需额外配置。

### 路由优先级

```text
/manager-api/**   → 业务 API（自带 fallback，返回 JSON 404）
/profile/avatar/**→ 头像静态回显（顶层，不鉴权）
/health           → 健康检查
其余路径          → 静态资源（fallback）：命中文件则返回，否则回落 index.html
```

API 子路由**必须**自带 fallback（`api_not_found`）：axum 的 `nest` 会把子路由的
fallback 一并嵌到 `/manager-api/*` 下，若不挂，未匹配的 API 路径会掉到静态
fallback 被 SPA 回落成 `index.html`，前端拿到 HTML 却期望 JSON，极难排查。

### SPA fallback（history 模式必需）

前端用 `createWebHistory`，路由如 `/map/service` 在磁盘上并不存在。
未命中静态文件的请求一律回落 `index.html`，交给前端路由接管，
否则**刷新页面直接 404**。

### 缓存策略

| 路径 | Cache-Control | 理由 |
| --- | --- | --- |
| `/assets/*` | `public, max-age=31536000, immutable` | 文件名带内容 hash，可长期缓存 |
| `/index.html`、`/` | `no-cache` | 必须每次校验，否则前端发版后用户拿到旧壳子配新资源而白屏 |

### 路径安全

`ServeDir` 内部经 `open_file` 校验（含 `..` 判为 `InvalidFilename`），
另有 `percent-encoding` 解码。实测：`%2e%2e` 会被 axum/matchit 在路由匹配阶段
解码并规范化（收敛到 dist 内）；`%2f`（编码斜杠）保留原形，由 `staticfiles.rs`
的显式校验拦下返回 400。金丝雀实测 6 种穿越变体**零泄漏**。

### 独立静态服务器方案（备选）

若前端需独立部署/CDN，改用 Nginx 反代，后端 `static_dir` 留空：

```nginx
server {
    listen 80;
    root /opt/r-mapserver/ui/dist;
    location / { try_files $uri $uri/ /index.html; }   # SPA fallback
    location /manager-api/ {
        proxy_pass http://127.0.0.1:6180/manager-api/;
    }
}
```
