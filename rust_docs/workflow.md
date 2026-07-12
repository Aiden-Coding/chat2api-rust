# Rust 重构版本核心流程文档

本文档详细描述客户端请求被接收，直至向 OpenAI 官方会话并以 SSE 响应返回给客户端的完整处理流程。

---

## 1. Completions 核心业务流图

下面的序列图展示了请求进入系统后，各个模块协作处理并流式响应的整个生命周期：

```mermaid
sequenceDiagram
    autonumber
    actor Client as 客户端
    participant Routes as 路由层 (routes.rs)
    participant Service as 服务层 (service.rs)
    participant Auth as 授权层 (auth.rs)
    participant POW as 求解器 (pow.rs)
    participant CF as Workers 代理 / 外部
    participant OpenAI as OpenAI 官方接口

    Client->>Routes: POST /v1/chat/completions (携带 Bearer)
    activate Routes
    Note over Routes: 开启最大 Retry Times 的重试保护循环
    
    loop 错误重试与 Token 轮询
        Routes->>Auth: 绑定 Seed 种子 / 轮询账号 Token
        Auth-->>Routes: 返回真实的真实 Token (AccessToken)
        
        Routes->>Service: 实例化 ChatService::new
        activate Service
        Note over Service: 校验本地 limit_details 频控拦截
        alt 被频控限制
            Service-->>Routes: 返回 TooManyRequests (直接本地拦截)
            Note over Routes: continue 循环：更换下一个 Token 重试
        end
        
        Routes->>Service: get_chat_requirements() 握手
        Service->>OpenAI: POST /sentinel/chat-requirements
        OpenAI-->>Service: 返回要求 (需解决 turnstile / proofofwork / arkose)
        
        alt 包含 Proof of Work (POW)
            Service->>POW: get_answer_token()
            POW-->>Service: 返回完成解密的验证 Token (多核并行)
        end
        
        alt 包含 Arkose 验证要求
            Service->>CF: 代理 POST 求解远程 Arkose Token
            CF-->>Service: 返回解密后的 Arkose Token
        end
        
        Service-->>Routes: Sentinel 校验参数备齐
        
        Routes->>Routes: api_messages_to_chat() 转换格式
        Note over Routes: 如果有外部图片，在此处抓取并读取宽高 (0 依赖解析)
        
        Routes->>Service: send_conversation_request() 发送会话
        Service->>OpenAI: POST /conversation (携带 Sentinel/Arkose/POW 头)
        
        alt 连接请求成功 200
            OpenAI-->>Service: 返回 Event-Stream 流式响应主体
            deactivate Service
            Note over Routes: 退出重试循环
        else 返回错误 (例如 429 频控 或 403 拦截)
            OpenAI-->>Service: 返回错误码
            Service->>Service: mark_token_error() 加入黑名单并解析 clears_in 缓存
            Note over Routes: continue 循环：更换下一个 Token 重试
        end
    end

    Routes->>Routes: 包装为 OpenAIStream
    loop 读取 stream 数据块
        Routes->>Routes: 正则解析，滤除敏感词，抽取文件链接
        Routes->>Client: 返回标准的 SSE 数据帧 (data: {choices: [...]})
    end
    
    deactivate Routes
```

---

## 2. 核心子业务控制流

### 2.1 消息转换与多模态下载流程

当客户端传入多模态内容（`image_url`）时，系统的流式处理步骤如下：

```text
[开始解析 messages] 
       │
       ▼
[检查类型 type == "image_url"] ──(No)──► [常规文本直接追加到 parts]
       │ (Yes)
       ▼
[判断 URL 协议]
       ├─► (data:image/...) ──► [Base64 直接内联解码为字节数据]
       └─► (http/https)   ──► [检查是否存在 CF_FILE_URL 代理]
                                     ├─► (Yes) ──► [使用 POST 通过 cf_file_url 中转下载]
                                     └─► (No)  ──► [带入 EXPORT_PROXY_URL 下载图片]
       │
       ▼
[利用 get_image_size 二进制扫描提取图片宽高] ──(失败)──► [回退为普通非图片附件上传]
       │ (成功)
       ▼
[上传文件到 OpenAI 获取 file_id] ──► [根据缩放算法累加 file_tokens]
       │
       ▼
[追加图片文件指针至会话协议的 parts]
```

### 2.2 429 会话级限流拦截与重试自愈机制

1. **接口入口**：由 `routes.rs` 捕获到用户 Completions 请求。
2. **前置限流判定**：
    在 `ChatService::new` 内，从全局状态提取 `limit_details` 映射表，检索 `Token + Model`。如果存在未到期的截止时间戳，直接向路由抛出 `429 TooManyRequests` 异常，此账号在此次轮询中被跳过，直接触发下一次重试迭代。
3. **捕获异常与黑名单排除**：
    向 OpenAI 发起请求若遭遇 `429` 响应，解析 Body 中的 `clears_in`，登记拦截信息，并将此 Token 移入 `error_token_list` 缓存列表防再次调度。
4. **接口自愈**：
    重试循环自动进入下一轮，通过 `get_req_token` 会自动筛选出不在 `error_token_list` 中的健康账号，从而无感恢复服务。
