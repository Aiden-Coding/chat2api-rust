# Grok API 配置指南

## 问题说明

当使用 Grok Web 模式（如 `grok-4.3-fast`）时，可能会遇到 **403 Forbidden** 错误：

```
"error": {
  "code": 7,
  "message": "Request rejected by anti-bot rules."
}
```

这是因为 Grok Web API (`grok.com`) 使用了 Cloudflare 反机器人保护。

## 解决方案

### 方案 1：使用 Console API（推荐）

最简单的方法是使用 Console 模式，它的反检测更宽松：

```bash
curl -X POST http://127.0.0.1:5005/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "grok-4.3-console",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": false
  }'
```

**Console 模式可用的模型：**
- `grok-4.3-console`
- `grok-4.3-low`
- `grok-4.3-medium`
- `grok-4.3-high`
- `grok-4.20-0309-console`
- `grok-4.20-multi-agent-console`
- `grok-build-console`

### 方案 2：添加 cf_clearance（支持 Web 模式）

如果需要使用 Web 模式（`grok-4.3-fast` 等），需要配置 `cf_clearance`。

#### 步骤 1：获取 cf_clearance

1. 用浏览器访问 https://grok.com
2. 按 `F12` 打开开发者工具
3. 切换到 `Application` 标签（Chrome）或 `Storage` 标签（Firefox）
4. 在左侧展开 `Cookies` → `https://grok.com`
5. 找到 `cf_clearance`，复制它的值

![获取 cf_clearance](docs/cf_clearance_example.png)

#### 步骤 2：配置环境变量

编辑你的 `.env` 文件，添加：

```env
CF_CLEARANCE=你复制的cf_clearance值
```

示例：
```env
CF_CLEARANCE=Y2KOfB.lX4cv3AykBB9FSv42_6wHJcFAOu0Aue4U3m0-1783873262-1.2.1.1-LskrNjAuSDWj8wdoP4ICCvMOilEpuSj4.zUtzLwMNl4_Pkby2D27SJ7NA...
```

#### 步骤 3：重启服务

```bash
# 重新编译
cargo build --release

# 重启服务
./target/release/chat2api
```

#### 步骤 4：测试

```bash
curl -X POST http://127.0.0.1:5005/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "grok-4.3-fast",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": false
  }'
```

## 注意事项

### cf_clearance 有效期

- **有效期**：通常为几小时到几天
- **过期后**：需要重新获取
- **失效症状**：再次出现 403 错误

### 自动化获取 cf_clearance

如果需要自动化，可以使用：

1. **Selenium + undetected-chromedriver**
2. **Playwright** with stealth mode
3. **FlareSolverr** 服务

### 模型对比

| 特性 | Console 模式 | Web 模式 |
|------|-------------|----------|
| 反检测难度 | 低 | 高 |
| 需要 cf_clearance | ❌ | ✅ |
| 模型名示例 | grok-4.3-console | grok-4.3-fast |
| API 端点 | console.x.ai | grok.com |
| 推荐使用 | ✅ | 仅在必要时 |

## 技术细节

### 反检测头

项目已自动添加以下反检测头（Web 模式）：

```rust
// Sentry 追踪信息
headers.insert("baggage", "sentry-environment=production,...");

// Statsig 指纹
headers.insert("x-statsig-id", "ZTpUeXBlRXJyb3I6...");

// XAI 请求 ID
headers.insert("x-xai-request-id", uuid);

// Cloudflare Clearance
Cookie: sso=xxx; sso-rw=xxx; cf_clearance=xxx
```

### 浏览器指纹

项目使用 `rquest` 库模拟 Chrome 浏览器指纹，包括：

- TLS 指纹（JA3）
- HTTP/2 指纹
- User-Agent
- Sec-CH-UA headers

## 故障排查

### 仍然出现 403

1. **确认 cf_clearance 未过期**
   - 重新获取 cf_clearance
   - 检查浏览器访问 grok.com 是否正常

2. **检查配置加载**
   ```bash
   # 启动时查看日志
   grep "CF_CLEARANCE" logs/app.log
   ```

3. **尝试 Console 模式**
   ```bash
   # 改用 console 模型
   "model": "grok-4.3-console"
   ```

### 429 Too Many Requests

- 触发了速率限制
- 等待几分钟后重试
- 使用多个 SSO token 轮换

### 其他错误

查看完整日志：
```bash
RUST_LOG=debug cargo run
```

## 参考

- [grok2api Python 实现](https://github.com/xxx/grok2api)
- [rquest 文档](https://github.com/0x676e67/rquest)
- [Cloudflare 反检测策略](https://developers.cloudflare.com/fundamentals/reference/cloudflare-challenges/)
