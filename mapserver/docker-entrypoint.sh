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

exec "$@"
