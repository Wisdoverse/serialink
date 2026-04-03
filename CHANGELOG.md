# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added / 新增

- **Multi-device test harness** -- DAG-based orchestration of multiple serial devices. TOML config with `[harness]`, `[[device]]`, `[[step]]` sections. Supports parallel execution, dependency ordering, and configurable failure policies (abort/continue/ignore).
  - **多设备测试编排** -- 基于 DAG 的多串口设备编排。TOML 配置包含 `[harness]`、`[[device]]`、`[[step]]` 段。支持并行执行、依赖排序和可配置的失败策略（abort/continue/ignore）。
- New CLI subcommand: `serialink --config harness.toml test`
  - 新增 CLI 子命令：`serialink --config harness.toml test`
- New MCP tool: `run_harness` -- submit test harness config as JSON (10 tools total).
  - 新增 MCP 工具：`run_harness` -- 以 JSON 提交测试编排配置（共 10 个工具）。
- New HTTP endpoint: `POST /api/harness/run`
  - 新增 HTTP 端点：`POST /api/harness/run`
- 7 built-in step actions: `open_port`, `close_port`, `send_and_expect`, `write_data`, `read_lines`, `snapshot`, `delay`.
  - 7 个内建步骤动作：`open_port`、`close_port`、`send_and_expect`、`write_data`、`read_lines`、`snapshot`、`delay`。
- **Pre-built binary releases** -- 6-platform CI/CD pipeline via GitHub Actions (already in release.yml).
  - **预编译二进制发布** -- 通过 GitHub Actions 的 6 平台 CI/CD 流水线（已在 release.yml 中）。

## [0.2.0] - 2026-04-03

### Added / 新增

- **Binary protocol support** with per-session text/binary mode, configurable frame parsing (FixedSize, LengthPrefixed, Delimited, ModbusRtuGap), checksum validation (CRC-16 Modbus, CRC-8, XOR, Sum8, LRC), and built-in Modbus RTU/ASCII decoders.
  - **二进制协议支持**：逐会话文本/二进制模式，可配置帧解析（固定长度、长度前缀、定界符、Modbus RTU 间隔），校验和验证（CRC-16 Modbus、CRC-8、XOR、Sum8、LRC），内建 Modbus RTU/ASCII 解码器。
- **`send_data` MCP tool** for hex-encoded binary send (9 tools total).
  - **`send_data` MCP 工具**：十六进制编码的二进制发送（共 9 个工具）。
- **`--hex` flag** on CLI `send` command for binary data transmission.
  - **`--hex` 标志**：CLI `send` 命令支持二进制数据传输。
- **TOML `[protocol]` config section** for binary frame parsing configuration.
  - **TOML `[protocol]` 配置段**：用于二进制帧解析配置。
- **Built-in presets**: `modbus_rtu` and `modbus_ascii` for zero-config Modbus monitoring.
  - **内建预设**：`modbus_rtu` 和 `modbus_ascii`，零配置 Modbus 监控。
- **Pipeline engine wired** into CLI and MCP via `--config` flag.
  - **Pipeline 引擎接入**：通过 `--config` 标志接入 CLI 和 MCP。

### Changed / 变更

- **BREAKING: Agent-native CLI** — default output format is now JSON. Use `--human` or `--format text` for human-readable output. `--json` flag removed.
  - **破坏性变更：Agent-native CLI** — 默认输出格式为 JSON。使用 `--human` 或 `--format text` 切换为人类可读输出。`--json` 标志已移除。
- **BREAKING: Semantic exit codes always active** — `--exit-code` flag removed. Exit codes: 0=success, 1=pattern_not_matched, 2=connection_error, 3=timeout, 4=invalid_input, 5=internal_error.
  - **破坏性变更：语义退出码始终生效** — `--exit-code` 标志已移除。退出码：0=成功, 1=模式未匹配, 2=连接错误, 3=超时, 4=无效输入, 5=内部错误。
- **Structured error output** — errors on stderr as JSON: `{"error":"...","message":"...","exit_code":N}`.
  - **结构化错误输出** — 错误以 JSON 输出到 stderr：`{"error":"...","message":"...","exit_code":N}`。
