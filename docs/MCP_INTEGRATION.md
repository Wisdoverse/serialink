[Back to README](../README.md)

# MCP Integration

The [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) lets AI
coding tools call functions directly over stdio. When serialink runs as an MCP
server, tools like Claude Code and Codex can discover ports, open connections,
send commands, and read output without any shell scripting.

## Configure Claude Code

Add to `~/.claude/mcp.json` or your project `.mcp.json`:

```json
{
  "mcpServers": {
    "serialink": {
      "command": "serialink",
      "args": ["serve", "--mcp"]
    }
  }
}
```

### Verify

```
> claude "list serial ports on this machine"
```

Claude Code will call `list_ports` via MCP and return structured JSON.

## MCP Tools

serialink exposes 9 tools over MCP stdio transport:

| Tool | Description |
|------|-------------|
| `list_ports` | Discover available serial ports with metadata |
| `open_port` | Open a connection (supports `mode`, `protocol` params for binary sessions) |
| `close_port` | Close a session and release the port |
| `read_lines` | Read the most recent N lines (default 50, max 1000) |
| `write_data` | Write a string to the port (5s timeout) |
| `send_data` | Send binary data as hex-encoded bytes (e.g. Modbus frames) |
| `send_and_expect` | Write + wait for a regex match (max 30s timeout) |
| `snapshot` | Dump buffered lines (default 500, max 5000) |
| `list_sessions` | List all active sessions |

## Example Agent Workflow

An AI agent debugging an ESP32 might issue these tool calls:

```
1. list_ports         -> discovers /dev/ttyUSB0
2. open_port          -> session_id: "a1b2c3d4-..."
3. write_data         -> sends "AT+RST\r\n"
4. send_and_expect    -> waits for "ready" (up to 30s)
5. read_lines         -> retrieves boot log for analysis
6. close_port         -> releases the port
```

## SSE Transport

For remote AI agents, use the SSE transport:

```bash
serialink serve --sse
```

SSE is restricted to localhost-only (no auth middleware). Hard error if binding
to non-loopback address.

---

## MCP 集成（中文）

MCP 让 AI 编码工具通过 stdio 或远程 SSE 直接调用 serialink，而不需要额外
包装脚本。上面的 Claude Code 配置可以直接复用：

```json
{
  "mcpServers": {
    "serialink": {
      "command": "serialink",
      "args": ["serve", "--mcp"]
    }
  }
}
```

验证时可以让 Claude Code 执行 `list serial ports on this machine`，它会通
过 `list_ports` 返回结构化结果。serialink 当前暴露 9 个 MCP 工具，覆盖端
口发现、会话管理、读写、二进制发送、`send_and_expect` 和快照读取，适合把
串口设备能力稳定地提供给代理使用。

| 工具 | 说明 |
|------|------|
| `list_ports` | 发现可用串口及其元数据 |
| `open_port` | 打开连接（支持 `mode`、`protocol` 参数用于二进制会话） |
| `close_port` | 关闭会话并释放端口 |
| `read_lines` | 读取最近 N 行，默认 50 行，最多 1000 行 |
| `write_data` | 向串口写入字符串，超时 5 秒 |
| `send_data` | 以十六进制编码发送二进制数据（如 Modbus 帧） |
| `send_and_expect` | 写入数据并等待正则匹配，最长 30 秒 |
| `snapshot` | 导出缓冲区内容，默认 500 行，最多 5000 行 |
| `list_sessions` | 列出所有活动会话 |

示例工作流：

```
1. list_ports         -> 发现 /dev/ttyUSB0
2. open_port          -> session_id: "a1b2c3d4-..."
3. write_data         -> 发送 "AT+RST\r\n"
4. send_and_expect    -> 等待 "ready"（最长 30 秒）
5. read_lines         -> 取回启动日志供分析
6. close_port         -> 释放端口
```
