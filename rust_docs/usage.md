# Rust 重构版本使用文档

本文档介绍如何使用 Rust 版本的各项 API 接口，并提供常用的配置项指南。

---

## 1. 环境变量详细配置指南

项目支持通过系统环境变量或根目录下创建的 `.env` 文件进行加载。

| 环境变量名 | 示例值 | 默认值 | 描述 |
| :--- | :--- | :--- | :--- |
| **`PORT`** | `8080` | `5005` | 服务本地监听的端口号。 |
| **`API_PREFIX`** | `mysecret` | 空 | 路由前缀密码隔离。设置后，请求地址需变更为 `/mysecret/v1/chat/completions`。 |
| **`AUTHORIZATION`** | `sec1,sec2` | 空 | 客户端调用本服务时需填入的 APIKEY。支持逗号分隔，匹配后将轮询账号池。 |
| **`AUTH_KEY`** | `custom_gate_key` | 空 | 专属 ChatGPT 逆向网关所需的自定义 Auth 校验头。 |
| **`PROXY_URL`** | `http://127.0.0.1:7890` | 空 | 全局代理列表（支持英文逗号分隔）。每次请求均会为客户端分配随机代理节点。 |
| **`EXPORT_PROXY_URL`** | `http://127.0.0.1:1080` | 空 | 出口代理。专门在多模态资源（如图片）抓取下载时使用，保护隐私。 |
| **`CF_FILE_URL`** | `https://worker-url` | 空 | Cloudflare Workers 代理接口。用以后端以中转下载图片等多模态文件。 |
| **`HISTORY_DISABLED`**| `true`/`false` | `true` | 是否禁用官网历史存档，默认为 `true`（免登录模式必须为 true）。 |
| **`RETRY_TIMES`** | `3` | `3` | 会话层错误最大轮询重试次数。 |
| **`ENABLE_LIMIT`** | `true` | `true` | 是否启用本地频控机制，缓存并拦截 429 会话，防封禁。 |
| **`RANDOM_TOKEN`** | `true`/`false` | `true` | 账号池选取策略。`true` 为随机，`false` 为多线程 Atomic 顺序轮询。 |

---

## 2. Completions 接口使用示例

与标准 OpenAI 接口格式完全对齐。支持传入真实的 `AccessToken`，或者传入配置的本地 `AUTHORIZATION` 授权码。

### 2.1 常规非流式会话
*   **请求路径**：`POST /v1/chat/completions`
*   **请求头**：
    - `Content-Type: application/json`
    - `Authorization: Bearer <your_token_or_auth_code>`
*   **请求体**：
```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": "请用 Rust 编写一个经典二分查找算法。"
    }
  ],
  "stream": false
}
```

### 2.2 传入 Base64 格式内联图片的多模态会话
在 `content` 数组的 `image_url` 内，可以直接传入 base64 编码的二进制流：
*   **请求体**：
```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "text",
          "text": "分析下这张图片里有什么？"
        },
        {
          "type": "image_url",
          "image_url": {
            "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAUA..."
          }
        }
      ]
    }
  ],
  "stream": false
}
```

---

## 3. Web 可视化 Tokens 管理端

*   **访问路径**：`GET /tokens` (或带 API 前缀路径)
*   **交互逻辑**：
    1.  提供文本框用于批量贴入 Tokens 列表（支持 `AccessToken` 或 `RefreshToken`，一行一个）。
    2.  页面能够动态渲染当前的活跃/正常 Token 总数。
    3.  提供一键清空后台 Token 池的功能。

---

## 4. 种子绑定清空接口

如果您启用了 `auto_seed` 属性，项目会自动缓存并隔离每个用户 Seed 绑定的官方 Token。如果您想要重新打乱和绑定，可调用此接口：

*   **请求路径**：`POST /seed_tokens/clear`
*   **响应示例**：
```json
{
  "status": "success",
  "seed_tokens_count": 0
}
```
该请求会重置本地缓存，且擦除 `data/seed_map.json` 与 `data/conversation_map.json` 并重新写盘。
