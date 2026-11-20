# 按地区选择 .env 预设

## 为什么必须显式声明地区

`gis-encoding` 修复「传统编码字节被误读」的 GIS 路径时，需要知道该按哪些编码
候选去试（GBK / BIG5 / EUC-KR / Shift_JIS / CP932）。Windows 上原本可用系统
ANSI 代码页 `GetACP()` 猜，但这条路不能跨平台：

- **Linux / macOS 没有等价接口**。`nl_langinfo(CODESET)` 与 `LANG` / `LC_ALL` /
  `LC_CTYPE` 返回的是**当前进程 locale 的字符集**，现代系统几乎恒为 UTF-8；
  它不描述磁盘上路径字节原本用的传统编码。Unix 路径在内核里就是不解释的字节串。
- **实测反例**：同一台中文 Windows 上 `GetACP() = 936`（GBK），而 Git Bash
  注入 `LANG=en_US.UTF-8`、`LC_ALL=C.UTF-8`。按 POSIX locale 猜会判成英文区，
  GBK 乱码路径反而不被修复。
- 服务进程（systemd / Docker / Windows 服务）的 locale 由启动方决定，
  与数据来源地区无关；同一台机器上也可能混有不同地区的历史数据。

因此统一改为**由 `.env` 显式声明地区**，跨平台行为一致。`GetACP()` 仅作为
可选的 Windows 兼容兜底，需要显式打开 `win-acp` feature 才编译进去，默认关闭。

## 五份预设

| 文件 | `GIS_ENCODING_REGION` | 探测的传统编码 | 典型场景 |
| :--- | :--- | :--- | :--- |
| `.env.zh-CN` | `zh-CN` | GBK | 简体中文（默认） |
| `.env.zh-TW` | `zh-TW` | BIG5 + EUC-TW | 繁体中文 |
| `.env.en` | `en` | 无（只认 UTF-8 / ASCII） | 英文、西文 |
| `.env.ko` | `ko` | EUC-KR | 韩语 |
| `.env.ja` | `ja` | Shift_JIS / CP932 | 日语 |

用法：选一份复制为 `.env`（Windows 安装包内位于 `env/` 目录），再按实际环境
改数据库口令等公共项。

```sh
cp config/env/.env.zh-TW .env
```

Windows：

```bat
copy env\.env.zh-TW .env
```

## 优先级（`Region::from_env`）

1. `GIS_ENCODING_REGION` —— 推荐，取值见上表；
2. `GIS_ENCODING_LOCALE` —— 旧名，保留兼容，也接受 `zh_CN.UTF-8`、
   `Japanese_Japan.932` 等 POSIX / Windows locale 写法；
3. `win-acp` feature 开启时的 Windows `GetACP()`；
4. 兜底 `zh-CN`（保持既有部署默认）。

**不再读取** `LANG` / `LC_ALL` / `LC_CTYPE`。若需要严格校验取值，用
`Region::parse()`（未知值返回 `None`），`Region::as_str()` 返回规范化取值。

## 维护约定

这 5 份模板**只有 `GIS_ENCODING_REGION` 一行与文件头注释不同**，其余公共项
完全一致；公共项的真源是安装脚本 `build/script/install.bat` 生成的 `.env.example`。
改公共项时请同步 5 份模板与 `mapserver/install/windows/install.bat`。

公共项分两档（判据与逐项说明见 `build/script/README.md` §3.3）：

- **必需**：`MANAGER_DB_PASSWORD`、`MANAGER_JWT_SECRET`、`MAPSERVER_DB_PASSWORD`
  —— 配置 XML 里写成 `${VAR}` 无默认值，不设就展开成空串。
- **可选**：`MANAGER_DB_HOST` / `MANAGER_DB_PORT`、`MAPSERVER_DB_HOST` /
  `MAPSERVER_DB_PORT`、`MAPSRV_HOST` / `MAPSRV_PORT`、`MAPSERVER_MANAGER_URL`、
  `DATA_ROOT`、`TESTDATA_ROOT`、`SHPPGSQL_BIN`。其中 `DATA_ROOT` 的占位写成
  `${DATA_ROOT:-data}`，**不设即取 `data`**——启动工作目录下的 `data\` 子目录
  （安装脚本已创建，源码树有 `.gitkeep` 占位）；只有数据在外置盘时才需要设。

`MAPSERVER_PORT`、`MAPSERVER_DATA_DIR`、`MAPSERVER_CACHE_DIR`、`API_KEY` 已失效、
代码不再读取：端口取 `config/mapserver.xml` 的 `<server port>`，数据根默认 `data\`
且可用 `DATA_ROOT` 覆盖。

`start-mapserver.bat` / `start-mapapi.bat` / `start-manager.bat` 都会从 `.env`
加载变量，地区值因此对 map-api 与 manager 同时生效。Docker 部署用
`-e GIS_ENCODING_REGION=...` 注入即可。
