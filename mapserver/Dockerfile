# syntax=docker/dockerfile:1
# ============================================================================
# r-mapserver / map-api — 发布服务平面镜像（开发计划 P0-1 一键部署）
#
# 多阶段：
#   1) builder  rust:1-slim-book，装 build-essential + pkg-config
#      （rusqlite 以 bundled 方式从源码编 libsqlite3，需 C 工具链；
#        ureq 走 rustls、tokio-postgres 不启用 TLS，故无需 OpenSSL）。
#   2) runtime  debian:bookworm-slim，带 ca-certificates + gettext-base
#      （envsubst），配合 docker-entrypoint.sh 把挂载的配置模板渲染成
#      运行期 mapserver.xml —— DB 口令等经环境变量注入，配置本体不入库、
#      也不烘焙进镜像（守 r-mapserver「纯发布服务平面」红线）。
#
# 镜像内不含任何 config/data：启动时以卷挂载
#   /app/config.template  配置模板（*.xml，含 ${VAR} 占位，只读）
#   /app/config           entrypoint 渲染产物（可写临时层）
#   /app/data             图层数据根（COG/GeoJSON/归档等）
#   /app/cache            foyer 磁盘缓存（需可写）
# ============================================================================

# ---- build stage -----------------------------------------------------------
FROM rust:1-slim-book AS builder

RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# 先复制清单与源码目录，尽量利用依赖编译缓存。
# 注：/Cargo.lock 被 .gitignore 忽略，全新 clone 时不存在，cargo 自行解析。
COPY Cargo.toml ./
COPY crates ./crates
COPY services ./services

RUN cargo build --release -p map-api

# ---- runtime stage ---------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates gettext-base \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /src/target/release/map-api /app/map-api
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
# 归一化行尾（防 Windows CRLF 破坏 shebang）并赋可执行位。
RUN sed -i 's/\r$//' /usr/local/bin/docker-entrypoint.sh \
 && chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 8080

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["serve", "--config", "/app/config/mapserver.xml"]
