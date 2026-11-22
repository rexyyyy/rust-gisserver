# 三仓联通链路与 data_root（开发计划 P0-4 · 第一阶段）

目标：把 **gis-srv 加工产物 → rust-gisserver 发布 → rust-gisserver 管理/发布** 串成一条
与机器无关的链路。核心是一个统一的**数据根 `data_root`**，配置里只写占位、
绝不写死盘符（换机不碎）。

## 链路

```
gis-srv（能力底座/数据工厂, :7070）
      │  加工产物落数据根：COG / GeoJSON / MBTiles / PMTiles / 3D Tiles
      ▼
data_root（默认 ./data；外置盘时用 DATA_ROOT 注入）
      │
rust-gisserver map-api（发布平面, :8080）
      │  图层配置文件 <source path="${DATA_ROOT:-data}/…">；
      │  server.data_root 限定「新增服务」只能引用其内文件
      ▼
rust-gisserver（管理平面, :6180）
         Workspace → Store(base_path，须在 DATA_ROOT 内) → Resource → 发布
两种目录后端（map-api `catalog.backend`）：
  · files：POST /admin/services 让 map-api 落图层 XML（<catalog.root>/<type>/*.xml）
  · db   ：service_doc(JSONB) 落 sys_published_layer，map-api 启动/热重载直接读库
           （配置已从 XML 文件进一步走到 PG 目录发布，manager 是该表唯一写入方）
```

## data_root 机制

- **配置加载即展开**：`xmlcfg::parse_to_value` 顶部调 `expand_env`，把
  `${VAR}` / `${VAR:-默认}` 用环境变量替换（rust-gisserver 与 rust-gisserver 两份同构
  `xmlcfg.rs`）。因此 `${DATA_ROOT}` 可写在 mapserver.xml、各图层 xml、以及
  任何经该入口解析的配置里。
- **两条目录后端都展开**：`files` 后端的图层 XML 经 `parse_to_value` 展开；
  `db` 后端（PG 配置发布）在 `db_catalog::load_catalog` 里对 `service_doc` JSONB
  文本先跑 `expand_env` 再 `from_str`——所以 `${DATA_ROOT}` 占位在**磁盘 XML 与
  PG 库**里等价生效，`db` 后端与其「零语义差异」的约定名副其实。绝对路径不含
  `${`，不受影响（纯增量）。
- **两个根**（数据根目录与测试数据根目录）：
  | 占位 | 含义 | 典型值 |
  |---|---|---|
  | `${DATA_ROOT:-data}` | 真实数据根（cog/vector/mbtiles/pmtiles/3dtiles + archives） | 默认 `data`（相对启动工作目录）；也可设为任意绝对路径 |
  | `${TESTDATA_ROOT}` | e2e 转换产物根 | 按测试环境设置 |
- **默认值语义（2026-11-14 起）**：`mapserver.xml` 用
  `data_root="${DATA_ROOT:-data}"`，不设 env 即以相对路径 `data` 生效，等价于
  安装目录 `data\`（`start-*.bat` 会先 `cd /d` 到安装目录；源码开发从
  `mapserver\` 启动则是 `mapserver\data`）。`within_data_root` 仍对**空串**
  不设限，因此想恢复「向导可浏览全盘」必须显式把 `DATA_ROOT` 设为空，或删掉
  `server` 的 `data_root` 属性——这是有意收紧的默认边界，不是零破坏。
  守卫会 `canonicalize` 该目录，目录不存在会 500，安装脚本与源码 `.gitkeep`
  保证其存在；真实数据文件被 `.gitignore` 挡在仓库外。

## 怎么设

**容器（推荐演示）**：compose 已把 `./data` 挂到 map-api `/app/data`，容器配置模板
固定 `data_root=/app/data`，无需额外 env。数据文件放进 `deploy/data/` 即可。

**本地安装 / `cargo run map-api`**：默认无需任何数据根变量，配置加载器取
`${DATA_ROOT:-data}` → 工作目录下 `data\`。把 COG / 瓦片放进该目录即可。
配置加载器不读 `.env`；只有数据在外置盘、或跑 e2e fixture 时才在会话注入：
```powershell
$env:DATA_ROOT     = '<数据根目录>'   # 可选覆盖，默认 .\data
$env:TESTDATA_ROOT = '<测试数据根目录>'
```
`TESTDATA_ROOT` 未设置时相关 e2e demo 图层会被**跳过并告警**（非致命，其余服务照常）。

## 换机不碎 · 现状与后续（务必知悉）

- **现状**：配置文件里的**路径引用已去机器化**（`${DATA_ROOT}` 占位，仓库不含固定盘符路径），
  且加载器对**磁盘 XML 与 PG `service_doc`** 两条后端都会 `expand_env`。但「新增服务」
  写入时（map-api `catalog_admin::create` 的 `guard_data_root` 仅**校验**、结果丢弃；
  manager `publish` 用 browse 得到的**绝对** `source.path`），无论落 XML 还是落库
  `service_doc`，存的仍是**绝对路径** —— 即 data_root 只做到了**边界约束**，没做到
  **相对化存储**。换机/换根时，运行期新增的图层需重新登记。
- **后续（超出 P0-4「集成胶水」范围，建议列 P1）**：
  1. map-api `create` 写入前把绝对路径规约为 `${DATA_ROOT}` 相对（返回 `guard_data_root` 的相对形式）；
  2. serve 端打开 source.path 时按 `server.data_root` 拼接相对路径；
  3. manager `publish` 存 `res.source` 为 base_path 相对（files 落 XML、db 落 `service_doc`）。
  三者齐备后，因 `service_doc` 已接 `expand_env`，`${DATA_ROOT}` 占位在两种后端下
  都能真正「换机不碎」。

## 自检 / 一键

```powershell
cd rust-gisserver/deploy
copy .env.example .env          # 只填口令；data_root 默认 ./data，容器固定 /app/data
pwsh -File start.ps1            # 构建起栈 -> 等 PostGIS healthy -> 自动 validate
pwsh -File validate.ps1 -CheckDataRoot   # 也可对已在跑的服务单独自检
```
`validate.ps1` 检查：PostGIS TCP、map-api `/health`、WMS GetCapabilities、
`/admin/components`（打印当前 data_root）、manager 根、（可选）默认 `./data`
或 `DATA_ROOT` 覆盖后的目录存在性，以及 `TESTDATA_ROOT`。
