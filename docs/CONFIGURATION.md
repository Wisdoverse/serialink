[Back to README](../README.md)

# Configuration

serialink supports TOML configuration files for repeatable setups. Load a
config with the `--config` flag:

```bash
serialink --config pipeline.toml monitor /dev/ttyUSB0
serialink --config modbus_rtu.toml monitor /dev/ttyUSB0 -b 9600
serialink serve --http --config pipeline.toml
```

## Full TOML Reference

```toml
[port]
path = "/dev/ttyUSB0"
baud_rate = 115200         # 1 - 3,000,000
data_bits = 8              # 5, 6, 7, or 8
stop_bits = 1              # 1 or 2
parity = "none"            # "none", "odd", "even"
auto_reconnect = true
reconnect_interval_ms = 2000

# Pipeline steps applied in order to serial data via --config flag.

[[pipeline]]
type = "timestamp"
format = "iso8601"         # "iso8601", "unix", "rfc2822"

[[pipeline]]
type = "regex_filter"
pattern = "ERROR|WARN"
mode = "include"           # "include" or "exclude"

[[pipeline]]
type = "log_level_parser"
format = "esp-idf"         # "esp-idf", "syslog", "generic"

[[pipeline]]
type = "aggregator"
window_ms = 500
trigger_pattern = "^\\[\\d+\\]"

# Binary protocol configuration (optional).

[protocol]
name = "modbus_rtu"
decoder = "modbus_rtu"     # "modbus_rtu", "modbus_ascii", or omit for raw
frame_timeout_ms = 100
max_frame_size = 256

[protocol.framing]
type = "modbus_rtu_gap"    # "fixed_size", "length_prefixed", "delimited", "modbus_rtu_gap"

[serve]
mcp = true
http = true                # HTTP server; WebSocket streaming is planned
port = 8600
```

## Field Reference

### `[port]` -- Serial Port Settings

| Field | Description | Default |
|-------|-------------|---------|
| `path` | Serial port path (e.g. `/dev/ttyUSB0`) | -- |
| `baud_rate` | Baud rate, 1 to 3,000,000 | -- |
| `data_bits` | Data bits: 5, 6, 7, or 8 | 8 |
| `stop_bits` | Stop bits: 1 or 2 | 1 |
| `parity` | Parity: `none`, `odd`, `even` | `none` |
| `auto_reconnect` | Auto-reconnect on disconnect | `false` |
| `reconnect_interval_ms` | Reconnect interval in milliseconds | 2000 |

### `[[pipeline]]` -- Transform Steps

Pipeline steps are applied in order to serial data. Available types:

| Type | Fields | Description |
|------|--------|-------------|
| `timestamp` | `format` | Add timestamps (`iso8601`, `unix`, `rfc2822`) |
| `regex_filter` | `pattern`, `mode` | Filter lines by regex (`include` or `exclude`) |
| `log_level_parser` | `format` | Parse log levels (`esp-idf`, `syslog`, `generic`) |
| `aggregator` | `window_ms`, `trigger_pattern` | Aggregate lines within a time window |

### `[protocol]` -- Binary Protocol

| Field | Description | Default |
|-------|-------------|---------|
| `name` | Protocol name | -- |
| `decoder` | Decoder: `modbus_rtu`, `modbus_ascii`, or omit for raw | -- |
| `frame_timeout_ms` | Frame timeout in milliseconds | 500 |
| `max_frame_size` | Maximum frame size in bytes | 1024 |
| `framing` | Framing rule config (see [Binary Protocol](BINARY_PROTOCOL.md)) | -- |

### `[serve]` -- Server Settings

| Field | Description | Default |
|-------|-------------|---------|
| `mcp` | Enable MCP server | `false` |
| `http` | Enable HTTP server | `false` |
| `port` | Listen port | 8600 |

---

## 配置概览（中文）

serialink 支持用 TOML 表达可重复的串口配置和 pipeline 配置，便于在 CI、实
验环境和长期运行的设备上复用同一套参数。通过 `--config` 参数加载配置文件。

| 字段 | 说明 |
|------|------|
| `port.path` | 串口路径，例如 `/dev/ttyUSB0` |
| `port.baud_rate` | 波特率，范围 1 到 3,000,000 |
| `port.data_bits` | 数据位，支持 5 / 6 / 7 / 8 |
| `port.stop_bits` | 停止位，支持 1 或 2 |
| `port.parity` | 校验位，支持 `none` / `odd` / `even` |
| `port.auto_reconnect` | 是否自动重连 |
| `port.reconnect_interval_ms` | 重连间隔，毫秒 |
| `protocol.name` | 协议名称 |
| `protocol.decoder` | 解码器名称：`modbus_rtu`、`modbus_ascii` 或不填（原始帧） |
| `protocol.frame_timeout_ms` | 帧超时，毫秒（默认 500） |
| `protocol.max_frame_size` | 最大帧大小，字节（默认 1024） |
| `protocol.framing` | 帧规则配置（见[二进制协议支持](BINARY_PROTOCOL.md)） |

`[[pipeline]]` 通过 `--config` 标志加载，依次定义原始串口数据经过的转换步骤，
包括时间戳、正则过滤、日志级别解析和聚合器等。`[protocol]` 段用于配置二进制
帧解析。
