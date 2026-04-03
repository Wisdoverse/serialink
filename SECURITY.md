# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security vulnerability in serialink, **please do not open a public issue.**

Instead, report it privately:

- **Email:** dev@wisdoverse.com
- **Subject:** [SECURITY] serialink -- brief description

We will acknowledge your report within 48 hours and provide an estimated timeline for a fix. We aim to release security patches within 7 days of confirmation.

## Security Model

Serialink is designed to be safe for use by AI agents and in CI environments
where untrusted input may reach the tool.

### Trust Boundaries

```
Untrusted input (AI agent / CI script)
           |
    +------v-----------+
    | Input validation  |  Port allowlist, regex limits, parameter caps
    +------+-----------+
           |
    +------v-----------+
    | Session manager   |  16-session cap, duplicate port prevention
    +------+-----------+
           |
    +------v-----------+
    | Serial port I/O   |  std::sync::Mutex for SharedState, 5s write timeout
    +------+-----------+
           |
       Hardware
```

### Input Validation

- **Port path allowlist**: Only `/dev/tty*`, `/dev/serial/*`, `/dev/cu.*`, and
  `COMx` paths are accepted. Path traversal (`..`) is rejected.
- **Baud rate**: Must be between 1 and 3,000,000.
- **Regex patterns**: Maximum 1024 characters. Compiled size capped at 1 MB
  (both NFA and DFA) to prevent ReDoS.
- **Timeouts**: `send_and_expect` capped at 30 seconds. Writes time out after
  5 seconds.

### Session Isolation

- Maximum 16 concurrent sessions enforced by `SessionManager`.
- Duplicate port prevention: opening a port that is already held by another
  session returns an error.
- `read_lines` capped at 1000 lines per call. `snapshot` defaults to 500 lines
  (max 5000).
- Per-connection buffer holds up to 10,000 lines in a `VecDeque` behind
  `std::sync::Mutex`.

### Concurrency Design

`SharedState` (buffer, status, port handle) is protected by `std::sync::Mutex`,
not `tokio::sync::Mutex`. This is intentional: the critical sections are short
(buffer push/pop, status read) and never `.await` while the lock is held.
`SessionManager` uses `tokio::sync::Mutex` for the session map because session
creation involves async I/O.

### MCP Server (stdio transport)

The MCP server communicates over stdin/stdout. The trust boundary is the parent
process that launches serialink. Any process that can spawn `serialink serve --mcp`
has full access to all serial port tools. This is by design -- MCP stdio transport
assumes the parent process is trusted.

**Implication:** Do not expose the MCP server to untrusted clients.

### SSE Transport

SSE is restricted to localhost-only binding. serialink returns a hard error if
you attempt to bind to a non-loopback address.

### HTTP API

- HTTP on non-loopback addresses requires `--api-key` (hard error without it).
- API key is transmitted via `X-API-Key` header only (no query parameter --
  avoids leaks through logs/referers).
- CORS is restricted to same-origin always (no permissive mode).

### Serial Port Access

Serialink writes data to serial ports exactly as provided by the caller. There
is no command sanitization or allowlist for serial data. If you connect serialink
to safety-critical hardware, ensure that the calling agent or user is authorized
to send commands to that device.

### Test Harness Constraints

- **Max devices per harness:** 16
- **Max steps per harness:** 256
- **Step ID:** max 64 characters, alphanumeric + underscore + hyphen only
- **Overall timeout:** max 300 seconds
- **Per-step timeout:** max 30 seconds
- **JSON payload size:** max 64 KB (HTTP `POST /api/harness/run` endpoint)
- **Port path validation:** same allowlist as other interfaces (`/dev/tty*`,
  `/dev/serial/*`, `/dev/cu.*`, `COMx`). Path traversal (`..`) rejected.
- **Regex limits:** same as existing (1024 chars, 1 MB compiled size)
- **Isolated SessionManager:** each harness run creates its own `SessionManager`
  instance -- no cross-contamination with server sessions or other harness runs.
- **DAG cycle detection:** dependency graph is validated before execution.
  Cycles are rejected with a hard error, preventing infinite execution loops.

## Dependency Auditing

We recommend running `cargo audit` regularly to check for known vulnerabilities
in dependencies. The `rmcp` crate (v0.1.x) is pre-1.0 and should be monitored
for security updates.
