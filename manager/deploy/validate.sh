#!/usr/bin/env bash
# ============================================================================
# 跨服务连通性自检 —— deploy/validate.sh（Linux 宿主机）
#
# 存活 / 功能 / gisdata 挂载点检查；任一 FAIL → 退出码 1（供 start.sh / CI 使用）。
# 与 Windows 版 validate.ps1 检查项一一对应，HTTP 探测用 curl。
#
# Usage:
#   bash deploy/validate.sh --dir deploy_zh-cn/linux --check-data-root
#   bash deploy/validate.sh --map-api-base http://host:8080 --db-port 5432
#   bash deploy/validate.sh --admin-token 'xxx'
# ============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

MAP_API_BASE='http://localhost:8080'
MANAGER_BASE='http://localhost:6180'
DB_HOST='localhost'
DB_PORT=5432
ADMIN_TOKEN=''
DIR_ARG=''
CHECK_DATA_ROOT=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --map-api-base)   MAP_API_BASE="$2"; shift 2 ;;
        --manager-base)   MANAGER_BASE="$2"; shift 2 ;;
        --db-host)        DB_HOST="$2"; shift 2 ;;
        --db-port)        DB_PORT="$2"; shift 2 ;;
        --admin-token)    ADMIN_TOKEN="$2"; shift 2 ;;
        --dir)            DIR_ARG="$2"; shift 2 ;;
        --check-data-root) CHECK_DATA_ROOT=1; shift ;;
        -h|--help)        sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "未知参数: $1" >&2; exit 2 ;;
    esac
done

fail_count=0
add() {  # add NAME STATUS DETAIL
    printf '  %-32s %-6s %s\n' "$1" "$2" "$3"
    [[ "$2" == 'FAIL' ]] && fail_count=$((fail_count + 1))
}

# HTTP 探测：任意 HTTP 状态即视为服务存活；连接失败 = FAIL
probe() {  # probe NAME URL -> 写入全局变量 $RESP_BODY / $RESP_CODE
    local name="$1" url="$2"
    local args=( -s -o /dev/null -w '%{http_code}' --max-time 10 )
    [[ -n "$ADMIN_TOKEN" ]] && args+=( -H "X-Admin-Token: $ADMIN_TOKEN" )
    RESP_CODE="$(curl "${args[@]}" "$url" 2>/dev/null)" || RESP_CODE=''
    if [[ -n "$RESP_CODE" && "$RESP_CODE" != '000' ]]; then
        add "$name" 'PASS' "HTTP $RESP_CODE"
        RESP_BODY="$(curl -s --max-time 10 ${ADMIN_TOKEN:+-H "X-Admin-Token: $ADMIN_TOKEN"} "$url" 2>/dev/null)"
        return 0
    else
        add "$name" 'FAIL' '连接失败或超时'
        RESP_BODY=''
        return 1
    fi
}

echo "=== rust-gisserver 部署自检 ==="

# 1) PostGIS 端口可达性（bash /dev/tcp，无需 nc）
if (echo > "/dev/tcp/$DB_HOST/$DB_PORT") >/dev/null 2>&1; then
    add 'PostGIS TCP' 'PASS' "$DB_HOST:$DB_PORT"
else
    add 'PostGIS TCP' 'FAIL' "$DB_HOST:$DB_PORT"
fi

# 2) map-api /health
probe 'map-api /health' "$MAP_API_BASE/health" || true

# 2b) PROJ 后端（/health 的 proj 字段）：跨 CRS 重投影（EPSG:4490 / 高斯-克吕格
# 45xx / UTM）依赖 PROJ 库 + proj.db。proj=false 不阻断启动，但跨 CRS 请求会
# 失败，故报 WARN 提醒。真正的可用性判据是 proj_create_crs_to_crs 能否成功。
health_body="$(curl -s --max-time 10 "$MAP_API_BASE/health" 2>/dev/null)"
if grep -q '"proj"[[:space:]]*:[[:space:]]*true' <<<"$health_body"; then
    add 'PROJ 后端' 'PASS' 'proj_loaded=true（跨 CRS 重投影可用）'
elif grep -q '"proj"' <<<"$health_body"; then
    add 'PROJ 后端' 'WARN' 'proj_loaded=false：容器内缺 PROJ 库或 proj.db，跨 CRS 请求会失败'
else
    add 'PROJ 后端' 'INFO' '未能从 /health 读取 proj 字段'
fi

# 3) WMS GetCapabilities
if probe 'map-api WMS GetCapabilities' \
        "$MAP_API_BASE/wms?service=WMS&version=1.3.0&request=GetCapabilities"; then
    if grep -Eq 'WMS_Capabilities|ServiceMetadata' <<<"$RESP_BODY"; then
        # 覆写上一行 DETAIL：简单起见追加一行 INFO
        add '  └ capabilities 内容' 'INFO' '正常'
    fi
fi

# 4) /admin/components（admin 平面 + data_root 展示）
if probe 'map-api /admin/components' "$MAP_API_BASE/admin/components"; then
    dr="$(sed -n 's/.*"data_root"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' <<<"$RESP_BODY" | head -1)"
    if [[ -n "$dr" ]]; then
        add 'data_root 配置' 'INFO' "$dr"
    fi
fi

# 5) manager 根
probe 'manager /' "$MANAGER_BASE/" || true

# 6) gisdata 挂载点存在性（可选）
if [[ "$CHECK_DATA_ROOT" -eq 1 ]]; then
    if [[ -n "$DIR_ARG" ]]; then
        deploy_dir="$DIR_ARG"
    else
        deploy_dir="$SCRIPT_DIR"
    fi
    dr="${GISDATA_DIR:-$deploy_dir/gisdata}"
    if [[ -d "$dr" ]]; then
        add 'gisdata 目录' 'PASS' "$dr"
    else
        add 'gisdata 目录' 'WARN' "$dr"
    fi
    if [[ -z "${TESTDATA_ROOT:-}" ]]; then
        add 'TESTDATA_ROOT 目录' 'INFO' 'env 未设置（e2e demo 图层跳过，非致命）'
    elif [[ -d "$TESTDATA_ROOT" ]]; then
        add 'TESTDATA_ROOT 目录' 'PASS' "$TESTDATA_ROOT"
    else
        add 'TESTDATA_ROOT 目录' 'WARN' "$TESTDATA_ROOT"
    fi
fi

echo
if [[ "$fail_count" -gt 0 ]]; then
    echo "存在 $fail_count 项 FAIL"
    exit 1
else
    echo '全部关键检查通过'
    exit 0
fi
