# serialink

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Wisdoverse/serialink/ci.yml?branch=main&label=CI)](https://github.com/Wisdoverse/serialink/actions)
[![Crate](https://img.shields.io/crates/v/serialink.svg)](https://crates.io/crates/serialink)
[![Rust](https://img.shields.io/badge/MSRV-1.75-blue.svg)](https://www.rust-lang.org)

**Structured serial port tool for AI agents, automation, and CI/CD.** Turns serial I/O into machine-readable JSON, regex-based expect matching, and MCP/HTTP interfaces.

<!-- AGENT-READABLE-BLOCK: structured metadata for AI agents to quickly understand this tool -->
<details>
<summary>Agent Tool Card</summary>

**What:** CLI + MCP server + HTTP API for serial port automation
**When to use:** Reading/writing serial devices, hardware testing, Modbus monitoring, firmware validation
**Interfaces:** CLI (`serialink`), MCP (9 tools over stdio/SSE), HTTP REST API
**Input formats:** Text commands, hex binary (`--hex`), TOML config
**Output formats:** JSON (structured lines with timestamps), base64 + structured frames (binary mode)
**Session modes:** `text` (line-oriented, default) | `binary` (frame-oriented, Modbus/custom protocols)
**Built-in protocols:** Modbus RTU, Modbus ASCII
**Limits:** 16 concurrent sessions, 1000 lines/read, 30s max expect timeout, 1024-char regex, 65535-byte max frame

### MCP Tools (9)
| Tool | Purpose |
|------|---------|
| `list_ports` | Discover available serial ports |
| `open_port` | Open session (text or binary mode) |
| `close_port` | Close session, release port |
| `read_lines` | Read recent N lines (max 1000) |
| `write_data` | Write text string to port |
| `send_data` | Send hex-encoded binary data |
| `send_and_expect` | Write + regex wait (max 30s) |
| `snapshot` | Dump buffer (max 5000 lines) |
| `list_sessions` | List all active sessions |

### CLI Quick Reference
```
serialink list [--human]
serialink monitor <port> [-b baud] [--human] [-f regex]
serialink send <port> <data> [-e pattern] [-t timeout] [--hex]
serialink serve --mcp|--sse|--http [--bind addr] [--api-key key] [--config file.toml]
```

Exit codes: 0=matched, 1=not matched, 2=connection error, 3=timeout, 4=invalid input, 5=internal error

### Common Patterns
```bash
# Discover ports (JSON array by default)
serialink list

# Send AT command, expect OK — exit code 0 on match, 1 on no match, 3 on timeout
serialink send /dev/ttyUSB0 "AT\r\n" -e "^OK" -t 5
echo $?

# Monitor Modbus RTU device (JSON lines by default)
serialink --config examples/modbus_rtu.toml monitor /dev/ttyUSB0 -b 9600

# Send raw Modbus hex frame
serialink send /dev/ttyUSB0 "01 03 00 00 00 0A C5 CD" --hex -b 9600

# Start MCP server for AI agents
serialink serve --mcp

# Start HTTP API with auth
serialink serve --http --bind 0.0.0.0:8600 --api-key SECRET
```

</details>

<details>
<summary>Agent Tool Card (中文)</summary>

**用途：** 面向 AI 代理、自动化和 CI/CD 的串口工具
**何时使用：** 串口设备读写、硬件测试、Modbus 监控、固件验证
**接口：** CLI (`serialink`)、MCP（9 个工具，stdio/SSE）、HTTP REST API
**输入格式：** 文本命令、十六进制二进制（`--hex`）、TOML 配置
**输出格式：** JSON（带时间戳的结构化行）、base64 + 结构化帧（二进制模式）
**会话模式：** `text`（面向行，默认）| `binary`（面向帧，Modbus/自定义协议）
**内建协议：** Modbus RTU、Modbus ASCII
**限制：** 16 个并发会话、每次读取最多 1000 行、expect 最长 30 秒、正则最长 1024 字符

</details>

## Why serialink?

| Capability | serialink |
|------------|-----------|
| Machine-readable output | Default JSON output |
| Send-and-expect with regex | One command with semantic exit codes |
| CI/CD exit codes | Semantic exit codes (0–5), always active |
| AI agent integration | 9 MCP tools over stdio, plus HTTP REST API |
| Binary protocol support | Modbus RTU/ASCII presets, custom frame parsers, hex send |
| Session management | Up to 16 concurrent sessions |
| Port allowlist validation | Enforced on every open |
| Runtime footprint | Static binary, no Python runtime dependency |

| 能力 | serialink |
|------|-----------|
| 机器可读输出 | 默认 JSON 输出 |
| 正则 send-and-expect | 一个命令即可完成，语义退出码始终生效 |
| CI/CD 退出码 | 语义退出码（0–5），无需额外参数 |
| AI 代理集成 | 通过 stdio 提供 9 个 MCP 工具，并提供 HTTP REST API |
| 二进制协议支持 | Modbus RTU/ASCII 预设、自定义帧解析器、十六进制发送 |
| 会话管理 | 最多 16 个并发会话 |
| 端口白名单校验 | 每次打开端口时都会强制校验 |
| 运行时体积 | 静态二进制，无 Python 运行时依赖 |

## Quick Start

```bash
cargo install serialink

serialink list

serialink send /dev/ttyUSB0 "AT\r\n" -e "^OK" -t 5
echo $?   # 0=matched, 1=not matched, 3=timeout
```

Use `serialink list` to discover ports, then `serialink monitor` for live
output or `serialink serve --mcp` / `serialink serve --http` for agent and
web access.

## Installation

### From crates.io (requires Rust 1.75+)

```bash
cargo install serialink
```

### Build from source

```bash
git clone https://github.com/Wisdoverse/serialink.git
cd serialink
cargo build --release
# Binary at ./target/release/serialink
```

### Pre-built binaries

Release artifacts are published in [GitHub Releases](https://github.com/Wisdoverse/serialink/releases)
when available.

## Documentation

| Guide | Description |
|-------|-------------|
| [MCP Integration](docs/MCP_INTEGRATION.md) | Configure Claude Code, MCP tools reference, agent workflows |
| [HTTP API](docs/HTTP_API.md) | REST endpoints, curl examples, authentication, Web UI |
| [Binary Protocol](docs/BINARY_PROTOCOL.md) | Modbus RTU/ASCII, custom frame parsers, hex send |
| [Configuration](docs/CONFIGURATION.md) | TOML config reference (pipeline, protocol, port settings) |
| [CI/CD Integration](docs/CI_CD.md) | GitHub Actions workflow for hardware-in-the-loop testing |
| [Architecture](ARCHITECTURE.md) | System design, data flow, concurrency model |
| [Security](SECURITY.md) | Trust boundaries, input validation, session isolation |
| [Contributing](CONTRIBUTING.md) | Development setup, PR guidelines, code style |

## Architecture Overview

```
                    +--------------------------+
                    |     Interface Layer      |
                    |  CLI | MCP Server | HTTP |
                    +----------+---------------+
                               |
                    +----------v---------------+
                    |   Data Pipeline Engine   |
                    | timestamp -> regex_filter |
                    | -> log_level -> aggregator|
                    |  (wired via --config)     |
                    +----------+---------------+
                               |
                    +----------v---------------+
                    |   Protocol Layer         |
                    | ReadStrategy | FrameParser|
                    | Modbus decoder | Checksum |
                    +----------+---------------+
                               |
                    +----------v---------------+
                    |  Serial Abstraction Layer |
                    | discovery | port | manager|
                    | auto-reconnect | sessions |
                    +----------+---------------+
                               |
                         /dev/ttyUSB0
                         /dev/ttyACM0
                            ...
```

**Four layers:**

1. **Serial abstraction** -- port discovery, multi-session management (max 16),
   auto-reconnect, device enumeration. `ReadStrategy` trait abstracts
   line-oriented vs. frame-oriented reads.
2. **Protocol layer** -- binary frame parsing (`FrameParser`), protocol
   decoding (Modbus RTU/ASCII), checksum validation, and built-in presets.
3. **Pipeline engine** -- configurable transform chain (timestamps, regex
   filter, log level parser, aggregator). Wired into the reader loop via
   `--config`.
4. **Interface layer** -- CLI for agents and scripts, MCP for AI agents (9
   tools), HTTP REST API for remote access.

See [ARCHITECTURE.md](ARCHITECTURE.md) for implementation details.

## Roadmap

- [x] Core serial abstraction (discovery, connect, read/write, sessions)
- [x] CLI (list, monitor, send) with default JSON output and semantic exit codes
- [x] MCP server (9 tools over stdio transport)
- [x] Wire pipeline engine into CLI and MCP
- [x] HTTP REST API with API key authentication
- [x] Web dashboard for monitoring
- [x] MCP SSE remote transport
- [x] Binary protocol support (Modbus, custom frame parsers)
- [ ] Multi-device orchestration (test harness mode)
- [ ] Pre-built binary releases

## Contributing

Contributions welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Apache License 2.0 -- see [LICENSE](LICENSE).
