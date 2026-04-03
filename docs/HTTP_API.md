[Back to README](../README.md)

# HTTP API and Web UI

serialink exposes an HTTP REST API and an embedded Web UI dashboard for
browser-based workflows and remote access.

## Start the HTTP Server

```bash
# Local only (default)
serialink serve --http

# Network accessible with authentication
serialink serve --http --bind 0.0.0.0:8600 --api-key YOUR_SECRET_KEY
```

Open `http://localhost:8600` for the built-in web dashboard.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check (no auth required) |
| GET | `/api/ports` | List available serial ports |
| POST | `/api/sessions` | Open a serial port session (supports `mode`, `protocol` fields) |
| GET | `/api/sessions` | List active sessions |
| GET | `/api/sessions/{id}/lines?count=50` | Read recent lines (binary lines return base64 + frame) |
| POST | `/api/sessions/{id}/write` | Write data to port (supports `hex` field for binary) |
| POST | `/api/sessions/{id}/send-and-expect` | Send + wait for pattern |
| GET | `/api/sessions/{id}/snapshot?limit=500` | Get buffer snapshot |
| DELETE | `/api/sessions/{id}` | Close session |
| POST | `/api/harness/run` | Run a multi-device test harness | Yes |

## Authentication

Set via `--api-key` flag or `SERIALINK_API_KEY` environment variable.
Include in requests as the `X-API-Key` header.

## Examples

```bash
# List ports with auth
curl -H "X-API-Key: YOUR_KEY" http://localhost:8600/api/ports

# Open a text session
curl -X POST -H "X-API-Key: YOUR_KEY" -H "Content-Type: application/json" \
  -d '{"port_path": "/dev/ttyUSB0", "baud_rate": 115200}' \
  http://localhost:8600/api/sessions

# Open a binary session with Modbus RTU preset
curl -X POST -H "X-API-Key: YOUR_KEY" -H "Content-Type: application/json" \
  -d '{"port_path": "/dev/ttyUSB0", "baud_rate": 9600, "mode": "binary", "protocol": "modbus_rtu"}' \
  http://localhost:8600/api/sessions

# Read lines
curl -H "X-API-Key: YOUR_KEY" http://localhost:8600/api/sessions/{id}/lines?count=20

# Write hex data to a binary session
curl -X POST -H "X-API-Key: YOUR_KEY" -H "Content-Type: application/json" \
  -d '{"hex": "01 03 00 01 00 01 D5 CA"}' \
  http://localhost:8600/api/sessions/{id}/write

# Send and expect
curl -X POST -H "X-API-Key: YOUR_KEY" -H "Content-Type: application/json" \
  -d '{"data": "AT\r\n", "pattern": "OK", "timeout_ms": 5000}' \
  http://localhost:8600/api/sessions/{id}/send-and-expect
```

### Run Test Harness

Run a multi-device test harness that orchestrates open, send, expect, and close
actions across multiple serial ports with dependency ordering.

- **Method:** `POST /api/harness/run`
- **Auth:** Required (`X-API-Key` header)
- **Request body:** JSON with `harness`, `device[]`, `step[]`
- **Response:** `HarnessReport` JSON (always 200 for an executed harness, 422 for validation errors)
- **Limits:** max 16 devices, 256 steps, 300s timeout

```bash
curl -X POST http://localhost:8600/api/harness/run \
  -H "Content-Type: application/json" \
  -H "X-API-Key: YOUR_KEY" \
  -d '{
    "harness": {"name": "smoke", "timeout": 30},
    "device": [{"name": "dut", "port": "/dev/ttyUSB0", "baud_rate": 115200}],
    "step": [
      {"id": "open", "device": "dut", "action": "open_port"},
      {"id": "check", "depends_on": ["open"], "device": "dut", "action": "send_and_expect", "params": {"data": "AT\r\n", "expect": "OK", "timeout": 5}}
    ]
  }'
```

Example response:

```json
{
  "harness": "smoke",
  "passed": true,
  "duration_ms": 1234,
  "steps": [
    {"id": "open", "status": "passed", "duration_ms": 52},
    {"id": "check", "status": "passed", "duration_ms": 1180, "output": "OK"}
  ],
  "devices_used": ["dut"]
}
```

## Security Notes

- HTTP on non-loopback addresses requires `--api-key` (hard error without it).
- API key is transmitted via `X-API-Key` header only (no query parameter -- avoids leaks through logs/referers).
- CORS is restricted to same-origin always (no permissive mode).

---

## HTTP API 与 Web UI（中文）

HTTP REST API 和内嵌 Web UI 面向浏览器工作流和远程访问。

启动 HTTP 服务：

```bash
# 本地模式（默认）
serialink serve --http

# 开放网络访问并启用认证
serialink serve --http --bind 0.0.0.0:8600 --api-key YOUR_SECRET_KEY
```

打开 `http://localhost:8600` 可看到内嵌 Web UI 仪表盘。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/health` | 健康检查，不需要认证 |
| GET | `/api/ports` | 列出可用串口 |
| POST | `/api/sessions` | 打开串口会话（支持 `mode`、`protocol` 字段） |
| GET | `/api/sessions` | 列出活动会话 |
| GET | `/api/sessions/{id}/lines?count=50` | 读取最近日志行（二进制行返回 base64 + 帧信息） |
| POST | `/api/sessions/{id}/write` | 向端口写入数据（支持 `hex` 字段用于二进制） |
| POST | `/api/sessions/{id}/send-and-expect` | 写入数据并等待匹配结果 |
| GET | `/api/sessions/{id}/snapshot?limit=500` | 获取缓冲区快照 |
| DELETE | `/api/sessions/{id}` | 关闭会话 |
| POST | `/api/harness/run` | 运行多设备测试 harness |

**认证：** 可以通过 `--api-key` 参数或 `SERIALINK_API_KEY` 环境变量设置。
请求里需要把密钥放到 `X-API-Key` 请求头。
