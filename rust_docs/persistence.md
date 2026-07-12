# Rust 重构版本数据持久化文档

为了保证高并发部署和进程异常重启时的数据自愈性，Rust 重构版本设计了完善的“内存与文件双写持久化机制”。本篇文档详细说明项目持久化数据的分布、内容含义以及运作原理。

---

## 1. 持久化数据文件概览

所有持久化数据统一保存在项目根目录下的 `data/` 文件夹中：

| 磁盘文件名 | 数据格式 | 持久化存储的内容 | 作用与自愈机制 |
| :--- | :--- | :--- | :--- |
| **`token.txt`** | 纯文本 | **主可用 Token 池**。每行存放一个 AccessToken 或 RefreshToken。 | 运行时轮询与随机调度的底层账号池来源。可以使用 GET /tokens/add 动态追加。 |
| **`error_token.txt`** | 纯文本 | **异常失效 Token 列表**。每行存放一个已过期的 Token。 | 运行时自动剔除这些 Token，防止故障账号干扰请求。服务重启会自动排除它们。 |
| **`refresh_map.json`** | JSON | **Token 刷新映射**。记录 RefreshToken 到其换取出的 AccessToken 以及刷新时间戳。 | 缓存有效期为 5 天。有效期内直接返回缓存的 AccessToken，规避高频请求官方 Auth0 接口导致的风控。 |
| **`fp_map.json`** | JSON | **拟态浏览器指纹**。记录为每个 Token 随机计算生成的 User-Agent 以及 sec-ch-ua 等 Client-Hints 属性。 | 确保同一个 Token 的请求在 OpenAI 侧看起来始终是由同一个设备发出的，极大地规避封号风控。 |
| **`seed_map.json`** | JSON | **隔离会话绑定**。记录传入的随机隔离种子（Seed Token）和其所一对一绑定的官方真实 Token。 | 网关/镜像模式下，实现多用户的会话防串改与用户数据隔离。 |
| **`conversation_map.json`** | JSON | **会话与 Token 映射**。记录 `conversation_id` 与对应 Token 的映射。 | 确保用户在同一个上下文（Conversation）中多次追问时，自动路由到同一个 Token 进行连贯提问。 |
| **`wss_map.json`** | JSON | **WebSocket 缓存**。记录目前建立好握手接入连接的 WebSocket URL 和时间戳。 | 减少重复握手，优化连接性能。 |

---

## 2. 数据双写持久化工作原理

项目在状态管理层采用了**“内存同步更新 + 异步写盘持久化”**的双向绑定机制：

### 2.1 运行时更新 (运行时双写)

以追加单个 Token 为例：

1. 客户端调用 `GET /tokens/add/{token}` 路由。
2. `AppState` 获得独占写锁（`tokio::sync::RwLock::write`），将 Token 克隆压入内存的 `token_list` 向量中。
3. 程序自动异步读取并重新回写至物理磁盘的 `data/token.txt` 尾部，完成双写，确保内存与磁盘数据时时同步。

### 2.2 启动时恢复 (自愈初始化)

当服务因为升级、宿主机维护或异常断电而发生重启时：

1. `main.rs` 首先拉起 `AppState::new(&config)` 实例化。
2. 程序通过文件系统判定 `data/` 目录以及上述 7 个数据文件是否存在。如果存在，会依次将其加载到内存中反序列化为对应的读写锁字典（如 `HashMap`）。
3. 系统自愈完毕，无缝继续提供 API 问答服务，历史的 Token、缓存的指纹以及频控状态完全恢复。
