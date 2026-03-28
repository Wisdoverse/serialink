[Back to README](../README.md)

# Binary Protocol Support

serialink supports per-session text or binary mode with frame parsing,
protocol decoding, and built-in Modbus presets. Binary mode is available
across CLI, MCP, and HTTP interfaces.

## Overview

- **Per-session mode**: each session is independently `text` (line-oriented)
  or `binary` (frame-oriented). Mode is set at open time.
- **Frame parsing**: raw byte streams are split into frames using configurable
  framing rules (fixed size, length-prefixed, delimited, Modbus RTU gap).
- **Protocol decoding**: optional decoders (e.g. Modbus RTU) turn raw frames
  into structured fields with human-readable summaries.
- **Checksum validation**: frames are verified against CRC-16/Modbus, CRC-8,
  XOR, Sum8, or LRC checksums.
- **Built-in presets**: `modbus_rtu` and `modbus_ascii` provide ready-to-use
  configurations with correct framing, checksum, and decoder settings.

## CLI Examples

Monitor a Modbus RTU device using a TOML config:

```bash
serialink --config modbus_rtu.toml monitor /dev/ttyUSB0 -b 9600
```

Send a raw hex command:

```bash
serialink send /dev/ttyUSB0 "01 03 00 01 00 01 D5 CA" --hex -b 9600
```

## MCP Agent Workflow for Modbus

An AI agent querying a Modbus RTU sensor might issue:

```
1. list_ports              -> discovers /dev/ttyUSB0
2. open_port               -> port_path: "/dev/ttyUSB0", baud_rate: 9600,
                              protocol: "modbus_rtu"
                           -> session_id: "a1b2c3d4-..."
3. send_data               -> hex: "01 03 00 01 00 01 D5 CA"
                              (Read Holding Register, addr 1, slave 1)
4. read_lines              -> returns decoded Modbus response frame
5. close_port              -> releases the port
```

## HTTP API Examples

Create a binary session with Modbus RTU preset:

```bash
curl -X POST -H "Content-Type: application/json" \
  -d '{"port_path": "/dev/ttyUSB0", "baud_rate": 9600, "mode": "binary", "protocol": "modbus_rtu"}' \
  http://localhost:8600/api/sessions
```

Send hex data:

```bash
curl -X POST -H "Content-Type: application/json" \
  -d '{"hex": "01 03 00 01 00 01 D5 CA"}' \
  http://localhost:8600/api/sessions/{id}/write
```

Read decoded frames:

```bash
curl http://localhost:8600/api/sessions/{id}/lines?count=10
```

## JSON Output Format

Binary-mode lines are returned with base64-encoded raw bytes and optional
structured frame data:

```json
{
  "timestamp": "2026-04-03T10:15:30.123Z",
  "content_base64": "AQMAAQABAAAB1co=",
  "protocol": "modbus_rtu",
  "frame": {
    "summary": "ReadHoldingRegisters slave=1 addr=1 qty=1",
    "fields": {
      "slave_id": 1,
      "function_code": 3,
      "start_address": 1,
      "quantity": 1
    },
    "checksum_valid": true
  }
}
```

## TOML Configuration: `[protocol]` Section

Add a `[protocol]` section to your TOML config file for binary sessions:

```toml
[port]
path = "/dev/ttyUSB0"
baud_rate = 9600

[protocol]
name = "modbus_rtu"
decoder = "modbus_rtu"
frame_timeout_ms = 100
max_frame_size = 256

[protocol.framing]
type = "modbus_rtu_gap"
```

### Framing Rule Types

| Type | Fields | Description |
|------|--------|-------------|
| `fixed_size` | `size` | Fixed-length frames |
| `length_prefixed` | `start`, `length_offset`, `length_size`, `length_endian`, `length_includes_header`, `trailer_size` | Length field in header |
| `delimited` | `start`, `end` | Start/end byte markers |
| `modbus_rtu_gap` | `baud_rate` (optional) | Inter-frame silence gap |

### Checksum Types

`crc16_modbus`, `crc8`, `xor`, `sum8`, `lrc`

## Built-in Presets

| Preset | Framing | Checksum | Decoder | Max Frame |
|--------|---------|----------|---------|-----------|
| `modbus_rtu` | `modbus_rtu_gap` | `crc16_modbus` | `modbus_rtu` | 256 bytes |
| `modbus_ascii` | Delimited (`:` ... `\r\n`) | `lrc` | `modbus_ascii` | 513 bytes |

---

## 二进制协议支持（中文）

serialink 支持逐会话文本或二进制模式，提供帧解析、协议解码和内建 Modbus 预设。

- **逐会话模式**：每个会话可以独立设置为 `text`（面向行）或 `binary`（面向帧），在打开会话时指定。
- **帧解析**：原始字节流通过可配置的帧规则拆分为帧（固定长度、长度前缀、定界符、Modbus RTU 间隔）。
- **协议解码**：可选的解码器（如 Modbus RTU）将原始帧转化为结构化字段，并附带人类可读摘要。
- **校验和验证**：支持 CRC-16/Modbus、CRC-8、XOR、Sum8、LRC 校验。
- **内建预设**：`modbus_rtu` 和 `modbus_ascii` 提供开箱即用的帧、校验和与解码器配置。

| 帧规则类型 | 字段 | 说明 |
|-----------|------|------|
| `fixed_size` | `size` | 固定长度帧 |
| `length_prefixed` | `start`、`length_offset`、`length_size`、`length_endian`、`length_includes_header`、`trailer_size` | 头部中的长度字段 |
| `delimited` | `start`、`end` | 起止字节标记 |
| `modbus_rtu_gap` | `baud_rate`（可选） | 帧间静默间隔 |

**校验和类型：** `crc16_modbus`、`crc8`、`xor`、`sum8`、`lrc`

| 预设 | 帧规则 | 校验和 | 解码器 | 最大帧长 |
|------|--------|--------|--------|----------|
| `modbus_rtu` | `modbus_rtu_gap` | `crc16_modbus` | `modbus_rtu` | 256 字节 |
| `modbus_ascii` | 定界符（`:` ... `\r\n`） | `lrc` | `modbus_ascii` | 513 字节 |
