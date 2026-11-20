# manager —— rust-gisserver 管理系统（独立项目）

rust-gisserver 的「系统管理 + 地图服务管理」控制台：后端用 Rust（axum + tokio-postgres，仅 PostgreSQL）重写若依（RuoYi-Vue 5.X）的系统管理接口，前端沿用 plus-ui-5.X 的 Element Plus 实现并裁剪。

**独立项目**：2026-10-26 起从 `rust-gisserver/manager` 拆出，位于 `thirdparty/manager`，git 独立管理（拆分前历史保留在 rust-gisserver 仓库，最后共同提交 `ce576ae`）。与 rust-gisserver（map-api，8080）的对接关系见 rust-gisserver 仓库 `docs/manager-scope-analysis-2026-10-26.md`。

## 目录结构

```
manager/
├─ backend/                  Rust 后端（axum 0.7 + tokio-postgres）
│  ├─ src/
│  │  ├─ main.rs             启动入口：加载配置、探库、建连接池
│  │  ├─ config.rs           YAML 配置（server / database / security / storage）
│  │  ├─ state.rs            共享状态：连接池、在线会话、权限缓存
│  │  ├─ auth.rs             JWT 签发/校验 + 鉴权中间件 + 权限判定
│  │  ├─ perm.rs             菜单/按钮权限装配
│  │  ├─ models.rs           行 → VO 的映射
│  │  ├─ resp.rs / error.rs  统一响应与错误
│  │  └─ handlers/           各业务模块 handler
│  │     ├─ auth.rs          登录/登出/验证码/getInfo/getRouters
│  │     ├─ system/          user / role / menu / dept / misc / avatar
│  │     └─ monitor.rs       在线用户 / 登录日志 / 操作日志
│  ├─ config/                manager.yaml（6180）、manager.probe.yaml（6181）
│  ├─ sql/                   system.sql（系统板块）/ map.sql（地图与数据板块）
│  └─ storage/avatar/        头像落盘目录（按 年/月/日 分层）
└─ ui/                       前端（Vue 3 + Vite + Element Plus）
   ├─ src/views/system/      用户/角色/菜单/部门/岗位/字典/参数/公告
   ├─ src/views/monitor/     在线用户/登录日志/操作日志
   └─ vite.config.ts         dev 代理：/manager-api 与 /profile → 后端 6180
```

## 快速开始

### 1. 数据库

```bash
# 建库（库名含连字符，SQL 标识符必须加双引号；外层改用单引号包裹）
psql -h localhost -p 5434 -U postgres -c 'CREATE DATABASE "mapserver-manager"'

# 建表 + 初始化数据（两个文件都要跑；顺序不敏感，均可重复执行）
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/system.sql
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/map.sql
```

两个脚本都是**全量幂等**的：建表 `create ... if not exists`、索引 `create index if not exists`、
写入 `on conflict do nothing`、排序归一按 `menu_id` 写回。已有库重复执行安全。

`system.sql` 覆盖系统管理与监控板块（建表 + 用户/角色/字典/参数/公告种子）；
`map.sql` 覆盖地图服务与数据服务板块（引擎注册表、组织层四表、两个顶级目录的菜单）。

### 英文初始数据（安装时二选一）

`system.en_US.sql` / `map.en_US.sql` 由 `manager/tools/gen_i18n_sql.py` 依据
`manager/tools/i18n_terms.json` 生成，只替换种子数据里的中文文本（菜单名、字典项、公告、
组织名、`COMMENT ON` 对象说明），DDL、列顺序、行数与中文版完全一致。

**必须与中文版二选一执行**，不允许在同一库先后跑两套：种子写入用 `on conflict do nothing`，
后跑的一套会静默失效，菜单/字典仍停留在先安装的语言。

```bash
# 英文版（替代上面两条中文命令，不要两套都跑）
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/system.en_US.sql
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/map.en_US.sql
```

新增中文种子词条后需同步补 `i18n_terms.json` 并重新生成，禁止手工编辑生成物：

```bash
python manager/tools/gen_i18n_sql.py          # 重新生成
python manager/tools/gen_i18n_sql.py --check  # 校验生成物是否最新（CI 用）
```

### 已有库切换语言（不要重装）

`system.en_US.sql` / `map.en_US.sql` 只在空库安装时有效；已有中文种子的库直接执行会因
`on conflict do nothing` 静默失效。这种情况用就地切换补丁，不清空任何表：

```bash
# 生成补丁（中文基线或 i18n_terms.json 变更后重新生成）
python manager/tools/gen_i18n_update_sql.py
python manager/tools/gen_i18n_update_sql.py --check

# 单事务执行（-1 保证出错整体回滚）
psql -h localhost -p 5434 -U postgres -d mapserver-manager -v ON_ERROR_STOP=1 -1 -f manager/backend/sql/switch_to_en_US.sql

# 回滚到中文
psql -h localhost -p 5434 -U postgres -d mapserver-manager -v ON_ERROR_STOP=1 -1 -f manager/backend/sql/switch_to_zh_CN.sql
```

