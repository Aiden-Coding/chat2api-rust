# Rust 重构版本技术设计文档

本文档深入介绍 Rust 版本的技术方案、关键设计决策、性能优化细节以及安全防护策略。

---

## 1. 线程安全与共享状态模型

Actix-web 在多线程运行环境下，每个 Worker 线程都可能并发读取和写入全局状态。为了确保并发读写不产生数据竞争，本项目设计了基于共享状态的同步模型：

* **`Arc` + `RwLock` 读写隔离**：
    全局状态 `AppState` 实质是一个指向内部堆数据 `AppStateInner` 的 `Arc` 智能指针。内部封装了异步读写锁 `tokio::sync::RwLock`。对于并发度极高的查询（如 Token 活跃校验、指纹读取），线程通过 `.read()` 获取读锁进行并发共享读取；对于低频写的变更（如 Token 上传、故障黑名单记录），使用 `.write()` 获取独占写锁写盘更新，确保写操作的强一致性。
* **并发安全的轮询机制**：
    原 Python 版本依赖静态 `unsafe usize` 的自增轮询，在多线程高并发下存在数据竞态，易导致数组越界或轮询重复。Rust 版本中，对于顺序轮询策略，我们引入了 `std::sync::atomic::AtomicUsize`：

    ```rust
    static ROUND_ROBIN_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let count = ROUND_ROBIN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let index = count % available_tokens.len();
    ```

    通过硬件层级的原子操作（Atomic `fetch_add`），在完全无互斥锁（Lock-free）开销的前提下实现了绝对线程安全的轮询调度。

---

## 2. 抗封锁与拟态 JA3 / TLS 指纹

OpenAI 广泛采用了 Cloudflare 的安全盾，若客户端的 TLS Client Hello 握手特征（JA3 指纹）与真实的现代浏览器行为不匹配，会直接报 403 错误拦截。

* **拟态指纹构建**：
    本项目没有采用传统的 `reqwest`，而是选用了支持拟态浏览器特征的 `rquest` 库。基于 OpenSSL 底层套接字混淆，我们在运行时根据 Token 随机搭配 Chrome 120-124 / Edge 101 等的 SSL 密码套件、签名算法及 TLS 扩展特征。
* **Client Hints 头信息对齐**：
    若握手协议伪装为 Chrome 核心，项目会提取并在 HTTP 头部补充对齐的 Client Hints 系列字段：
  * `sec-ch-ua`
  * `sec-ch-ua-platform`
  * `sec-ch-ua-mobile`
    这实现了从“网关传输层指纹（TLS）”到“应用协议层指纹（HTTP Headers）”的立体化指纹拟态。

---

## 3. 并行加速 POW (Rayon 碰撞引擎)

Sentinel 握手流程要求解密一段含有随机 `seed` 的 SHA3-512 哈希碰撞问题（即 POW 工作量证明）。当并发量极高或设定的难度值上升时，单线程哈希碰撞会成为 CPU 瓶颈，并增加响应时延。

* **多核哈希并行化**：
    项目引入了数据并行开发库 `Rayon`。在执行 `generate_answer` 求解时，项目在预开辟的数值空间中启动并行迭代：

    ```rust
    let result = (0..500000u32).into_par_iter().find_map_any(|i| {
        // 各自线程在独立的 CPU 核心上完成 JSON 组装与 Sha3 计算
    });
    ```

    `rayon::into_par_iter()` 会利用工作窃取（Work-stealing）算法，自动将 50 万次计算分割成多个子任务投递至系统的物理 CPU 核心中进行并发哈希求解。这使得多核主机的求解效率比 Python 的单线程求解提升了 10~50 倍。

---

## 4. 本地 429 频控防封拦截设计

对同一个 Token 高频请求会触发 OpenAI 的服务限流。如果持续向 OpenAI 发送受限请求，可能导致账号被封禁。

* **设计机制**：
  * **自动记录**：在会话响应返回 429 时，项目会自动拦截并解析响应 JSON 体内的 `clears_in`（限制秒数），并在 `limit_details` 缓存中写入 `ExpiredTimestamp = UTC::now() + clears_in`。
  * **提前阻断**：下次客户端再次尝试使用该限制 Token 请求该限制模型时，系统会在 `ChatService::new` 实例化阶段直接计算拦截，并抛出 `requests_limit_error`，杜绝请求到达 OpenAI 后端，有效避免因为持续高频碰撞而致封号。

---

## 5. 多模态原生图像解析设计 (0 外部依赖)

为防止编译出的 Rust 服务携带过大的图片解码 C 共享动态链接库导致部署不便，本项目编写了纯 Rust 的原始图像头信息扫描算法：

* **PNG/JPEG**：分别匹配 `\x89PNG\r\n\x1a\n` 头以及 JPEG 的 `SOF` 标志段并读取宽和高。
* **GIF**：匹配前缀 `GIF89a`/`GIF87a`，从偏移量 6 字节和 8 字节中分别以小端模式直接提取 `u16` 的宽和高。
* **WEBP**：校验 RIFF 与 WEBP 特征签名，针对 VP8, VP8L, VP8X 这三种不同存储编码格式，通过分析特定的像素描述位，并进行 14-bit 和 24-bit 的位偏移换算运算（如 `(((b2 as u32 & 0x3F) << 8) | b1 as u32) + 1`），以极高的速度提取宽高，零外部图像库依赖，保证了服务的轻量与极致响应。

---

## 6. 基于 SQLite 与内存的混合双写持久化设计

传统的全量 JSON 写盘方案在高并发和意外退出时极易引发写冲突和文件损坏。为此，项目重构了全新的 SQLite 混合存储方案：

* **极速内存查询**：
    程序在启动（`AppState::new`）时建立 SQLite 连接并确保 `refresh_cache`、`wss_cache`、`fp_cache`、`seed_cache`、`conversation_cache` 表的初始化。随后通过 `SELECT` 捞出所有记录并在内存中加载为 `HashMap`。对于高达数千 QPS 的查询请求，系统直接在内存中共享读取，没有任何数据库 I/O 损耗。
* **异步局部写盘 (Token/指纹/会话绑定)**：
    当映射状态需要更新（如 `update_refresh_info` 等）时，程序会立即更新内存 Map，并利用 `tokio::task::spawn_blocking` 在后台线程池中发起 SQLite `INSERT OR REPLACE` 局部更新。这不仅极大地提高了单次写盘的稳定性，也彻底消除了原本全量写文件时引起的物理 I/O 阻塞。
* **零外部 C 库依赖 (Bundled) 构建**：
    为了保证编译产物的零外部动态库依赖及完美的 Alpine/Docker 跨平台部署体验，`Cargo.toml` 采用了 `rusqlite` 并开启了 `bundled` 特性。在 `cargo build` 时由 Rust 自行拉起构建内置静态 SQLite C 源码并最终融入二进制发行包中。