- **`list` outputs compact JSON array** by default (was table format).
  - **`list` 默认输出紧凑 JSON 数组**（原为表格格式）。
- **`send` outputs structured JSON result** — `{"status":"matched",...}` or `{"status":"timeout",...}`.
  - **`send` 输出结构化 JSON 结果** — `{"status":"matched",...}` 或 `{"status":"timeout",...}`。
- **Documentation restructured** — README trimmed to ~200 lines with Agent Tool Card; detailed docs split into `docs/` directory.
  - **文档重构** — README 精简至约 200 行并加入 Agent Tool Card；详细文档拆分至 `docs/` 目录。

## [0.1.0] - 2026-03-28 (yanked)

### Added / 新增

- **Serial abstraction layer** with multi-port management, auto-reconnect, and device discovery.
  - **串口抽象层**：支持多端口管理、自动重连和设备发现。
- **Data pipeline engine** as a standalone library with configurable transforms (line buffer, timestamp, regex filter, log level parser). CLI/MCP integration is planned for a later release.
  - **数据流水线引擎**：作为独立库提供，支持 line buffer、timestamp、regex filter 和 log level parser 等转换器。CLI/MCP 集成计划在后续版本提供。
- **CLI interface** with four subcommands: `serialink list`, `serialink monitor`, `serialink send`, and `serialink serve`.
  - **CLI 接口**：提供四个子命令：`serialink list`、`serialink monitor`、`serialink send` 和 `serialink serve`。
- **MCP Server** with 8 tools for AI agent integration.
  - **MCP 服务**：为 AI 代理集成提供 8 个工具。
- **MCP SSE remote transport** (`--sse`) for remote AI agent connections over HTTP.
  - **MCP SSE 远程传输**（`--sse`）：通过 HTTP 为远程 AI 代理连接提供服务。
- **HTTP REST API** (`--http`) mirroring MCP tools as REST endpoints (axum 0.8).
  - **HTTP REST API**（`--http`）：使用 axum 0.8 将 MCP 工具映射为 REST 端点。
- **Web UI dashboard** embedded via `include_str!` for zero-dependency serving.
  - **Web UI 仪表盘**：通过 `include_str!` 嵌入二进制，零额外运行时依赖。
- **Graceful shutdown** with `close_all()` for clean session teardown on exit.
  - **优雅关闭**：通过 `close_all()` 在退出时清理会话。
- **TOML configuration** for port settings and pipeline definition.
  - **TOML 配置**：用于串口设置和 pipeline 定义。
- **Security hardening** with port allowlists, session limits, regex limits, and write/read timeouts.
  - **安全加固**：包括端口白名单、会话上限、正则限制以及读写超时。
- **CI/CD support** via `--exit-code` flag on `send` command.
  - **CI/CD 支持**：`send` 命令提供 `--exit-code` 选项。
- **GitHub Actions** CI workflow (check, test, fmt, clippy) and release workflow (multi-platform binaries).
  - **GitHub Actions**：包含 check、test、fmt、clippy 的 CI 工作流，以及多平台二进制发布工作流。

### Security / 安全

- Addressed deadlock risk by switching `SharedState` to `std::sync::Mutex` so `block_on` is never used inside `spawn_blocking`.
  - 通过将 `SharedState` 改为 `std::sync::Mutex` 规避了死锁风险，避免在 `spawn_blocking` 中使用 `block_on`。
- Fixed resource leaks by storing background reader `JoinHandle`s and awaiting them during close.
  - 通过保存后台读取任务的 `JoinHandle` 并在关闭时等待其结束，修复了资源泄漏。
- Added `SessionManager::close_all()` for graceful shutdown cleanup.
  - 新增 `SessionManager::close_all()`，用于优雅关闭时清理会话。
- Fixed broadcast channel lag handling in MCP `send_and_expect`.
  - 修复了 MCP `send_and_expect` 中对 broadcast channel lag 的处理，lag 不再被当作致命错误。

[Unreleased]: https://github.com/Wisdoverse/serialink/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Wisdoverse/serialink/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Wisdoverse/serialink/releases/tag/v0.1.0
