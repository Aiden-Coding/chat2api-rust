# Rust 版本使用指南

本项目已采用 Rust 进行重构，以满足高并发、低内存消耗的生产环境需求。本文档将详细介绍 Rust 版本的使用方式、核心特性、环境变量以及接口定义。

## 环境变量说明

Rust 版本的配置通过读取系统的环境变量或根目录下的 `.env` 文件实现。

### 安全相关

* **`API_PREFIX`**
  * **默认值**：空
  * **描述**：路由前缀隔离。如果设置了 `API_PREFIX=api_sec`，那么您请求的端点将变为 `http://127.0.0.1:5005/api_sec/v1/chat/completions`。
* **`AUTHORIZATION`**
  * **默认值**：空
  * **描述**：由您自己定义的身份验证令牌（支持逗号分隔的多个授权码，如 `token1,token2`）。
  * **作用**：设置后，客户端向本项目发起请求时必须携带该授权码（`Bearer your_auth_code`），此时项目会从后台账号池中轮询或随机选择真实 Token 发起会话。
* **`AUTH_KEY`**
  * **描述**：私人专属 ChatGPT 网关验证秘钥。

### 请求相关

* **`CHATGPT_BASE_URL`**
  * **默认值**：`https://chatgpt.com`
  * **描述**：ChatGPT 网关地址。支持逗号分隔配置多个，项目会随机挑选可用网关。
* **`PROXY_URL`**
  * **描述**：用于与官网握手与发送请求的全局代理。支持逗号分隔配置多个代理链接（格式为 `http://ip:port` 或带用户密码的格式）。
* **`EXPORT_PROXY_URL`**
  * **描述**：出口代理。专门用于抓取并下载多模态资源（如外部图像文件），防泄露源站 IP。
* **`CF_FILE_URL`**
  * **描述**：Cloudflare Workers 提供的多模态资源代理下载中转 URL。
* **`PORT`**
  * **默认值**：`5005`
  * **描述**：程序本地监听服务的网络端口。

### 功能相关

* **`HISTORY_DISABLED`**
  * **默认值**：`true`
  * **描述**：是否禁用历史会话保存。免登录模式必须关闭历史保存。
* **`POW_DIFFICULTY`**
  * **默认值**：`000032`
  * **描述**：需要解算的工作量证明（Proof of Work）目标难度值。
* **`RETRY_TIMES`**
  * **默认值**：`3`
  * **描述**：会话接口最大失败重试次数。在重试期间，项目会自动轮询切换至其他可用的健康 Token。
* **`ENABLE_LIMIT`**
  * **默认值**：`true`
  * **描述**：是否开启本地频控，记录并提前拦截被 ChatGPT 限流的账号，防止盲目发请求导致账号被封。
* **`OAI_LANGUAGE`**
  * **默认值**：`zh-CN`
  * **描述**：向官网握手时发送的首选区域语言。

---

## 接口使用说明

### 1. 核心会话接口

* **请求地址**：`POST /v1/chat/completions` (或 `POST /{{API_PREFIX}}/v1/chat/completions`)
* **请求头**：
  * `Content-Type: application/json`
  * `Authorization: Bearer <your_token_or_authorization_code>`
  * `Chatgpt-Account-Id: <Optional_Team_Account_Id>` (如果需要切换 Team 工作区)

#### 示例 1：非流式对话 (JSON 响应)

```bash
curl --location 'http://127.0.0.1:5005/v1/chat/completions' \
--header 'Content-Type: application/json' \
--data '{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": "你是什么模型？"
    }
  ],
  "stream": false
}'
```

#### 示例 2：多模态对话 (发送外部图片 URL)

```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "text",
          "text": "描述这张图片的内容"
        },
        {
          "type": "image_url",
          "image_url": {
            "url": "https://example.com/sample.png"
          }
        }
      ]
    }
  ],
  "stream": false
}
```

#### 示例 3：多模态对话 (发送 Base64 图片)

支持将图片直接编码为 Base64 格式并作为 inline data url 传入：

```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "text",
          "text": "提取图片中的文字"
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

### 2. Tokens 网页管理

* **访问地址**：`GET /tokens` (或 `GET /{{API_PREFIX}}/tokens`)
* **功能**：提供可视化的界面进行批量 Token（支持 AccessToken 与 RefreshToken）的导入、清空，并直观显示目前后台活跃的健康 Token 数量。

### 3. 会话隔离种子清理

* **请求地址**：`POST /seed_tokens/clear` (或 `POST /{{API_PREFIX}}/seed_tokens/clear`)
* **功能**：重置并清空保存在本地的随机 Seed 种子与会话隔离关系图（`seed_map.json` / `conversation_map.json`）。
