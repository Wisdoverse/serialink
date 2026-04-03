# Serialink Architecture

## Overview / 概览

Serialink is a three-layer system. The serial abstraction layer owns
hardware access, the data pipeline engine processes serial data when it is
integrated, and the interface layer exposes the public entry points. The
current shipped surface includes the CLI, MCP stdio, MCP SSE, HTTP REST API,
and the embedded Web UI dashboard. The pipeline engine already exists as a
standalone library, but it is not yet wired into the current reader path.

serialink 是一个三层系统：串口抽象层负责硬件访问，数据流水线引擎在
接入后负责处理串口数据，接口层则提供对外入口。当前已经交付的能力包括
CLI、MCP stdio、MCP SSE、HTTP REST API，以及内嵌 Web UI 仪表盘。
pipeline 引擎作为独立库已经存在，但还没有接入当前读取路径。

```
+-------------------------------------------------------+
|                   Interface Layer / 接口层            |
| CLI / 命令行 | MCP Server (stdio/SSE) / MCP 服务      |
| HTTP REST API + Web UI / HTTP REST API + 内嵌 Web UI   |
+-------------------------------------------------------+
|                 Data Pipeline Engine                   |
| standalone library; not yet wired into current reader |
+-------------------------------------------------------+
|               Serial Abstraction Layer / 串口抽象层    |
|   Discovery  |  SerialConnection  |  SessionManager    |
+-------------------------------------------------------+
|              Hardware / OS Serial Ports                |
+-------------------------------------------------------+
```

Each layer depends only on the layer below it. The interface layer never
touches `serialport` directly; it always goes through the session manager.
The pipeline engine sits between the reader and consumers only after it is
integrated into the current read path.

每一层都只依赖其下一层。接口层不会直接接触 `serialport`，而是始终通过
session manager 访问串口。pipeline 引擎只有在接入当前读取路径后才会位于
读取端与消费者之间。

## Serial Abstraction Layer / 串口抽象层

Serial abstraction is fully implemented today. It owns device discovery,
open-port management, and session lifecycle handling. Higher layers use
`SessionManager` instead of talking to hardware directly, which keeps the
interface layer independent from `serialport`.

串口抽象层已经在当前版本中完整实现。它负责设备发现、端口打开管理和会
话生命周期。上层通过 `SessionManager` 访问硬件，而不是直接操作
`serialport`，这样接口层就能保持独立。

## Data Pipeline Engine / 数据流水线引擎

Status: implemented as a standalone library, not yet wired into the current
reader path. The pipeline processes serial data through ordered transforms
and can already be used in isolation, but raw `TimestampedLine` data still
flows directly from the reader to consumers today. Planned integration will
place the engine between the reader and downstream consumers.

状态：作为独立库已实现，但尚未接入当前读取路径。pipeline 会按顺序处理
串口数据转换链，在独立使用时已经可用，但当前 `TimestampedLine` 仍然直接
从 reader 流向消费者。规划中的集成会把 engine 放到 reader 与下游消费者
之间。

## Interface Layer / 接口层

Status: implemented today. The interface layer exposes the CLI for humans
and scripts, MCP stdio and SSE for agents, and the HTTP REST API plus
embedded Web UI dashboard for browser and remote workflows. It is the public
surface that turns the lower layers into commands, tools, and endpoints.

状态：当前已实现。接口层向人类和脚本提供 CLI，向代理提供 MCP stdio 和
SSE，向浏览器和远程工作流提供 HTTP REST API 及内嵌 Web UI 仪表盘。它是
把下层能力转换成命令、工具和接口的公共入口。

### `discovery.rs` -- Port Enumeration / 端口枚举

`list_ports()` calls `serialport::available_ports()` and maps the result
into a `Vec<PortInfo>`. Each `PortInfo` includes the port name, type
(USB / PCI / Bluetooth / Unknown), and optional USB metadata (VID, PID,
serial number, manufacturer).

This is a synchronous, one-shot operation with no persistent state.

`list_ports()` 会调用 `serialport::available_ports()`，并把结果映射成
`Vec<PortInfo>`。每个 `PortInfo` 都包含端口名、类型（USB / PCI /
Bluetooth / Unknown）以及可选的 USB 元数据（VID、PID、序列号、厂商）。

这是一个同步、一次性的操作，不保存任何持久状态。

### `port.rs` -- SerialConnection / 串口连接

`SerialConnection` is the core abstraction for an open serial port. It
manages:

- **Background reader task** -- A tokio task that continuously reads from the
  serial port using `spawn_blocking` (because `serialport-rs` is a
  synchronous library) and feeds lines into the system. The `JoinHandle`
  for this task is stored in the connection and awaited during `close()`,
  ensuring clean shutdown with no orphaned tasks.
- **Ring buffer** -- A `VecDeque<TimestampedLine>` capped at 10,000 lines.
  When the buffer is full, the oldest line is evicted. This bounds memory
  usage during long-running monitoring sessions.
- **Broadcast channel** -- A `tokio::sync::broadcast::channel` with a
  capacity of 256 messages. Every received line is sent to the broadcast
  channel, enabling multiple concurrent subscribers (CLI output, MCP tool
  reads, planned WebSocket streaming endpoints) to receive the same data
  independently.
- **Auto-reconnect** -- When a read error occurs (device unplugged, USB
  reset), the reader task transitions through `Disconnected` ->
  `Reconnecting` states and periodically attempts to reopen the port. The
  reconnect interval is configurable (default: 2000ms).
- **Write support** -- `write_data()` sends bytes to the port inside a
  `spawn_blocking` closure, acquiring the shared mutex to access the
  underlying `serialport::SerialPort` handle. Writes are wrapped in a
  5-second `tokio::time::timeout` to prevent indefinite blocking.
- **Port cloning** -- The serial port handle is cloned once at open time
  via `try_clone()`. The clone is used exclusively by the background reader
  task; the original is stored in shared state for writes. This avoids
  per-read mutex contention and eliminates the need to lock shared state
  on every read cycle.

`SerialConnection` 是打开串口后的核心抽象。它负责：

- **后台读取任务** -- 使用 `spawn_blocking` 持续读取串口数据，并把行写
  入系统。该任务的 `JoinHandle` 会保存在连接中，并在 `close()` 时等待
  完成，从而保证不会留下孤儿任务。
