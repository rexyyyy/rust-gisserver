# Windows 安装指南

## 系统要求

- Windows 10/11 (64-bit)
- Rust 工具链 (rustup)
- MSVC 2019/2022 (Visual Studio Build Tools)
- 可选: Node.js + pnpm (前端构建)

## 快速安装

### 1. 构建

```cmd
cd install\windows
build_all.bat
```

如需同时构建前端:
```cmd
build_all.bat frontend
```

### 2. 安装

```cmd
install.bat [安装目录]
```

默认安装到 `..\..\..\..\rmap_target`

### 4. 配置

```cmd
cd [安装目录]
copy .env.example .env
notepad .env
```

编辑 `.env` 配置数据库连接等参数：
- `MANAGER_DB_PASSWORD` - 数据库密码
- `MANAGER_JWT_SECRET` - JWT 密钥
- `MANAGER_DB_HOST/PORT` - 数据库地址

### 5. 启动

```cmd
start-mapserver.bat   # 端口 8080
start-manager.bat     # 端口 6180
```

## 目录结构

```
rmap_target/
├── bin/
│   ├── map-api.exe      # rust-gisserver 主程序
│   ├── rust-gisserver.exe    # rust-gisserver 管理后台
│   └── proj_9.dll       # PROJ 坐标转换库
├── config/              # 配置文件
├── data/                # 数据目录 (TPK, MBTiles 等)
├── cache/               # 响应缓存
├── logs/                # 日志
├── share/proj/          # PROJ 数据文件
├── web/                 # 前端静态文件
├── .env.example         # 配置模板
├── start-mapserver.bat  # 启动脚本
└── start-manager.bat    # 启动脚本
```

## 数据库配置

### PostgreSQL

```env
DATABASE_URL=postgresql://user:password@localhost:5432/rmap
```

需要创建数据库并运行迁移:
```sql
CREATE DATABASE rmap;
```

### 文件模式 (无需数据库)

rust-gisserver 支持 file-sidecar 模式，无需数据库即可运行。

## 注册为 Windows 服务

使用 NSSM (Non-Sucking Service Manager):

```cmd
nssm install rust-gisserver "[安装目录]\bin\map-api.exe"
nssm install rust-gisserver "[安装目录]\bin\rust-gisserver.exe"

nssm start rust-gisserver
nssm start rust-gisserver
```

## 卸载

```cmd
uninstall.bat [安装目录]
```

## 故障排除

### PROJ.9 加载失败

确保 `proj_9.dll` 和 `share/proj/` 目录存在:
```
bin\proj_9.dll
share\proj\*
```

### 端口被占用

修改 `.env` 中的端口:
```env
MAPSERVER_PORT=8081
MANAGER_PORT=6181
```

### 前端 404

确保构建了前端:
```cmd
build_all.bat frontend
```

## 性能优化

- 使用 SSD 存储数据和缓存
- 增加缓存大小 (默认 512MB)
- 使用 PostgreSQL 模式 (比文件模式快 3-5x)

## 安全建议

- 修改默认 API Key
- 使用 HTTPS (反向代理)
- 限制数据库权限
- 定期备份数据