补丁按主键配对中英文基线中相同的种子行，只更新有差异的文本列（菜单名、字典项、公告、
组织名、对象注释），不动用户自建行、运行时表与 `perms` / `password` / `config_key` 等业务字段。
英文译文超出 `varchar(n)` 的列会先放宽上限（只放宽、不改数据）；对象注释在容错块中执行，
库里缺少基线对象时自动跳过。两个方向的补丁互为回滚，均可重复执行。
`switch_to_*.sql` 是生成物，禁止手工编辑。

后端用户可见提示由请求头 `X-Language: zh_CN|en_US` 决定（缺省回落 `Accept-Language`，
再缺省 `zh_CN`），前端 axios 拦截器统一注入；详见 `docs/i18n-and-comment-style.md`。

### 顶级菜单 ID 与顺序

地图板块重排为「数据服务在前、地图服务在后」，顶级 ID 也随之收敛：

| menu_id | 名称 | 原 ID | 说明 |
|---|---|---|---|
| 6 | 数据服务 | 123 | 工作空间 / 数据源 / 数据图层 / 已发布服务 |
| 7 | 地图服务 | 6 | 服务清单 / 日志 / 样式 / 引擎 / 缓存等 |

改号只涉及 `sys_menu.menu_id`、`sys_menu.parent_id`、`sys_role_menu.menu_id` 三处
（`sys_menu` 除主键外无外键约束，全库无其他引用），由两个脚本开头的 `§0` 迁移块完成：
旧号先搬到临时高位号再落到目标号，避免主键撞车；全程包在 `do $$` 块里，异常整体回滚。
迁移块在两个文件中各放一份，因此**执行顺序不影响结果**。

前端无需清缓存：动态路由的 `name` 由后端运行时按 `capitalize(path)+menu_id` 生成，
不落库持久化。

### 2. 后端

```bash
cd backend

# 构建（Windows 需 MSVC 环境，见 build_manager.bat）
build_manager.bat release        # 或 debug

# 启动（默认读 config/manager.xml，监听 0.0.0.0:6180）
./target/release/manager.exe
./target/release/manager.exe --config config/manager.probe.xml   # 6181，验证码关闭，供探针使用
```

### 3. 前端

```bash
cd ui
pnpm install
pnpm run dev            # http://localhost:5100
pnpm run build:prod     # 产物在 dist/
```

## 配置说明

`backend/config/manager.xml`（配置格式为 XML：属性承载标量，`<item>` 是列表元素）：

| 段 | 键 | 说明 |
|---|---|---|
| server | `addr` | 监听地址，默认 `0.0.0.0:6180` |
| server | `base_api` | 最外层路由前缀，需与前端 `VITE_APP_BASE_API` 一致（`/manager-api`） |
| server | `static_dir` | 前端 `dist/` 目录；配置后同端口托管 SPA（一体化部署），留空则只提供 API |
| server | `cors_origins` | 允许的前端来源（开发态 vite 端口） |
| database | `url` | PostgreSQL 连接串 |
| database | `pool_max` | 连接池上限 |
| mapsvc | `base_url` | 默认 map-api 引擎地址；`sys_engine` 表有启用中的默认引擎时以表为准 |
| security | `jwt_secret` | JWT 密钥，生产必须换随机长串 |
| security | `token_expire_minutes` | Token 有效期（分钟） |
| security | `captcha_enabled` | 是否开启图形验证码 |
| security | `white_list` | 免鉴权路径（相对 base_api） |
| storage | `avatar_dir` | 头像存储根目录；相对路径按进程工作目录解析，生产建议给绝对路径 |

## 头像上传（本地存储）

前端 `userAvatar.vue` 调 `POST {base_api}/system/user/profile/avatar`，multipart 字段名 `avatarfile`，期望响应 `data.imgUrl`。

- **落盘**：`storage/avatar/{YYYY}/{MM}/{DD}/u{userId}_{uuid}.{ext}`，扩展名由文件头魔数判定（仅 PNG/JPEG），不信任客户端 content-type；单文件上限 5MB。
- **回写**：`UPDATE sys_user SET avatar = '/profile/avatar/...'`（字段为 `varchar(500)`，存相对路径）。
- **回显**：`GET /profile/avatar/*path` 挂在顶层路由，**不进 base_api、不挂鉴权中间件** —— 浏览器 `<img src>` 不会带 Authorization 头，走鉴权会导致 401。开发态由 vite 的 `/profile` 代理转发到后端。
- **安全**：拒绝 `..`、绝对路径、反斜杠；`canonicalize` 后校验仍在根目录内。

