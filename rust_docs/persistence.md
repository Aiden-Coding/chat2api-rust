# Rust 重构版本数据持久化文档

为了保证高并发部署和进程异常重启时的数据自愈性，Rust 重构版本设计了完善的“内存与文件双写持久化机制”。本篇文档详细说明项目持久化数据的分布、内容含义以及运作原理。

---

## 1. 持久化数据文件概览

所有持久化数据统一保存在项目根目录下的 `data/` 文件夹中：

| 磁盘文件名/表名 | 数据格式 | 持久化存储的内容 | 作用与自愈机制 |
| :--- | :--- | :--- | :--- |
| **`token.txt` (已废弃)** | 纯文本 | **主可用 Token 配置文件**。 | 仅在首次启动时，若 SQLite 中无数据，执行一次性迁移并导入数据库，此后停用。 |
| **`error_token.txt` (已废弃)** | 纯文本 | **异常失效 Token 配置文件**。 | 仅在首次启动时，若 SQLite 中无数据，执行一次性迁移并导入数据库，此后停用。 |
| **`data.db` (数据库)** | **SQLite 3** | **核心业务数据与缓存持久化数据库**。存储了内部所有业务映射与 Token 数据。 | 包含 `tokens`、`error_tokens`、`refresh_cache`、`wss_cache`、`fp_cache`、`seed_cache`、`conversation_cache` 共 7 张表。存储结构均采用 KV 格式（`key TEXT PRIMARY KEY, val TEXT`）。 |

### `data.db` 数据表定义与作用说明：
1. **`tokens`** (主可用 Token 池)：运行时轮询与随机调度的底层活跃账号池，可以通过 API 接口动态追加。
2. **`error_tokens`** (异常失效 Token 黑名单)：存放由于失效被剔除的账号，防止故障账号干扰业务请求，在内存校验时自动排除它们。
3. **`refresh_cache`** (Token 刷新缓存)：记录 RefreshToken 到其换取出的 AccessToken 以及刷新时间戳的 JSON 缓存（有效期 5 天），避免高频重复刷新官方 Auth0 接口导致的风控。
4. **`fp_cache`** (浏览器拟态指纹缓存)：记录每个 Token 专属的伪装浏览器指纹（User-Agent、sec-ch-ua 等 Client-Hints 属性），保证该账号在 OpenAI 侧访问设备特征的一致性。
5. **`seed_cache`** (隔离会话绑定缓存)：网关模式下隔离种子（Seed Token）和其绑定的真实官方 AccessToken 的一对一映射。
6. **`conversation_cache`** (历史会话路由缓存)：记录 `conversation_id` 到具体 Token 的路由，保证同一会话上下文连续提问被正确分发给同一个账号。
7. **`wss_cache`** (WebSocket 连接缓存)：记录建立握手接入连接的 WebSocket URL 缓存，减少高频重复握手。

---

## 2. 数据双写持久化工作原理

项目在状态管理层采用了**“内存同步高并发读 + 数据库异步写盘（SQLite）”**的双向绑定机制：

### 2.1 运行时更新 (运行时双写)

以获取新 Seed 绑定或动态追加 Token 为例：

1. 客户端发起请求，`AppState` 进行内存数据操作，获取写锁并更新内存中对应的缓存向量/字典。
2. 随后通过 `tokio::task::spawn_blocking` 在后台线程池中同步向 `data.db` 数据库的对应表中执行 `INSERT OR REPLACE` 写入，实现局部单行更新。
3. 这样做使得整个 I/O 操作完全不阻塞 Actix-web 的主工作线程，避免了高并发下的磁盘写瓶颈。

### 2.2 启动时恢复 (自愈初始化)

当服务因为升级、宿主机维护或异常断电而发生重启时：

1. `main.rs` 首先拉起 `AppState::new(&config)` 实例化。
2. 程序首先创建 `data/` 目录并调用 `init_db()` 初始化 SQLite 数据库，若 7 张核心表不存在则自动执行建表建索引。
3. **平滑数据迁移**：程序首先检查 `tokens` 和 `error_tokens` 表是否为空。如果为空，且本地存在历史的 `token.txt` / `error_token.txt` 配置文件，系统会自动读取它们并将所有 Token 自动无缝迁移导入 SQLite 数据库中。
4. 程序全量 `SELECT` 捞出所有记录，直接加载并反序列化回内存对应的 HashMap 中，以提供零 I/O 阻塞的极速内存级查询服务。
5. 系统恢复完毕，无缝继续提供服务，历史状态完全对齐。
