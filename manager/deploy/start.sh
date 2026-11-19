#!/usr/bin/env bash
# ============================================================================
# 一键启动 + 自检 —— deploy/start.sh（Linux 宿主机）
#
# 读平台目录的 .env → docker compose up -d（默认 --build）→ 等 PostGIS healthy
# → 调用 validate.sh。功能与 Windows 版 start.ps1 一一对应。
#
# Usage:
#   bash ../../deploy/start.sh --dir .                    # 在平台目录内执行
#   bash deploy/start.sh --dir deploy_zh-cn/linux         # 从 manager/ 执行
#   bash deploy/start.sh --dir deploy_en/linux --no-build
#   bash deploy/start.sh --dir deploy_zh-cn/linux --skip-validate
# ============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NO_BUILD=0
SKIP_VALIDATE=0
DIR_ARG=""

usage() {
    sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-build)      NO_BUILD=1; shift ;;
        --skip-validate) SKIP_VALIDATE=1; shift ;;
        --dir|-d)        DIR_ARG="${2:-}"; shift 2 ;;
        -h|--help)       usage; exit 0 ;;
        *) echo "未知参数: $1" >&2; usage >&2; exit 2 ;;
    esac
done

# 平台目录：--dir 相对脚本所在目录（deploy/）解析；省略则默认 deploy/ 自身。
if [[ -z "$DIR_ARG" ]]; then
    COMPOSE_DIR="$SCRIPT_DIR"
elif [[ "$DIR_ARG" = /* ]]; then
    COMPOSE_DIR="$DIR_ARG"
else
    COMPOSE_DIR="$SCRIPT_DIR/$DIR_ARG"
fi
COMPOSE_DIR="$(cd "$COMPOSE_DIR" 2>/dev/null && pwd)" || {
    echo "目录不存在: $COMPOSE_DIR" >&2; exit 2;
}
if [[ ! -f "$COMPOSE_DIR/docker-compose.yml" ]]; then
    echo "未找到 docker-compose.yml: $COMPOSE_DIR（--dir 应指向 deploy_zh-cn/linux 等平台目录）" >&2
    exit 2
fi

# 加载 .env（字面量解析，不做变量展开，与 PowerShell 版口径一致）
ENV_FILE="$COMPOSE_DIR/.env"
if [[ ! -f "$ENV_FILE" ]]; then
    echo "缺少 .env：请在 $COMPOSE_DIR 下 cp ../../deploy/.env.example .env 并填写 MANAGER_JWT_SECRET" >&2
    exit 2
fi
while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    if [[ "$line" =~ ^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]]; then
        export "${BASH_REMATCH[1]}=${BASH_REMATCH[2]}"
    fi
done < "$ENV_FILE"

cd "$COMPOSE_DIR"

# 启动
UP_ARGS=(compose up -d)
[[ "$NO_BUILD" -eq 0 ]] && UP_ARGS+=(--build)
echo "docker ${UP_ARGS[*]}"
docker "${UP_ARGS[@]}"

# 等 PostGIS healthy（最多 3 分钟）
echo "等待 PostGIS 健康…"
deadline=$(( $(date +%s) + 180 ))
healthy=0
while (( $(date +%s) < deadline )); do
    sleep 3
    ps_out="$(docker compose ps --format '{{.Name}} {{.State}} {{.Health}}' 2>/dev/null || true)"
    if grep -Eq 'gis-postgis.*running.*healthy' <<<"$ps_out"; then
        healthy=1; break
    fi
done
if [[ "$healthy" -eq 0 ]]; then
    echo "警告：PostGIS 未在 3 分钟内转 healthy，继续尝试自检" >&2
fi

# 自检
if [[ "$SKIP_VALIDATE" -eq 0 ]]; then
    bash "$SCRIPT_DIR/validate.sh" \
        --db-port "${MAPSERVER_DB_PORT:-5434}" \
        --dir "$COMPOSE_DIR" \
        --check-data-root
fi

echo
echo "前端    : http://localhost:6180/"
echo "map-api : http://localhost:8080/health"
echo "停止    : docker compose down   （删卷加 -v）"
