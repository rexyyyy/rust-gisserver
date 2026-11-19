#!/bin/sh
# ============================================================================
# rust-gisserver / map-api 容器入口（P0-1 + P0-3）
#
# 把挂载进来的配置模板 /app/config.template/*.xml 用 envsubst 渲染到
# /app/config，DB 口令 / 服务主机名等敏感值经环境变量注入 —— 镜像与仓库
# 都不含明文密钥，也不烘焙任何配置。渲染后即 exec 真正的 map-api 命令。
#
# 可用环境变量：
#   MAPSERVER_CONFIG_TEMPLATE  模板目录（默认 /app/config.template）
#   MAPSERVER_CONFIG_DIR       渲染输出目录（默认 /app/config）
#   GIS_ENCODING_REGION        路径编码地区：zh-CN / zh-TW / en / ko / ja
#                              （容器 locale 恒为 UTF-8，不能据此推断路径编码）
#   PROJ_DLL / PROJ_DATA       PROJ 库与 proj.db 位置（镜像内已设默认值）
# ============================================================================
set -e

TMPL_DIR="${MAPSERVER_CONFIG_TEMPLATE:-/app/config.template}"
OUT_DIR="${MAPSERVER_CONFIG_DIR:-/app/config}"

if [ -d "$TMPL_DIR" ]; then
  mkdir -p "$OUT_DIR"
  for f in "$TMPL_DIR"/*.xml; do
    [ -e "$f" ] || continue
    envsubst < "$f" > "$OUT_DIR/$(basename "$f")"
  done
  echo "[entrypoint] rendered config: $TMPL_DIR -> $OUT_DIR"
fi

# PROJ_LIB 是 PROJ 9.1 之前的旧变量名，常被系统 PostGIS 的旧版 proj.db 占用
# （LAYOUT.MINOR 不匹配时 PROJ 会直接报错中止）。这里显式清掉，确保只用
# 本镜像 proj-data 提供的 proj.db；PROJ_DATA 优先级本就高于 PROJ_LIB。
unset PROJ_LIB

# PROJ 自检：缺失不阻断启动（内置 4326/3857 与同 CRS 恒等变换仍可用），但要在
# 日志里显式暴露，否则"跨 CRS 互转 500"这类问题在运维侧很难第一时间归因。
# 判据是库文件 + proj.db 都在位；服务自身的 /health 会进一步报 proj_loaded。
if [ -n "${PROJ_DLL:-}" ] && [ -f "${PROJ_DLL}" ]; then
  echo "[entrypoint] PROJ lib: $PROJ_DLL"
else
  echo "[entrypoint] PROJ lib NOT found (PROJ_DLL=${PROJ_DLL:-unset})" >&2
fi
if [ -n "${PROJ_DATA:-}" ] && [ -f "${PROJ_DATA}/proj.db" ]; then
  echo "[entrypoint] PROJ data: $PROJ_DATA/proj.db"
else
  echo "[entrypoint] PROJ proj.db NOT found (PROJ_DATA=${PROJ_DATA:-unset})" >&2
fi

exec "$@"
