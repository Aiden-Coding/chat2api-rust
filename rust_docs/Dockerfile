# -------------------------------------------------------------
# 阶段 1: 依赖项预编译与业务代码构建阶段 (Builder Stage)
# -------------------------------------------------------------
FROM rust:latest AS builder

WORKDIR /usr/src/chat2api

# 安装编译所需的 C 工具链、CMake 和 Perl (boring-sys 编译所必需)
RUN apt-get update && \
    apt-get install -y --no-install-recommends cmake clang perl make && \
    rm -rf /var/lib/apt/lists/*

# 1. 拷贝 Cargo 依赖清单文件
COPY Cargo.toml Cargo.lock ./

# 2. 创建骨架 main.rs，预先拉取并编译所有依赖库
# 这一步是为了最大化利用 Docker 缓存层（Docker Layer Cache）。
# 只要 Cargo.toml 没有发生改变，重复构建时就不需要重新下载和编译三方依赖。
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# 3. 拷贝真实的 Rust 源代码、HTML 网页模板和版本信息文件
COPY src ./src
COPY templates ./templates
COPY version.txt ./

# 4. 再次触发编译。因为依赖已经编译完成并被 Docker 缓存，这一步只会编译我们自己的业务代码，仅需数秒。
RUN touch src/main.rs && cargo build --release

# -------------------------------------------------------------
# 阶段 2: 极小运行阶段 (Runner Stage)
# -------------------------------------------------------------
FROM debian:bookworm-slim

WORKDIR /app

# 1. 安装系统所必须的 SSL/TLS 根证书包 (ca-certificates)
# 这一步非常关键，因为逆向问答涉及向 OpenAI Auth0 等外部 HTTPS 发送网络包，若无根证书将报错。
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# 2. 从 Builder 编译阶段拷贝精简过后的二进制可执行程序以及静态模板
COPY --from=builder /usr/src/chat2api/target/release/chat2api /app/chat2api
COPY --from=builder /usr/src/chat2api/templates /app/templates
COPY --from=builder /usr/src/chat2api/version.txt /app/version.txt

# 3. 暴露运行时需要被挂载的数据存储卷
VOLUME ["/app/data"]

EXPOSE 5005

# 设定默认运行时容器环境变量
ENV PORT=5005
ENV HISTORY_DISABLED=true
ENV RETRY_TIMES=3
ENV ENABLE_LIMIT=true

# 启动程序
CMD ["./chat2api"]
