# syntax=docker/dockerfile:1
# ============================================================================
# manager — 管理平面镜像（开发计划 P0-1 一键部署）
#
# 三阶段：
#   1) ui       node:22-slim  pnpm 构建前端 → /ui/dist
#   2) api      debian:bookworm-slim + rustup 装 Rust 工具链（USTC 镜像，
#               钉 1.98.1 与本机一致；不用 rust:1-slim-book——国内 mirror
#               对其 manifest 返回 DENIED）编译后端 manager（reqwest 关 TLS、
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
# pnpm/npm mirror — USTC 国内源比默认 registry.npmjs.org 快得多。
RUN npm install -g pnpm@12 \
 && pnpm config set registry https://registry.npmmirror.com
COPY ui/package.json ui/pnpm-lock.yaml ui/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
RUN pnpm run build:prod

# ---- 2) backend ------------------------------------------------------------
FROM debian:bookworm-slim AS api

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_DIST_SERVER=https://rsproxy.cn \
    RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup \
    PATH=/usr/local/cargo/bin:$PATH

RUN sed -i 's|deb.debian.org|mirrors.ustc.edu.cn|g; s|security.debian.org|mirrors.ustc.edu.cn|g' /etc/apt/sources.list.d/debian.sources \
 && apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config curl ca-certificates \
 && curl -fsSLO "$RUSTUP_UPDATE_ROOT/dist/x86_64-unknown-linux-gnu/rustup-init" \
 && chmod +x rustup-init \
 && ./rustup-init -y --profile minimal --default-toolchain 1.98.1 --no-modify-path \
 && rm rustup-init \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p "$CARGO_HOME"/.cargo \
 && printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n\n[source.rsproxy]\nregistry = "https://rsproxy.cn/crates.io-index"\n\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n' > "$CARGO_HOME"/.cargo/config.toml
WORKDIR /src
COPY backend ./backend
RUN cargo build --release --manifest-path /src/backend/Cargo.toml

# ---- 3) runtime ------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN sed -i 's|deb.debian.org|mirrors.ustc.edu.cn|g; s|security.debian.org|mirrors.ustc.edu.cn|g' /etc/apt/sources.list.d/debian.sources \
 && apt-get update \
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
CMD ["/app/manager", "--config", "/app/config/manager.xml"]
