# syntax=docker/dockerfile:1
# ============================================================================
# r-manager — 管理平面镜像（开发计划 P0-1 一键部署）
#
# 三阶段：
#   1) ui       node:22-slim  pnpm 构建前端 → /ui/dist
#   2) api      rust:1-slim-book 编译后端 manager（reqwest 关 TLS、
#               tokio-postgres 无 TLS、jsonwebtoken 用 ring → 仅 C 工具链，
#               无需 OpenSSL）
#   3) runtime  debian:bookworm-slim + ca-certificates + gettext-base
#               前端 dist 烘进镜像（后端同端口托管静态资源），配置不烘焙：
#               运行期挂载 /app/config.template 由 entrypoint envsubst 渲染。
#
# 后端按进程工作目录解析相对路径；本镜像把 static_dir/avatar 用绝对路径写入
# 挂载配置，WORKDIR 固定 /app，避免路径歧义。
# ============================================================================

# ---- 1) frontend -----------------------------------------------------------
FROM node:22-slim AS ui
WORKDIR /ui
RUN npm install -g pnpm@9
COPY ui/package.json ui/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
RUN pnpm run build:prod

# ---- 2) backend ------------------------------------------------------------
FROM rust:1-slim-book AS api
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY backend ./backend
RUN cargo build --release --manifest-path /src/backend/Cargo.toml

# ---- 3) runtime ------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates gettext-base \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=api /src/backend/target/release/manager /app/manager
COPY --from=ui /ui/dist /app/ui/dist
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN sed -i 's/\r$//' /usr/local/bin/docker-entrypoint.sh \
 && chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 6180

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["--config", "/app/config/manager.xml"]
