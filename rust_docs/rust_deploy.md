# Rust 版本部署指南

本文档将详细介绍如何在各类服务器生产环境中部署重构后的 Rust 高性能版本。

---

## 方式一：本地二进制部署

如果您直接在物理机或云服务器上部署，可以将其编译为原生二进制文件并使用守护进程（如 `systemd` 或 `pm2`）进行托管。

### 1. 编译

在具有 Rust 工具链的环境中执行编译命令：

```bash
cargo build --release
```

编译成功后，生成的优化二进制文件位于 `./target/release/chat2api`。您可以将其复制到服务器的目标运行路径。

### 2. 使用 pm2 进行托管守护

如果服务器安装了 Node.js 环境，可以使用 `pm2` 非常方便地管理服务：

```bash
# 启动并命名服务，通过 env 参数注入环境变量
PORT=5005 HISTORY_DISABLED=true pm2 start ./chat2api --name "chat2api-rust"

# 常用 pm2 命令
pm2 status
pm2 logs chat2api-rust
pm2 restart chat2api-rust
pm2 stop chat2api-rust
```

### 3. 使用 systemd 服务守护 (推荐)

在 Linux 系统上，推荐编写 `systemd` 配置文件：

1. 创建并编辑 `/etc/systemd/system/chat2api.service`：

   ```ini
   [Unit]
   Description=Chat2API Rust Service
   After=network.target

   [Service]
   Type=simple
   User=root
   WorkingDirectory=/path/to/your/chat2api
   ExecStart=/path/to/your/chat2api/chat2api
   Restart=always
   RestartSec=5

   # 注入所需的环境变量
   Environment=PORT=5005
   Environment=HISTORY_DISABLED=true
   Environment=RETRY_TIMES=3
   Environment=ENABLE_LIMIT=true
   # Environment=PROXY_URL=http://your_proxy:port

   [Install]
   WantedBy=multi-user.target
   ```

2. 重新加载配置并启动：

   ```bash
   systemctl daemon-reload
   systemctl enable chat2api
   systemctl start chat2api
   
   # 查看状态与日志
   systemctl status chat2api
   journalctl -u chat2api -f
   ```

---

## 方式二：Docker 部署

Docker 部署无需在宿主机上配置 Rust 环境，是容器化管理的最优选。

### 1. 本地构建 Docker 镜像

在项目根目录下（即包含 `Cargo.toml` 的路径），使用以下命令进行构建：

```bash
docker build -t chat2api-rust:latest .
```

### 2. 运行 Docker 容器

```bash
docker run -d \
  --name chat2api-rust \
  -p 5005:5005 \
  -v $(pwd)/data:/app/data \
  -e PORT=5005 \
  -e HISTORY_DISABLED=true \
  -e RETRY_TIMES=3 \
  -e ENABLE_LIMIT=true \
  chat2api-rust:latest
```

> **注意**：挂载 `-v $(pwd)/data:/app/data` 能够确保您在网页上传的 `tokens.txt`、生成的指纹等文件不会因容器重启而丢失。

---

## 方式三：Docker Compose 部署

通过 Docker Compose 能够统一管理服务和可选的代理网络环境。以下是推荐的 `docker-compose.yml` 配置示例：

```yaml
version: '3.8'

services:
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
      - CHATGPT_BASE_URL=https://chatgpt.com
      # 如果需要配置代理：
      # - PROXY_URL=http://username:password@ip:port
    restart: always
```

在同级目录下执行以下命令运行：

```bash
docker compose up -d
```

---

## 方式四：Nginx 反向代理配置参考

通常我们在生产环境中会使用 Nginx 暴露 SSL 证书。对于支持流式响应（Stream）的服务，Nginx 需要关闭响应缓存缓冲，配置参考如下：

```nginx
server {
    listen 443 ssl;
    server_name yourdomain.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://127.0.0.1:5005;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # 核心配置：支持流式传输 (Event-Stream)
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        chunked_transfer_encoding on;
        
        # 延长超时时间以防流式问答断开
        proxy_read_timeout 600s;
        proxy_send_timeout 600s;
    }
}
```