- **环形缓冲区** -- 使用 `VecDeque<TimestampedLine>`，上限为 10,000 行。
  缓冲满时会丢弃最旧的一行，以控制长时间监控时的内存占用。
- **广播通道** -- 使用 `tokio::sync::broadcast::channel(256)` 把收到的
  行分发给所有订阅者，支持 CLI 输出、MCP 工具读取以及规划中的 WebSocket
  流式端点。
- **自动重连** -- 当读取出错时（例如设备拔出或 USB 重置），读取任务会
  进入 `Disconnected` -> `Reconnecting` 状态，并按间隔重试打开端口。
- **写入支持** -- `write_data()` 在 `spawn_blocking` 闭包中发送字节，并使用
  5 秒超时防止写入阻塞无限期卡住。
- **端口克隆** -- 串口句柄只在打开时通过 `try_clone()` 克隆一次。克隆件
  专供后台读取任务使用，原始句柄保留在共享状态里用于写入，这样可以避免
  每次读取都做额外的锁竞争。

Key types:

| Type               | Purpose                                      |
|--------------------|----------------------------------------------|
| `PortConfig`       | Baud rate, data bits, parity, flow control   |
| `ConnectionStatus` | Connected / Disconnected / Reconnecting / Closed |
| `TimestampedLine`  | A line with its receive timestamp and raw bytes |
| `SerialConnection` | The full managed connection (cloneable via Arc) |

| 类型 | 作用 |
|------|------|
| `PortConfig` | 波特率、数据位、校验位、流控 |
| `ConnectionStatus` | 已连接 / 已断开 / 重连中 / 已关闭 |
| `TimestampedLine` | 带接收时间戳和原始字节的数据行 |
| `SerialConnection` | 完整的托管连接（可通过 `Arc` 克隆） |

### `manager.rs` -- SessionManager / 会话管理器

`SessionManager` holds a `HashMap<String, Arc<SerialConnection>>` behind an
`Arc<Mutex<...>>`. It provides:

- `create_session(port_path, config)` -- Opens a connection, assigns a UUID
  v4 session ID, stores it, and returns the ID. Rejects the request if
  `MAX_SESSIONS` (16) has been reached or if the port is already open in
  another session.
- `get_session(id)` -- Looks up a session by ID.
- `close_session(id)` -- Cancels the background reader, awaits its
  `JoinHandle`, and removes the session from the map.
- `list_sessions()` -- Returns summary info for all active sessions.
- `close_all()` -- Iterates all sessions and closes each one. Intended for
  use during graceful shutdown (e.g., when the MCP server exits or a CLI
  command completes).

Session IDs are UUIDs, making them safe for use in URLs, JSON payloads, and
multi-tenant scenarios where multiple AI agents share one Serialink instance.

`SessionManager` 在 `Arc<Mutex<...>>` 保护下维护 `HashMap<String,
Arc<SerialConnection>>`。它提供：

- `create_session(port_path, config)` -- 打开连接，分配 UUID v4 会话 ID，
  存储并返回该 ID。如果达到 `MAX_SESSIONS`（16）上限，或者该端口已被其
  他会话打开，则会拒绝请求。
- `get_session(id)` -- 按会话 ID 查找会话。
- `close_session(id)` -- 取消后台读取任务，等待其 `JoinHandle` 完成，并将
  会话从映射中移除。
- `list_sessions()` -- 返回所有活跃会话的摘要信息。
- `close_all()` -- 迭代关闭所有会话，适用于 CLI 命令结束或 MCP 服务停止时
  的优雅退出。

会话 ID 使用 UUID，因此可安全用于 URL、JSON 载荷以及多个 AI 代理共享同一
个 Serialink 实例的场景。

## Data Pipeline Engine (`src/pipeline/`) / 数据流水线引擎（`src/pipeline/`）

The pipeline processes serial data through an ordered chain of transforms.

**Status: Phase 2 -- Planned Integration.** The pipeline engine is fully
implemented and tested as a standalone component, but it is not yet wired
into the serial reading path. Currently, raw `TimestampedLine` data flows
directly from the ring buffer and broadcast channel to consumers without
passing through the pipeline. Integration into the reader loop is planned
for a later integration pass.

pipeline 按照有序的转换链处理串口数据。

**状态：Phase 2 -- 规划中的集成。** pipeline 引擎作为独立组件已经完
整实现并通过测试，但还没有接入串口读取路径。当前原始 `TimestampedLine`
仍然会直接从环形缓冲区和广播通道流向消费者，尚未经过 pipeline 处理。
后续集成会把引擎放到 reader loop 和下游消费者之间。

### `transform.rs` -- Transform Trait and DataChunk / 转换 trait 与 DataChunk

```rust
pub struct DataChunk {
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub raw: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

#[async_trait]
pub trait Transform: Send + Sync {
    async fn process(&self, input: DataChunk) -> Vec<DataChunk>;
    fn name(&self) -> &str;
}
```

`DataChunk` carries a timestamp, string content, raw bytes, and an
extensible metadata map. Transforms can enrich metadata (e.g., the log level
parser adds a `level` key) without losing the original data.

`DataChunk` 包含时间戳、字符串内容、原始字节以及可扩展的元数据映射。
转换器可以在不丢失原始数据的前提下补充元数据，例如日志级别解析器会添
加 `level` 键。

The `process` method returns a `Vec<DataChunk>`, enabling three behaviors:

- **Passthrough**: return `vec![input]` (possibly modified).
- **Filter**: return `vec![]` to drop the chunk.
- **Fan-out**: return multiple chunks (e.g., splitting a multi-line buffer).

`process` 方法返回 `Vec<DataChunk>`，因此支持三种行为：

- **透传**：返回 `vec![input]`，可以带修改也可以不修改。
- **过滤**：返回 `vec![]`，直接丢弃该数据块。
- **分叉**：返回多个数据块，例如把多行缓冲拆开。

### `engine.rs` -- Pipeline / 流水线引擎

`Pipeline` holds a `Vec<Box<dyn Transform>>` and processes input chunks
sequentially through each transform. At each stage, every output chunk from
the previous transform is fed into the next one:

