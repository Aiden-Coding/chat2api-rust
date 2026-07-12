# Rust 重构版本部署文档

本文档介绍如何在 Linux 服务器中通过系统级守护、Docker 容器以及反向代理等方案部署 Rust 重构版本。

---

## 1. 原生二进制编译与本地部署

直接在物理机或 VPS 虚拟机上部署能够获得极致的响应性能。

### 1.1 编译与运行

需要服务器上安装有 Cargo 环境。

```bash
# 进入根目录编译 Release 二进制文件
cargo build --release

# 编译完毕后，拷贝二进制包到运行环境
cp ./target/release/chat2api /usr/local/bin/
```

### 1.2 systemd 系统服务托管 (推荐)

通过 systemd 实现故障自动重启、开机自启与环境变量隔离。

1. 编辑配置文件 `/etc/systemd/system/chat2api.service`：

    ```ini
    [Unit]
    Description=Chat2API Rust High Performance Service
    After=network.target

    [Service]
    Type=simple
    User=root
    WorkingDirectory=/opt/chat2api
    ExecStart=/usr/local/bin/chat2api
    Restart=always
    RestartSec=3

    # 配置环境变量
    Environment=PORT=5005
    Environment=HISTORY_DISABLED=true
    Environment=RETRY_TIMES=3
    Environment=ENABLE_LIMIT=true
    # Environment=PROXY_URL=http://your_proxy_ip:port

    [Install]
    WantedBy=multi-user.target
    ```

2. 载入并启动：

    ```bash
    systemctl daemon-reload
    systemctl enable chat2api
    systemctl start chat2api

    # 查阅实时运行日志
    journalctl -u chat2api -f -n 50
    ```

---

## 2. Docker 与 Docker Compose 部署

### 2.1 编写本地的 Dockerfile

在项目根目录中已提供 `Dockerfile`。您可以通过以下命令在本地进行镜像构建：

```bash
docker build -t chat2api-rust:latest .
```

### 2.2 运行容器

```bash
docker run -d \
  --name chat2api-rust \
  -p 5005:5005 \
  -v $(pwd)/data:/app/data \
  -e PORT=5005 \
  -e HISTORY_DISABLED=true \
  -e RETRY_TIMES=3 \
  -e ENABLE_LIMIT=true \
  --restart always \
  chat2api-rust:latest
```

### 2.3 Docker Compose 联动 Cloudflare WARP 代理部署

针对服务器 IP 经常被 OpenAI 拦截 403 的情况，推荐配合 Cloudflare WARP 容器作为本地 SOCKS5 代理。

创建 `docker-compose.yml`：

```yaml
version: '3.8'

services:
  # WARP 客户端容器：将本地流量中转至 Cloudflare 干净的网络节点，输出 socks5 代理
  socks5-warp:
    image: caomingjun/warp-go:latest
    container_name: socks5-warp
    restart: always
    ports:
      - "1080:1080"
    environment:
      - WARP_LICENSE= # 若有 plus 激活码可填

  # Chat2API Rust 容器
  chat2api-rust:
    image: chat2api-rust:latest
    container_name: chat2api-rust
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "5005:5005"
    volumes:
      - ./data:/app/data
    environment:
      - PORT=5005
      - HISTORY_DISABLED=true
      - RETRY_TIMES=3
      - ENABLE_LIMIT=true
      # 将网络请求全部代理到 socks5-warp 容器的 1080 端口上，防 403 封锁
      - PROXY_URL=socks5://socks5-warp:1080
    depends_on:
      - socks5-warp
    restart: always
```

一键拉起部署：

```bash
docker compose up -d
```

---

## 3. Nginx 反向代理配置 (Event-Stream SSE 调优)

在通过 Nginx 暴露前端 HTTPS 服务时，由于流式问答是基于 `text/event-stream` 协议持续响应的，若 Nginx 默认启用了缓冲机制，会导致消息直到全部回答完毕才一次性输出。

必须在 `location` 段内配置 `proxy_buffering off`。

```nginx
server {
    listen 443 ssl;
    server_name yourdomain.com;

    ssl_certificate /etc/nginx/certs/yourdomain.crt;
    ssl_certificate_key /etc/nginx/certs/yourdomain.key;

    location / {
        proxy_pass http://127.0.0.1:5005;
        
        # 1. 传递真实 IP
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # 2. 禁用 Nginx 本地缓冲区缓存，实现实时的打字机 SSE 推送
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        chunked_transfer_encoding on;

        # 3. 延长网关读写超时保护
        proxy_read_timeout 600s;
        proxy_send_timeout 600s;
    }
}
```