> 注意：根目录在启动时做一次 `canonicalize`（Windows 下会带 `\\?\` 前缀）。若改动根目录构造方式，务必同步 `starts_with` 的比较基准，否则回显会 400。

## 接口探针

`tools/mgr_probe.py` 覆盖登录、增删改查、越权、头像上传/回显/回写等 66 项断言：

```bash
python tools/mgr_probe.py                                    # 默认打 6181
python tools/mgr_probe.py http://127.0.0.1:6180/manager-api    # 指定实例
python tools/mgr_probe.py http://127.0.0.1:6180/manager-api out.txt
```

结果写入 `mgr_probe.txt`（或指定文件），末尾输出「合计: 通过 N / 失败 M」。

辅助诊断脚本：`avatar_probe.py`（头像单测）、`route_diag.py`（路由方法探测）、`ct_diag.py`（区分「路由未注册」与「content-type 被拒」）。

## 端到端功能测试（与 rust-gisserver 联动）

`tools/func_test.py` 针对**已运行的两个实例**做接口级走查（68 项），数据源为真实
图层目录（`rust-gisserver/config/*` + `E:\data\1` + PostGIS `gis` 库），覆盖：

- **map-api 数据面**：COG（WMS 1.1.1/1.3.0、WMTS 两套 TileMatrixSet、XYZ、`/cog` 直读）、
  矢量（WFS、OGC API Features/Tiles、`/mvt`）、PostGIS 动态 MVT（3857 与原投影双接口）、
  PMTiles / MBTiles 归档、3D Tiles、terrain、`/admin/*` 与 `/api/cache`，以及
  「请求问题 / 真故障」的 400/404/500 分流语义；
- **manager 管理面**：admin 与 role3 登录、getInfo / getRouters、服务跨引擎聚合与
  分页过滤、引擎列表 / 健康 / 详情 / 事件、服务分组、访问日志、配置读写回环
  （PUT 回写原文，内容不变）、服务归属引擎回环、鉴权与权限拒绝路径；
- **同端口 SPA 托管**：`/index.html`、前端路由回退、静态资源可达。

```bash
# 1) 起 map-api（rust-gisserver 仓库根目录，监听 :8080）
#    瓦片链路务必用 release 构建，debug 下解码/编码慢一个数量级
target\release\mapserve.exe serve --config config/mapserver.xml
# 2) 起 manager（backend 目录，监听 :6180，代理到 sys_engine 默认引擎）
target\debug\manager.exe
# 3) 跑联合功能测试
python tools/func_test.py [mapapi_base] [manager_base]
```

> 退出码即失败用例数，`0` 为全部通过。断言全部基于响应状态/内容类型/字节魔数
> （PNG、MVT protobuf）与 JSON 结构，不依赖人工判读。

## 前端联调冒烟

`tools/ui_smoke.py` 验证「vite dev → 代理 → 后端」整链路（6 项）：

```bash
# 1) 后端起在 6180（vite proxy 目标端口）
cd backend && ./target/release/manager.exe
# 2) 前端起在 5100
cd ui && pnpm run dev
# 3) 跑冒烟
python tools/ui_smoke.py
```

覆盖：首页可达、代理登录、代理取 getInfo、代理上传头像、代理回显头像（`<img>` 路径，不带 token）、路径穿越被拒。

## 动态路由契约（getRouters）

前端登录后调 `GET /system/menu/getRouters` 拿动态路由树，用 `router.addRoute()` 逐条注册。
**下发的 path/component 必须与上游 RuoYi-Vue-Plus 5.X 完全一致**，否则整棵动态路由注册失败、侧边栏一点就 404。

后端实现在 `backend/src/perm.rs::to_routers`，严格镜像上游 `SysMenuServiceImpl.buildMenus` + `SysMenu.getRouterPath/getComponentInfo`：

| 场景 | 判定条件 | path | component |
|---|---|---|---|
| 一级目录 | `parent_id=0` + `type=M` + `is_frame='1'` | `/` + path | `Layout` |
| 一级菜单 | `parent_id=0` + `type=C` + `is_frame='1'` | `/` | `Layout`，派生出唯一子路由 |
| 普通菜单 | 其余 | 原 path | DB 的 component |
| 子目录 | `parent_id≠0` + `type=M` + 无 component | 原 path | `ParentView` |
| 真外链 | `is_frame='0'` + http 地址 | 原 http 地址（不补 `/`） | `Layout` |
| 内链 | `is_frame='1'` + http 地址 | `innerLinkReplaceEach` 转义 | `InnerLink` |

`is_frame` 语义（上游 `SystemConstants`）：`"0"`=是外链（YES_FRAME，新窗口打开）、`"1"`=否（NO_FRAME，站内嵌）。
`meta.noCache` 取 `is_cache == "1"`；`meta.link` 仅当 path 是 http 时赋值；`meta.activeMenu` 取 remark 以 `/` 开头的值。

### 排查要点

- **顶层路由 path 必须以 `/` 开头**：vue-router 5.x 的 `addRoute` 对根级 path 强制要求，否则抛
  `Route paths should start with a "/"`，被 `permission.ts` 的 try/catch 吞掉后落回 `/index`，表现为「全站 404」。
- **`meta` 字段名是驼峰**（`noCache` / `activeMenu` / `alwaysShow`）：前端 `tagsView.ts`、`TopBar`、`Sidebar` 都按驼峰读，
  下发成 `no_cache` 会让 keep-alive 判断恒失效。
- 导出真实下发内容做前后端契约核对：

```bash
python tools/dump_routers.py http://127.0.0.1:6180/manager-api routers_after.json
```

## 按钮级权限契约

后端在 `backend/src/perm.rs::allows` 做 URL 级 + 按钮级校验（替代 casbin）。

**不做通用路径推导**：上游 RuoYi 用控制器注解 `@SaCheckPermission("...")` 显式声明权限，
同一资源下不同端点的权限码可能不同（`GET /system/user/{id}` 是 `query` 而非 `list`），
带 id 的路径也无法从路径反推动作名 —— `DELETE /system/post/7` 会误推成 `system:post:7`，
导致**非超管角色的写操作全部被拒**。

因此 `perm.rs` 维护两张显式表，逐条对齐上游注解：

| 表 | 作用 | 上游对应 |
|---|---|---|
| `PERM_RULES` | (方法, 路径模式, 权限码)，`:x` 段通配任意单段 | 各控制器的 `@SaCheckPermission` |
| `NO_PERM_PATTERNS` | 上游**明确无注解**的端点，只需登录态 | `SysProfileController`、`getInfo`、`myself` 等 |

| 场景 | 判定 | 权限码 |
|---|---|---|
| 列表 | `GET /xxx/list` | `xxx:list` |
| 详情 | `GET /xxx/{id}` | `xxx:query`（不是 `list`） |
| 新增 | `POST /xxx` | `xxx:add` |
| 修改 | `PUT /xxx` | `xxx:edit` |
| 删除 | `DELETE /xxx/{id}` | `xxx:remove` |
| 字典 | type 与 data 共用 | `system:dict:*` |

> 超管（`user_id=1` 或角色含 `admin`）直接放行，其余按表匹配。
> 规则表未覆盖的路径：读操作放行（依赖菜单可见性），写操作一律拒绝。

实测脚本（用 `test` 角色验证按钮级权限确实生效）：

```bash
python tools/perm_probe.py http://127.0.0.1:6180/manager-api
```

规则表静态校验（查冲突 / 漏覆盖 / 语义错，改完 `perm.rs` 必跑）：

```bash
python tools/check_perm_rules.py
```

输出「冲突 0 ／ 未覆盖端点 0 ／ 语义错 0」即通过。

> **回归时不要只用 admin 测**：admin 在 `allows()` 里直接放行，会掩盖权限推导错误。
> 必须额外用 `test`（role 3）或 `test1`（role 4）跑一遍写操作。

## 构建脚本（Windows 沙箱环境）

| 脚本 | 用途 |
|---|---|
| `backend/build_manager.bat [debug\|release]` | 后端 cargo build；检测到 PATH 中已有 cl.exe 则直接使用系统 MSVC 环境，否则回退脚本内登记的 VS/SDK 路径 |
| `ui/build_ui.bat [dev\|prod]` | 前端 vite build（脚本自身定位目录，优先系统 node/pnpm） |
| `tools/ui_build.py [install\|prod\|dev]` | 同上，Python 版（显式设置 node PATH） |

> `build_manager.bat` 手工设置 `PATH/LIB/INCLUDE` 而不调 `vcvars64.bat` —— 后者依赖 `reg.exe` 探测 Windows SDK 路径，在受限环境里会被拦。

## 与主项目的边界

- 本子系统**只依赖 PostgreSQL**，不依赖 rust-gisserver 的 PostGIS 数据源、瓦片缓存、SLD 渲染等能力，可独立部署。
- 不复用 rust-gisserver 的 crate；后端是独立 Cargo package（`backend/`），单独 `cargo build`，与 map-api 工作区互不影响。
- 未来若要合并（系统管理作为主服务的一部分），需统一 `base_api`、鉴权中间件与配置结构。

## 许可证 / License

本项目（rust-gisserver）采用 **GNU Affero General Public License v3.0**（SPDX：`AGPL-3.0-only`）授权，全文见 [LICENSE](LICENSE)。

Copyright (c) 2026 rex yang <13556388@qq.com> and contributors.