```
Input Chunk
    |
    v
[Transform 1] --> 0..N chunks
    |
    v
[Transform 2] --> 0..N chunks (for each input from stage 1)
    |
    v
[Transform 3] --> 0..N chunks
    |
    v
Output Chunks
```

The pipeline short-circuits if any stage produces zero chunks, avoiding
unnecessary work.

`Pipeline::from_config` constructs a pipeline from a list of
`PipelineStepConfig` enum variants, mapping each to its concrete transform
implementation.

如果某个阶段产生零个 chunk，pipeline 会直接短路，从而避免不必要的工作。

`Pipeline::from_config` 会从一组 `PipelineStepConfig` 枚举变体构建流水线，
并把每个配置项映射到具体的转换实现。

### Built-in Transforms (`src/pipeline/transforms/`) / 内置转换（`src/pipeline/transforms/`）

| Transform      | File                | Purpose                                              |
|----------------|---------------------|------------------------------------------------------|
| LineBuffer     | `line_buffer.rs`    | Accumulates raw bytes into complete lines            |
| Timestamp      | `timestamp.rs`      | Prepends or enriches chunks with formatted timestamps |
| RegexFilter    | `regex_filter.rs`   | Include or exclude chunks matching a regex pattern   |
| LogLevel       | `log_level.rs`      | Parses log level from content (ESP-IDF, syslog, generic formats) |

| 转换器 | 文件 | 作用 |
|--------|------|------|
| LineBuffer | `line_buffer.rs` | 把原始字节聚合成完整行 |
| Timestamp | `timestamp.rs` | 给 chunk 添加或补充格式化时间戳 |
| RegexFilter | `regex_filter.rs` | 按正则表达式包含或排除 chunk |
| LogLevel | `log_level.rs` | 解析内容中的日志级别（ESP-IDF、syslog、generic） |

### Extending the Pipeline / 扩展流水线

To add a new transform:

1. Create `src/pipeline/transforms/your_transform.rs`
2. Implement `Transform` for your struct
3. Register the module in `src/pipeline/transforms/mod.rs`
4. Add a variant to `PipelineStepConfig` in `src/pipeline/engine.rs`
5. Add a match arm in `Pipeline::from_config`
6. Mirror the config variant in `src/config.rs` for TOML support (note:
   `config.rs` and `engine.rs` maintain separate `PipelineStepConfig` enums
   that must be kept in sync manually)

See [CONTRIBUTING.md](CONTRIBUTING.md) for a detailed walkthrough.

要添加新的转换器：

1. 在 `src/pipeline/transforms/` 下创建新文件，例如 `json_parser.rs`
2. 为你的结构体实现 `Transform`
3. 在 `src/pipeline/transforms/mod.rs` 中注册模块
4. 在 `src/pipeline/engine.rs` 的 `PipelineStepConfig` 中添加新变体
5. 在 `Pipeline::from_config` 中添加匹配分支
6. 在 `src/config.rs` 中同步添加对应的配置变体

注意：`config.rs` 和 `engine.rs` 各自维护一份 `PipelineStepConfig` 枚举，需
要手动保持一致。

## Interface Layer (`src/interface/`) / 接口层（`src/interface/`）

### `cli.rs` -- Command-Line Interface / 命令行接口

Built with `clap` (derive API). Four subcommands:

| Command   | Description                                       |
|-----------|---------------------------------------------------|
| `list`    | Enumerate available serial ports (table or JSON)  |
| `monitor` | Stream serial output in real time with optional regex filter and duration limit |
| `send`    | Write data to a port, optionally waiting for a regex match in the response |
| `serve`   | Start a long-running server (MCP or HTTP mode)    |

The CLI creates a `SessionManager` internally for `monitor` and `send`
commands, opening and closing sessions within the command's lifetime.

CLI 使用 `clap`（derive API）构建，提供四个子命令。`monitor` 和 `send`
都会在命令生命周期内内部创建 `SessionManager`，并在开始和结束时打开/关闭
会话。

### `http.rs` -- HTTP REST API Server / HTTP REST API 服务

Built with `axum` 0.8, the HTTP server mirrors MCP tools as REST endpoints
and serves the embedded Web UI dashboard.

**Key components:**

- **REST endpoints** -- CRUD-style API under `/api/` that maps directly to
  `SessionManager` operations: list ports, create/list/close sessions, read
  lines, write data, send-and-expect, and buffer snapshots.
- **API key middleware** -- Optional authentication via `X-API-Key` header.
  Set with `--api-key` flag or `SERIALINK_API_KEY` environment variable.
  When configured, all `/api/*` routes require the key; `/health` and the Web
  UI are exempt. CORS restrictions are applied when an API key is set.
- **Embedded Web UI** -- The dashboard HTML (`web/index.html`) is embedded
  into the binary at compile time via `include_str!` and served at `/`. This
  eliminates runtime file dependencies and enables single-binary deployment.
- **Health check** -- `GET /health` returns `{"status": "ok"}` without
  authentication, suitable for load balancer probes.
- **Default bind address** -- `127.0.0.1:8600` (local only). Use
  `--bind 0.0.0.0:8600` for network access.

使用 `axum` 0.8 构建的 HTTP 服务把 MCP 工具映射成 REST 端点，并提供内嵌
Web UI 仪表盘。

**核心组件：**

- **REST 端点** -- `/api/` 下的 CRUD 风格接口直接对应 `SessionManager`
  操作：列出串口、创建/列出/关闭会话、读取行、写入数据、send-and-expect
  以及缓冲区快照。
- **API key 中间件** -- 通过 `X-API-Key` 请求头进行可选认证。可以用
  `--api-key` 参数或 `SERIALINK_API_KEY` 环境变量配置。启用后，所有
  `/api/*` 路由都需要 key；`/health` 和 Web UI 例外。配置 key 时还会启用
  CORS 限制。
- **嵌入式 Web UI** -- 仪表盘 HTML（`web/index.html`）在编译时通过
  `include_str!` 嵌入到二进制中，并由 `/` 提供。这消除了运行时文件依赖，
  也便于单文件部署。
- **健康检查** -- `GET /health` 在不认证的情况下返回 `{"status": "ok"}`，
  适合负载均衡器探针。
