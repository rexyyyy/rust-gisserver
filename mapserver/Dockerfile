# syntax=docker/dockerfile:1
# ============================================================================
# mapserver / map-api — 发布服务平面镜像（开发计划 P0-1 一键部署）
#
# 多阶段：
#   1) builder  debian:bookworm-slim + rustup 装 Rust 工具链（USTC 镜像）。
#      不用 rust:1-slim-book：国内 mirror 对该镜像 manifest 返回 DENIED，
#      而 debian 可正常拉取；工具链版本钉在 1.98.1（与本机开发一致）。
#      （rusqlite 以 bundled 方式从源码编 libsqlite3，需 C 工具链；
#        ureq 走 rustls、tokio-postgres 不启用 TLS，故无需 OpenSSL）。
#   2) runtime  debian:bookworm-slim，带 ca-certificates + gettext-base
#      （envsubst），配合 docker-entrypoint.sh 把挂载的配置模板渲染成
#      运行期 mapserver.xml —— DB 口令等经环境变量注入，配置本体不入库、
#      也不烘焙进镜像（守 mapserver「纯发布服务平面」红线）。
#
# 镜像内不含任何 config/data：启动时以卷挂载
#   /app/config.template  配置模板（*.xml，含 ${VAR} 占位，只读）
#   /app/config           entrypoint 渲染产物（可写临时层）
#   /app/data             图层数据根（COG/GeoJSON/归档等）
#   /app/cache            foyer 磁盘缓存（需可写）
# ============================================================================

# ---- build stage -----------------------------------------------------------
FROM debian:bookworm-slim AS builder

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static \
    RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config curl ca-certificates \
 && curl -fsSLO "$RUSTUP_UPDATE_ROOT/dist/x86_64-unknown-linux-gnu/rustup-init" \
 && chmod +x rustup-init \
 && ./rustup-init -y --profile minimal --default-toolchain 1.98.1 --no-modify-path \
 && rm rustup-init \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# 本仓布局：工作区在 mapserver/（成员 services/*），核心库是 path 依赖
# ../gisserver —— 故 compose 以仓库根为 build 上下文，把两个目录一起送进
# daemon；容器内保持 gisserver 与 mapserver 同级，依赖路径原样可解析。
# 先复制清单与核心库，源码放最后，尽量利用依赖编译缓存。
# 注：Cargo.lock 不入库（全新 clone 时不存在），cargo 自行解析依赖。
COPY mapserver/Cargo.toml ./mapserver/
COPY gisserver ./gisserver
COPY mapserver/services ./mapserver/services

RUN cargo build --release --manifest-path ./mapserver/Cargo.toml -p map-api

# ---- runtime stage ---------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates gettext-base \
      libproj25 proj-data \
 && rm -rf /var/lib/apt/lists/*

# PROJ 由运行期 dlopen 加载（不参与链接），库名按平台不同：
#   Windows proj_9.dll / Unix libproj.so。
# Debian 只装出 libproj.so.25（裸名软链 libproj.so 在 -dev 包里），而代码按裸名
# "libproj.so" 查找。这里建固定路径的软链并显式给出 PROJ_DLL —— 绝对路径 dlopen
# 不依赖 ld.so 搜索路径，也不受架构 triplet（x86_64-linux-gnu / aarch64-linux-gnu）
# 影响。PROJ_DATA 指向 proj-data 的安装目录（PROJ 9.1+ 的正式变量名）。
# proj.db 由 proj-data 提供，与 libproj25 同源同版本（Debian bookworm 9.1.1），
# 不会出现 Windows 侧那种 LAYOUT.MINOR 版本错配。
RUN LIBPROJ="$(dpkg -L libproj25 | grep -E '/libproj\.so\.[0-9]+\.[0-9.]+$' | head -1)" \
 && test -n "$LIBPROJ" \
 && mkdir -p /usr/local/lib \
 && ln -sf "$LIBPROJ" /usr/local/lib/libproj.so \
 && ldconfig \
 && test -f /usr/share/proj/proj.db

ENV PROJ_DLL=/usr/local/lib/libproj.so \
    PROJ_DATA=/usr/share/proj

WORKDIR /app

COPY --from=builder /src/mapserver/target/release/map-api /app/map-api
COPY mapserver/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
# 归一化行尾（防 Windows CRLF 破坏 shebang）并赋可执行位。
RUN sed -i 's/\r$//' /usr/local/bin/docker-entrypoint.sh \
 && chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 8080

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["serve", "--config", "/app/config/mapserver.xml"]
