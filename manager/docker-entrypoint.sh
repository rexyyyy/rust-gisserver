#!/bin/sh
# ============================================================================
# rust-gisserver 容器入口（P0-1 + P0-3）
#
# 把挂载进来的配置模板 /app/config.template/*.xml 用 envsubst 渲染到
# /app/config：DB 口令 / JWT 密钥 / map-api 主机名经环境变量注入，
# 镜像与仓库都不含明文密钥、不烘焙配置。渲染后 exec 真正的 manager 命令。
#
# 可用环境变量：
#   MANAGER_CONFIG_TEMPLATE  模板目录（默认 /app/config.template）
#   MANAGER_CONFIG_DIR       渲染输出目录（默认 /app/config）
# ============================================================================
set -e

TMPL_DIR="${MANAGER_CONFIG_TEMPLATE:-/app/config.template}"
OUT_DIR="${MANAGER_CONFIG_DIR:-/app/config}"

if [ -d "$TMPL_DIR" ]; then
  mkdir -p "$OUT_DIR"
  for f in "$TMPL_DIR"/*.xml; do
    [ -e "$f" ] || continue
    envsubst < "$f" > "$OUT_DIR/$(basename "$f")"
  done
  echo "[entrypoint] rendered config: $TMPL_DIR -> $OUT_DIR"
fi

exec "$@"