- **默认绑定地址** -- `127.0.0.1:8600`（仅本地）。需要网络访问时可使用
  `--bind 0.0.0.0:8600`。

### `mcp.rs` -- MCP Server (stdio and SSE transports) / MCP 服务（stdio 和 SSE 传输）

Implements the [Model Context Protocol](https://modelcontextprotocol.io/)
using the `rmcp` crate. Supports two transports:

- **stdio** (`--mcp`) -- For local AI agent integration. The agent launches
  serialink as a child process and communicates over stdin/stdout.
- **SSE** (`--sse`) -- For remote AI agent integration. Uses
  `rmcp::transport::sse_server` to expose MCP tools over HTTP Server-Sent
  Events, allowing remote agents to connect without stdio access.

`SerialinkMcpServer` implements `rmcp::handler::server::ServerHandler` and
exposes 8 tools:

| Tool             | Description                                         |
|------------------|-----------------------------------------------------|
| `list_ports`     | Enumerate available serial ports                    |
| `open_port`      | Open a connection, returns a session ID             |
| `close_port`     | Close an active session                             |
| `read_lines`     | Read the N most recent lines from the ring buffer   |
| `write_data`     | Write a string to the serial port                   |
| `send_and_expect`| Write data, then wait for a regex match in output   |
| `snapshot`       | Dump all buffered lines for a session               |
| `list_sessions`  | List all active sessions with status                |

Each tool validates its parameters, delegates to the `SessionManager`, and
returns JSON-formatted results via `CallToolResult`.

MCP 使用 `rmcp` crate 实现，并支持两种传输：

- **stdio** (`--mcp`) -- 适合本地 AI 代理集成。代理会把 serialink 作为子
  进程启动，并通过 stdin/stdout 通信。
- **SSE** (`--sse`) -- 适合远程 AI 代理集成。它使用
  `rmcp::transport::sse_server` 通过 HTTP Server-Sent Events 暴露 MCP 工具，
  让远程代理无需 stdio 也能连接。

`SerialinkMcpServer` 实现了 `rmcp::handler::server::ServerHandler`，并暴露 8
个工具。每个工具都会校验参数、委托给 `SessionManager`，并通过
`CallToolResult` 返回 JSON 格式结果。

## Request / Data Flow / 请求与数据流

The complete path of serial data through the system:

```
  +------------------+
  |   Serial Port / 串口硬件   |  Physical hardware (USB, UART, Bluetooth)
  +--------+---------+
           |
           | Raw bytes (sync read via serialport-rs)
           |
  +--------+---------+
  | spawn_blocking / 阻塞任务 |  Bridges sync I/O into async tokio runtime
  | Reader Task / 读取任务    |  (JoinHandle stored for shutdown cleanup)
  +--------+---------+
           |
           | Vec<TimestampedLine>
           |
     +-----+------+
     |            |
     v            v
  +------+   +----------+
  | Ring / 环形 |   | broadcast / 广播|   Ring buffer: historical queries (read_lines, snapshot)
  |Buffer / 缓冲|   | channel / 通道 |   Broadcast: real-time streaming to N subscribers
  +------+   +-----+----+
                    |
                    | TimestampedLine (per subscriber)
                    |
        +-----------+-----------+
        |           |           |
        v           v           v
  +----------+ +----------+ +----------+
  |   CLI    | |   MCP    | |   HTTP   |
  | (stdout) | |(stdio/SSE)| | (axum)  |
  +----------+ +----------+ +----------+

  Note: The Data Pipeline Engine exists but is not yet inserted into this
  path. When integrated (Phase 2), it will sit between the broadcast
  channel and the consumer layer, transforming TimestampedLine data into
  processed DataChunks before delivery.
```

**Harness path (test automation):**

```
  Interface (CLI test / MCP run_harness / HTTP POST /api/harness/run)
      │
      ▼
  Harness Engine (dag.rs + executor.rs)
      │ creates isolated
      ▼
  SessionManager (per-run, not shared with server sessions)
      │
      ▼
  SerialConnection (existing, auto_reconnect=false)
```

The harness engine does not participate in the normal real-time data flow
above. It creates its own isolated `SessionManager` and `SerialConnection`
instances for the duration of a test run, then tears them down. This keeps
harness runs from interfering with long-running server sessions.

harness 引擎不参与上面的常规实时数据流。它会在测试运行期间创建自己隔离
的 `SessionManager` 和 `SerialConnection` 实例，运行结束后全部销毁。这样
可以避免 harness 运行与长期运行的服务会话互相干扰。

The current data path is: hardware bytes enter the reader, are buffered,
and are then fanned out to CLI, MCP, and HTTP consumers. The pipeline engine
is still a future insertion point between the broadcast channel and those
consumers.

当前的数据流路径是：硬件字节先进入读取任务，再写入缓冲区，然后分发给
CLI、MCP 和 HTTP 消费者。pipeline 引擎仍然是未来的插入点，位于广播通道
与这些消费者之间。

## Concurrency Model / 并发模型

Serialink is built on the **tokio** async runtime. The concurrency design
addresses three challenges: bridging sync serial I/O into async code,
distributing data to multiple consumers, and managing shared mutable state.

Serialink 基于 **tokio** async runtime。并发设计主要解决三件事：把同步串
口 I/O 接到异步代码里、把数据分发给多个消费者，以及管理共享可变状态。

### spawn_blocking for Serial I/O / 使用 spawn_blocking 处理串口 I/O

The `serialport-rs` crate provides a synchronous API. Serial reads are
performed inside `tokio::task::spawn_blocking` to avoid blocking the async
executor. Each `SerialConnection` runs one background task that loops:
read bytes (blocking) -> parse lines -> push to buffer and broadcast channel.

`serialport-rs` 提供的是同步 API。串口读取会放到
`tokio::task::spawn_blocking` 里执行，以免阻塞 async executor。每个
`SerialConnection` 都会运行一个后台任务，循环执行：读取字节（阻塞）-> 解析
行 -> 写入缓冲区并广播给通道。

### broadcast::channel for Real-Time Distribution / 使用 broadcast::channel 做实时分发

A `tokio::sync::broadcast::channel(256)` distributes received lines to all
active subscribers. Each call to `subscribe()` returns an independent
receiver. This supports:

- CLI monitor output
- MCP `send_and_expect` (subscribes, writes, waits for pattern)
- Planned WebSocket streaming endpoints / 规划中的 WebSocket 流式端点

The broadcast channel was chosen over `mpsc` because multiple independent
consumers need the same data simultaneously. With `mpsc`, each message goes
to exactly one receiver; with `broadcast`, every receiver gets every message.

它支持：

- CLI monitor 输出
- MCP `send_and_expect`（订阅、写入、等待匹配）
- 规划中的 WebSocket 流式端点

之所以选 `broadcast` 而不是 `mpsc`，是因为多个独立消费者需要同时看到同
一份数据。`mpsc` 只能把每条消息发给一个接收者，而 `broadcast` 会把每条
消息发给所有活跃接收者。

### Arc<Mutex<T>> for Shared State / 使用 Arc<Mutex<T>> 保护共享状态

`SerialConnection` uses `Arc<std::sync::Mutex<SharedState>>` to protect the
ring buffer, connection status, and serial port write handle. The mutex is
`std::sync::Mutex`, not `tokio::sync::Mutex`, for two reasons:

1. **It is never held across `.await` points.** The lock is acquired, state
   is read or mutated, and the guard is dropped -- all within synchronous
   code blocks. Inside the reader loop, the lock protects the buffer push;
   inside `write_data`, the lock is acquired within `spawn_blocking` (which
   runs on a blocking thread pool, not the async executor). Neither case
   crosses an await boundary, so the lighter `std::sync::Mutex` is correct.

2. **It works safely inside `spawn_blocking`.** A `tokio::sync::Mutex` must
   be awaited to lock, but `spawn_blocking` closures are synchronous.
   Using `std::sync::Mutex` inside `spawn_blocking` is natural and avoids
   the need to enter a tokio runtime context from a blocking thread.

`SessionManager` uses `Arc<tokio::sync::Mutex<HashMap<...>>>` for its
session map. This is a tokio mutex because session management operations
(`create_session`, `close_session`) are async methods that may hold the
lock while performing `.await` operations (e.g., opening a connection,
closing a session). The two mutex choices are intentionally different.

`SerialConnection` 使用 `Arc<std::sync::Mutex<SharedState>>` 来保护环形缓冲
区、连接状态和串口写句柄。这里用的是 `std::sync::Mutex`，不是
`tokio::sync::Mutex`，原因有两个：

1. **不会跨 `.await` 持锁。** 锁定、读写状态、释放 guard 都在同步代码块
   里完成。读取任务里只在推送缓冲时短暂持锁；`write_data` 则是在
   `spawn_blocking` 中获取锁。这些路径都不会跨 await 边界，因此使用更轻量
   的 `std::sync::Mutex` 是正确的。

2. **适合 `spawn_blocking`。** `tokio::sync::Mutex` 需要通过 `await` 来锁定，
   而 `spawn_blocking` 闭包是同步的。这里直接使用 `std::sync::Mutex` 更自然，
   也避免了在阻塞线程里进入 tokio runtime 上下文。

`SessionManager` 则使用 `Arc<tokio::sync::Mutex<HashMap<...>>>` 维护会话映射。
这里用 tokio mutex 是因为会话管理操作（`create_session`、`close_session`）是
异步方法，可能会在持锁时执行 `.await`（例如打开连接、关闭会话）。这两种
mutex 的选择是刻意不同的。

### CancellationToken for Graceful Shutdown / 使用 CancellationToken 做优雅关闭

Each `SerialConnection` holds a `tokio_util::sync::CancellationToken`. When
`close()` is called, the token is cancelled, causing the background reader
task to exit its loop cleanly. The `close()` method then awaits the stored
`JoinHandle` to ensure the reader task has fully terminated before returning.
This prevents orphaned tasks and ensures serial port handles are released.

当调用 `close()` 时，会取消 token，后台读取任务会因此退出循环。随后
`close()` 会等待保存的 `JoinHandle` 完成，确保读取任务真正结束后再返回。
这样可以避免孤儿任务，也能确保串口句柄被释放。

### Resource Management / 资源管理

- **JoinHandle storage** -- Each `SerialConnection` stores the `JoinHandle`
  for its background reader task in an
  `Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>`. On `close()`, the
  handle is taken and awaited, guaranteeing the reader has exited.
- **close_all() for shutdown** -- `SessionManager::close_all()` iterates
  every active session and calls `close_session()` on each. This is used
  for clean process shutdown (e.g., when the CLI command ends or the MCP
  server stops).
- **Port handle cloned once** -- The serial port is cloned via `try_clone()`
  at open time. The clone goes to the reader task; the original stays in
  `SharedState` for writes. No per-read or per-write cloning occurs after
  initialization.

- **JoinHandle 存储** -- 每个 `SerialConnection` 都会把后台读取任务的
  `JoinHandle` 存进 `Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>`。在
  `close()` 时取出并等待它完成，确保读取任务已经退出。
- **close_all() for shutdown** -- `SessionManager::close_all()` 会遍历所有活
  动会话并逐个关闭。它用于 CLI 命令结束或 MCP 服务停止时的优雅退出。
- **端口句柄只克隆一次** -- 串口句柄只在打开时通过 `try_clone()` 克隆一
  次。克隆件给读取任务使用，原始句柄保留在 `SharedState` 中用于写入。初始
  化之后不会再发生每次读写的重复克隆。

## Security Model / 安全模型

Serialink runs as an MCP server on stdio transport, which means the parent
process (typically an AI agent framework) is the trust boundary. The
following defenses protect against malformed or excessive requests from the
MCP client.

Serialink 作为 stdio 传输的 MCP 服务运行，因此父进程（通常是 AI 代理框
架）就是信任边界。下面这些防御措施用于抵御格式错误或过量的 MCP 请求。

### Port Path Validation / 端口路径校验

`validate_port_path()` in `mcp.rs` enforces an allowlist of device path
prefixes before any port is opened:

- `/dev/tty*` (Linux TTY devices)
- `/dev/serial/*` (Linux serial-by-id symlinks)
- `/dev/cu.*` (macOS serial devices)
- `COM*` (Windows COM ports)

Paths must be absolute (or COMx on Windows). Traversal sequences (`..`) are
rejected. Empty paths are rejected. This prevents an MCP client from
requesting arbitrary file opens.

路径必须是绝对路径（Windows 允许 `COMx`）。路径穿越序列（`..`）会被拒
绝，空路径也会被拒绝。这样可以防止 MCP 客户端请求任意文件打开。

### Session Count Limit / 会话数量上限

`SessionManager` enforces `MAX_SESSIONS = 16`. Any `create_session` call
beyond this limit returns an error. This bounds resource consumption (file
descriptors, reader threads, buffer memory) regardless of how many open
requests the client sends.

`SessionManager` 强制 `MAX_SESSIONS = 16`。超过这个上限的任何
`create_session` 调用都会返回错误。这样可以限制文件描述符、读取线程和缓
冲内存的消耗，不会因为客户端反复打开连接而失控。

### Duplicate Port Prevention / 重复端口防护

`create_session` checks whether the requested port path is already open in
another session. If so, the request is rejected. This prevents conflicting
concurrent access to the same physical device.

`create_session` 会检查请求的端口路径是否已被其他会话打开。如果是，则会
拒绝请求。这样可以避免同一物理设备被并发冲突访问。

### Regex Pattern Limits / 正则表达式限制

All regex patterns (MCP `send_and_expect`, MCP `pattern` fields, CLI
`--filter`) are subject to:

- **Length limit**: 1024 characters maximum for the pattern string.
- **Compiled size limit**: 1 MB for both the NFA and DFA, enforced via
  `RegexBuilder::size_limit(1 << 20)` and `dfa_size_limit(1 << 20)`.

These limits prevent denial-of-service via catastrophic backtracking or
memory-exhausting regex compilation.

所有正则表达式（MCP `send_and_expect`、MCP 的 `pattern` 字段、CLI
`--filter`）都受以下限制：

- **长度限制**：模式字符串最多 1024 个字符
- **编译大小限制**：NFA 和 DFA 都限制为 1 MB，通过 `RegexBuilder::size_limit`
  和 `dfa_size_limit` 实现

这些限制可以防止灾难性回溯和耗尽内存的正则编译。

### Read and Snapshot Caps / 读取与快照上限

- `read_lines`: capped at 1,000 lines per request.
- `snapshot`: capped at 5,000 lines per request (with a default of 500).
- `send_and_expect`: collects at most 200 lines while waiting for a match.

These caps prevent a single tool call from returning unbounded data.

- `read_lines`：每次最多 1,000 行
- `snapshot`：每次最多 5,000 行（默认 500 行）
- `send_and_expect`：等待匹配时最多收集 200 行

这些上限可以防止单个工具调用返回无限增长的数据。

### Write Timeout / 写入超时

`write_data()` wraps the blocking write in a 5-second `tokio::time::timeout`.
If the port is unresponsive (hardware hang, flow control stall), the call
fails rather than blocking indefinitely.

`write_data()` 会把阻塞写入包进 5 秒的 `tokio::time::timeout`。如果端口
没有响应（硬件卡死或流控卡住），调用会失败，而不是无限期阻塞。

### Trust Model / 信任模型

MCP over stdio means Serialink inherits the trust level of its parent
process. The parent launches Serialink as a child, communicates over
stdin/stdout, and is responsible for deciding which tool calls to make.
Serialink does not authenticate MCP clients; it trusts whatever process
holds its stdio file descriptors. In practice, this means the security
boundary is the AI agent framework that spawns Serialink.

Serialink 不会对 MCP 客户端做额外认证；它信任持有自身 stdio 文件描述符
的进程。换句话说，安全边界就是启动 Serialink 的 AI 代理框架。

Serialink 作为 stdio 传输的 MCP 服务，不会再额外验证客户端身份。它默认
信任持有自身 stdio 文件描述符的父进程，因此真正的安全边界就是启动
Serialink 的 AI 代理框架。只要父进程可信，Serialink 就把它视为授权调用
者；如果父进程不可信，那么整个 MCP 通道也不应被当作安全边界。

## Configuration / 配置

Serialink uses TOML configuration files loaded by `src/config.rs`. The
structure mirrors the runtime components:

```toml
[port]
path = "/dev/ttyUSB0"
baud_rate = 115200
data_bits = 8
stop_bits = 1
parity = "none"
auto_reconnect = true
reconnect_interval_ms = 2000

[[pipeline]]
type = "line_buffer"
encoding = "utf-8"

[[pipeline]]
type = "timestamp"
format = "iso8601"

[[pipeline]]
type = "regex_filter"
pattern = "ERROR|WARN"
mode = "include"

[[pipeline]]
type = "log_level_parser"
format = "generic"

[serve]
mcp = true
http = false
port = 8600
```

The `[[pipeline]]` array defines an ordered list of transforms. Each entry
must have a `type` field that maps to a `PipelineStepConfig` variant.
Additional fields are transform-specific configuration.

`[[pipeline]]` 数组定义了一个有序的转换器列表。每个条目都必须包含
`type` 字段，并且该字段要映射到一个 `PipelineStepConfig` 变体。其余字段
则是各个转换器自己的配置项。

The `[serve]` section controls server behavior: enable MCP (stdio), the HTTP
REST API, and the HTTP listen port.

`[serve]` 节控制服务器行为：开启 MCP（stdio）、HTTP REST API，以及 HTTP
监听端口。`http = true` 表示当前对外提供 HTTP REST API 与 Web UI，不表示
已经提供 WebSocket 流式传输；后者仍是规划中的能力。

## Extension Points / 扩展点

### Why broadcast channel (not mpsc) / 为什么用 broadcast channel 而不是 mpsc

Serial data needs to reach multiple consumers simultaneously: the CLI
monitor, MCP tool responses, and planned WebSocket streaming endpoints. A
`broadcast::channel` delivers every message to every active receiver
independently. An `mpsc` channel delivers each message to exactly one
consumer, which would require manual fan-out logic.

串口数据需要同时送到多个消费者：CLI monitor、MCP 工具响应，以及规划中
的 WebSocket 流式端点。`broadcast::channel` 会把每条消息送给所有活跃接
收者；`mpsc` 只会把每条消息送给一个消费者，这样就需要手写 fan-out 逻辑。

### Why spawn_blocking / 为什么使用 spawn_blocking

The `serialport-rs` crate is synchronous and performs blocking system calls.
Running blocking I/O on a tokio async task would starve other tasks sharing
the same executor thread. `spawn_blocking` moves the work to a dedicated
thread pool, keeping the async executor responsive.

`serialport-rs` 是同步库，会执行阻塞系统调用。如果把阻塞 I/O 放到 tokio
异步任务里，会拖慢共享同一个 executor 线程的其他任务。`spawn_blocking`
会把工作放到专用线程池里，从而保持 async executor 响应灵敏。

### Why std::sync::Mutex for SerialConnection / 为什么 SerialConnection 用 std::sync::Mutex

The shared state inside `SerialConnection` is only accessed in synchronous
contexts: the reader loop locks it briefly to push lines into the buffer,
and `write_data` locks it inside `spawn_blocking` to access the port handle.
Neither usage crosses an `.await` point. `std::sync::Mutex` is cheaper than
`tokio::sync::Mutex` (no future allocation, no async scheduling) and is the
correct choice when the critical section is short and synchronous.
`tokio::sync::Mutex` is reserved for cases where the lock genuinely must be
held across await points, such as the `SessionManager` session map.

`SerialConnection` 内部的共享状态只会在同步上下文里访问：读取任务会短暂
持锁，把行写入缓冲区；`write_data` 则是在 `spawn_blocking` 里持锁访问串
口句柄。这两条路径都不会跨 `.await`，所以这里适合使用
`std::sync::Mutex`。它比 `tokio::sync::Mutex` 更轻量，不需要 future 分配，
也不会额外调度异步任务。`tokio::sync::Mutex` 则保留给真正需要跨 await
持锁的地方，例如 `SessionManager` 的会话映射。

### Why ring buffer / 为什么用环形缓冲区

Serialink is designed for long-running monitoring sessions that can generate
thousands of lines per minute. An unbounded buffer would eventually exhaust
memory. The ring buffer (VecDeque capped at 10,000 lines) provides bounded
memory usage while retaining enough history for the `read_lines` and
`snapshot` tools to return useful context.

Serialink 面向的是长时间运行的监控场景，串口一分钟可能产生成千上万行。
如果使用无限缓冲，内存迟早会耗尽。环形缓冲区（`VecDeque`，上限 10,000
行）可以限制内存占用，同时保留足够历史，供 `read_lines` 和 `snapshot`
返回有用上下文。

### Why session-based / 为什么采用会话制

Session-based management (UUID-keyed connections in a SessionManager) serves
several purposes:

- **Multiple concurrent ports**: An agent can open sessions on several ports
  simultaneously and address each by ID.
- **Remote management**: MCP and HTTP clients can open, query, and close
  sessions without managing OS-level file descriptors.
- **Stateless protocols**: Each tool call references a session ID rather than
  carrying connection state, making the MCP and HTTP interfaces naturally
  stateless and idempotent (for reads).
- **Multi-tenant safety**: UUID session IDs are unguessable, preventing
  accidental cross-session interference when multiple agents share one
  Serialink instance.

基于会话的管理（在 `SessionManager` 中使用 UUID 作为连接 key）有几个好处：

- **多个端口并发**：代理可以同时在多个端口上打开会话，并用 ID 分别访问
- **远程管理**：MCP 和 HTTP 客户端可以打开、查询和关闭会话，而不必直接
  管理操作系统文件描述符
- **无状态协议**：每次工具调用都通过 session ID 引用连接状态，让 MCP 和
  HTTP 接口天然保持无状态，并且读取操作天然幂等
- **多租户安全**：UUID session ID 不易猜测，可避免多个代理共享同一个
  Serialink 实例时发生跨会话干扰

## Test Harness Engine (`src/harness/`) / 测试 Harness 引擎（`src/harness/`）

The harness engine provides structured, multi-step test automation for serial
devices. It sits between the interface layer (CLI, MCP, HTTP) and the serial
abstraction layer, orchestrating sequences of actions against one or more
serial ports with dependency ordering, concurrency, and failure handling.

harness 引擎为串口设备提供结构化的多步测试自动化能力。它位于接口层
（CLI、MCP、HTTP）和串口抽象层之间，按照依赖顺序、并发度和失败策略来编
排针对一个或多个串口的操作序列。

### Architecture Overview / 架构概览

The harness is an orchestration layer, not a new abstraction over serial
ports. It reuses existing serialink primitives (`SessionManager`,
`SerialConnection`, `send_and_expect`, `read_lines`) and composes them into
directed acyclic graphs (DAGs) of test steps. Each harness run creates its
own isolated `SessionManager` so that test execution never interferes with
server sessions running in `serve` mode.

```
Interface (CLI test / MCP run_harness / HTTP POST /api/harness/run)
    │
    ▼
Harness Engine (dag.rs + executor.rs)
    │ creates isolated
    ▼
SessionManager (per-run, not shared)
    │
    ▼
SerialConnection (existing, auto_reconnect=false)
```

harness 是编排层，而不是串口的新抽象。它复用已有的 serialink 原语
（`SessionManager`、`SerialConnection`、`send_and_expect`、`read_lines`），
把它们组合成有向无环图（DAG）形式的测试步骤。每次 harness 运行都会创建
自己的隔离 `SessionManager`，测试执行不会干扰 `serve` 模式下正在运行的
服务会话。

### Module Breakdown / 模块结构

#### `schema.rs` -- Types / 类型定义

Defines the configuration and reporting types for harness runs:

| Type              | Purpose                                                       |
|-------------------|---------------------------------------------------------------|
| `HarnessConfig`   | Top-level config: devices list + steps list                   |
| `DeviceConfig`    | Per-device: port path, baud rate, alias for step references   |
| `StepConfig`      | Single step: action, device ref, depends_on, on_fail policy   |
| `OnFail`          | Failure policy enum: `Abort`, `Continue`, `Ignore`            |
| `HarnessReport`   | Final report: per-step results, overall pass/fail, timing     |

`HarnessConfig` is deserialized from TOML. Steps reference devices by alias,
and declare dependencies on other steps by name.

`HarnessConfig` 从 TOML 反序列化。步骤通过别名引用设备，并按名称声明对
其他步骤的依赖。

#### `dag.rs` -- DAG Construction and Scheduling / DAG 构建与调度

Builds a directed acyclic graph from step dependencies and produces an
execution schedule:

1. **Graph construction** -- Each step becomes a node. `depends_on` entries
   become directed edges. Missing dependency references are rejected at
   build time.
2. **Cycle detection** -- Uses Kahn's algorithm (BFS topological sort). If
   the algorithm cannot visit all nodes, a cycle exists and the harness
   refuses to run, returning an error that names the involved steps.
3. **Topological sort** -- Produces a total ordering that respects all
   dependency edges.
4. **Parallel group extraction** -- Steps are grouped by topological depth
   (distance from root nodes). Steps at the same depth have no mutual
   dependencies and can execute concurrently.

从步骤依赖关系构建有向无环图，并生成执行调度：

1. **图构建** -- 每个步骤成为一个节点，`depends_on` 条目变成有向边。构建
   时会拒绝引用不存在的依赖。
2. **环检测** -- 使用 Kahn 算法（BFS 拓扑排序）。如果算法无法访问所有节
   点，说明存在环，harness 拒绝运行并返回涉及的步骤名称。
3. **拓扑排序** -- 生成遵守所有依赖边的全序。
4. **并行组提取** -- 按拓扑深度分组。同一深度的步骤之间没有相互依赖，可
   以并发执行。

#### `executor.rs` -- Action Dispatch and Execution / 动作分派与执行

Executes the DAG schedule against real serial ports:

- **Action dispatch** -- Maps each step's action to an existing serialink
  primitive. The 7 supported actions are:

  | Action             | Maps to                                      |
  |--------------------|----------------------------------------------|
  | `open`             | `SessionManager::create_session`             |
  | `close`            | `SessionManager::close_session`              |
  | `send`             | `SerialConnection::write_data`               |
  | `send_and_expect`  | `SerialConnection::send_and_expect`          |
  | `read_lines`       | `SerialConnection::read_lines` (ring buffer) |
  | `delay`            | `tokio::time::sleep`                         |
  | `assert_contains`  | Regex match against collected output          |

- **Group execution** -- Groups are executed sequentially (depth 0, then
  depth 1, etc.). Within each group, all steps are spawned concurrently
  using `tokio::task::JoinSet`. The executor waits for all tasks in the
  current group to complete before starting the next group.
- **on_fail semantics** -- When a step fails:
  - `Abort` -- Cancel all remaining steps in the current group (via
    `JoinSet::abort_all`) and skip all subsequent groups. The harness
    reports the failure and stops.
  - `Continue` -- Log the failure but proceed with remaining steps.
    Dependents of the failed step still run (optimistic execution).
  - `Ignore` -- Treat the step as passed regardless of outcome. Dependents
    proceed normally.

执行 DAG 调度：

- **动作分派** -- 每个步骤的 action 映射到已有的 serialink 原语，共 7 种。
- **按组执行** -- 组按深度顺序依次执行。同组内的步骤通过
  `tokio::task::JoinSet` 并发运行。当前组全部完成后才开始下一组。
- **on_fail 语义** -- 步骤失败时：
  - `Abort` -- 取消当前组内所有剩余步骤并跳过后续组
  - `Continue` -- 记录失败，继续执行；依赖它的步骤仍然会运行
  - `Ignore` -- 无论结果如何都视为通过

### DAG Execution Model / DAG 执行模型

```
Steps:  A ──► C ──► E
        B ──► D ──►─┘

Depth:  0     1     2

Group 0: [A, B]  -- run concurrently
Group 1: [C, D]  -- run concurrently (after group 0 completes)
Group 2: [E]     -- run alone (after group 1 completes)
```

Groups execute **sequentially**, steps within a group execute
**concurrently**. This maximizes parallelism while respecting dependency
constraints. A step only appears in a group after all of its dependencies
have been placed in earlier groups.

组之间**顺序执行**，组内步骤**并发执行**。这样可以在尊重依赖约束的前提
下最大化并行度。一个步骤只有在其所有依赖都已放入更早的组之后，才会被分
配到当前组。

### Key Design Decisions / 关键设计决策

- **Isolated SessionManager per harness run.** The executor creates a fresh
  `SessionManager` that is not shared with the server's session pool. This
  prevents harness test steps from consuming session slots, colliding with
  active monitoring sessions, or leaving stale sessions after a test run.
  The isolated manager is dropped (and all its sessions closed) when the
  harness run completes.

- **auto_reconnect=false for deterministic testing.** Connections opened by
  the harness disable auto-reconnect. If a device disappears mid-test, the
  step fails immediately rather than silently retrying. This makes test
  results deterministic and avoids masking real hardware failures.

- **Buffer-first read_lines.** When a step reads lines, the executor first
  drains the ring buffer (historical data), then subscribes to the broadcast
  channel for new data if more lines are needed. This ensures that data
  produced by a previous step (e.g., a `send` in the same group) is
  captured even if it arrived before the `read_lines` step started
  executing.

- **Steps map 1:1 to existing serialink primitives.** The harness does not
  introduce new serial I/O abstractions. Every action delegates directly to
  an existing `SerialConnection` or `SessionManager` method. This keeps the
  harness thin and ensures that test behavior matches production behavior
  exactly.

- **每次 harness 运行使用隔离的 SessionManager。** 执行器会创建一个新的
  `SessionManager`，不与服务器的会话池共享。这样可以避免测试步骤占用会话
  名额、与活跃监控会话冲突，或在测试结束后留下残留会话。
- **auto_reconnect=false 保证测试确定性。** harness 打开的连接会禁用自动
  重连。如果设备在测试中途断开，步骤会立即失败而不是静默重试。
- **缓冲区优先的 read_lines。** 读取行时，执行器先从环形缓冲区取历史数
  据，如果还需要更多行再订阅广播通道。这样可以确保前一步骤产出的数据不
  会丢失。
- **步骤与 serialink 原语一一对应。** harness 不引入新的串口 I/O 抽象，
  每个动作都直接委托给已有的方法，确保测试行为与生产行为完全一致。
