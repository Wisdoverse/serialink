# serialink

Structured serial port tool for automation, CI/CD, and AI agents. Rust CLI + MCP Server.

## Commands

```bash
cargo build                              # Build
cargo test                               # Test (326 tests: unit, HTTP API, pipeline, harness)
cargo fmt                                # Format
cargo clippy -- -D warnings -A dead_code # Lint (allow dead_code: some pipeline helpers not yet called externally)
cargo run -- list                        # List serial ports (JSON by default)
cargo run -- --human list                # List serial ports (human-readable table)
cargo run -- --format text list          # Same as --human
cargo run -- monitor /dev/ttyS0 -b 115200           # Monitor port (JSON lines by default)
cargo run -- --human monitor /dev/ttyS0 -b 115200   # Monitor port (human-readable)
cargo run -- send /dev/ttyS0 "AT\r\n" -e "OK"       # Send + expect (JSON result, semantic exit code)
cargo run -- --config pipeline.toml monitor /dev/ttyS0  # Monitor with pipeline transforms
cargo run -- serve --mcp                 # MCP server on stdio (appears to hang — normal)
cargo run -- serve --sse                 # MCP SSE server on 0.0.0.0:8600 (for remote AI agents)
cargo run -- serve --http                # HTTP REST API on 0.0.0.0:8600
cargo run -- serve --http --bind 127.0.0.1:8600 --api-key SECRET  # With auth
cargo run -- serve --http --config pipeline.toml   # HTTP with pipeline transforms
cargo run -- --config modbus_rtu.toml monitor /dev/ttyS0        # Monitor with Modbus RTU decoding (JSON by default)
cargo run -- send /dev/ttyS0 --hex "01 03 00 00 00 0A C5 CD"    # Send hex data
cargo run -- serve --http --config modbus_rtu.toml               # HTTP with binary protocol
cargo run -- --config harness.toml test              # Run test harness (JSON report)
cargo run -- --human --config harness.toml test       # Human-readable test report
```

## Architecture

Three-layer design: `serial/` -> `pipeline/` -> `interface/`.

- `src/serial/port.rs` — Core: SerialConnection, background reader via `spawn_blocking`, ring buffer (10K lines), broadcast channel (256). Uses `std::sync::Mutex` for SharedState (not tokio Mutex — prevents deadlock in spawn_blocking). Port cloned once at open time for reading. Persistent remainder buffer across reads — partial lines are NOT emitted until `\n` arrives. Remainder cleared on reconnect.
- `src/serial/manager.rs` — SessionManager: max 16 sessions, duplicate port prevention, `close_all()` for shutdown cleanup. Uses `tokio::sync::Mutex` (holds lock across .await). `create_session` holds lock for the entire check+open+insert to prevent races.
- `src/serial/discovery.rs` — Port enumeration via serialport-rs.
- `src/pipeline/` — Transform trait + engine. Wired into `reader_loop` via `--config` flag. Pipeline is optional (`Option<Arc<Pipeline>>`), stored in `SessionManager`, and applied to all sessions. `serial/port.rs` has an optional dependency on `pipeline/` for transform processing (pragmatic layer inversion). Pipeline filters apply globally — including to `send_and_expect`, which may time out if the expected pattern is on a filtered line.
- `src/exit_codes.rs` — Semantic exit codes: SUCCESS(0), PATTERN_NOT_MATCHED(1), CONNECTION_ERROR(2), TIMEOUT(3), INVALID_INPUT(4), INTERNAL_ERROR(5).
- `src/interface/cli.rs` — clap subcommands: list, monitor, send, serve, test. Agent-native: JSON output by default, `--human`/`--format text` for human-readable.
- `src/interface/mcp.rs` — MCP Server (rmcp 0.1). 10 tools. Implements `ServerHandler` manually (not `#[tool]` macro). Supports stdio and SSE transports.
- `src/interface/http.rs` — HTTP REST API (axum 0.8). Mirrors MCP tools as REST endpoints. API key auth via `X-API-Key` header only (query param removed — leaks through logs/referers). CORS restricted to same-origin always (no permissive mode).
- `src/config.rs` — TOML config structs. Re-exports `PipelineStepConfig` from `pipeline/engine.rs` (single source of truth — no duplicates).
- `src/protocol/` — Binary protocol support. Frame parsing, checksum validation, Modbus decoders, output formatting.
  - `types.rs` — Core types: FrameConfig, FramingRule, ChecksumType, SessionMode, RawFrame, DecodedFrame, ProtocolDecoder trait, metadata key constants.
  - `checksum.rs` — CRC-16 Modbus, CRC-8, XOR, Sum8, LRC validation/computation.
  - `frame_parser.rs` — Frame parser using Tokio Decoder contract. Supports FixedSize, LengthPrefixed, Delimited, ModbusRtuGap framing.
  - `frame_strategy.rs` — FrameReadStrategy: binary mode read strategy producing base64-encoded TimestampedLine entries.
  - `modbus.rs` — Modbus RTU and ASCII decoders (8 function codes + exceptions).
  - `presets.rs` — Built-in protocol presets (modbus_rtu, modbus_ascii).
  - `format.rs` — Shared binary output formatting for all interfaces.
- `src/serial/read_strategy.rs` — ReadStrategy trait + LineReadStrategy. Abstracts text vs binary reading in reader_loop.
- `src/harness/` — Multi-device test harness with DAG-based orchestration.
  - `schema.rs` — HarnessConfig, DeviceConfig, StepConfig, OnFail, HarnessReport, StepReport types. Serde for TOML (`[[device]]`/`[[step]]` array tables) and JSON (agent API).
  - `dag.rs` — DAG construction from `depends_on` fields. Kahn's algorithm for cycle detection + topological sort. `parallel_groups()` returns step groups by depth for concurrent execution.
  - `executor.rs` — `run_harness()` creates isolated SessionManager per run (no server cross-contamination). Executes DAG groups via `tokio::JoinSet`. 7 actions map 1:1 to serialink primitives: open_port, close_port, send_and_expect, write_data, read_lines (buffer-first then subscribe), snapshot, delay. `auto_reconnect=false` for deterministic testing. On-fail: Abort (cancel all), Continue (optimistic — dependents still run, auto-skip if device open failed), Ignore (omit from report).

## Gotchas

- `serialport` crate needs `libudev-dev` on Linux. We use `default-features = false` to skip it — enumeration works but without USB metadata.
- `ServerHandler` trait methods return `impl Future`, not `async fn`. The `#[allow(clippy::manual_async_fn)]` annotations are required.
- `SharedState` uses `std::sync::Mutex`. Do not change to tokio Mutex — causes deadlock in `write_data` spawn_blocking path.
- `write_data` has 5s timeout wrapping `spawn_blocking`. If serial write hangs, it errors instead of blocking forever.
- `--config` is a global CLI flag that loads a TOML file and builds a pipeline from `[[pipeline]]` steps. The pipeline is stored in `SessionManager` and passed to every `SerialConnection`.
- `LineBufferTransform` will hang if configured at the reader_loop level — `blocking_read_lines` strips `\n` from content, so `LineBufferTransform` (which splits on `\n`) will buffer forever and emit nothing. It is designed for raw byte streams, not post-split lines.
- No `.env` or runtime config needed. All configuration is via CLI flags or `--config` TOML file.
- CLI defaults to JSON output (`--format json`). Use `--human` or `--format text` for human-readable. In JSON mode, tracing is suppressed to `error` level to avoid polluting stderr. Errors are structured JSON on stderr in JSON mode.
- `--exit-code` flag removed from `send`. Semantic exit codes are always used. `send -e` returns exit 0 (matched), 1 (not matched), 3 (timeout). Errors return 2 (connection) or 4 (invalid input).
- `main.rs` builds tokio runtime manually (no `#[tokio::main]`) to control `std::process::exit()` with semantic codes.
- Binary-mode lines skip pipeline transforms in reader_loop. Text transforms (regex_filter, log_level) on base64 content cause silent data loss. Binary-aware transforms are a v2 feature.
- Modbus RTU gap detection is CRC-primary. OS UART buffering makes sub-ms gap detection unreliable. CRC-16 is the authoritative frame boundary. Gap detection uses conservative timeout (T_3.5 * 2, min 5ms).
- `send_and_expect` in binary mode matches regex against `frame_summary` metadata, not base64 content. CLI `--filter` follows the same rule.
- `ProtocolDecoder::decode` receives full frame including delimiters/CRC. Each decoder strips its own transport framing.
- Harness `--config` reuses the global flag. TOML `[harness]`, `[[device]]`, `[[step]]` sections are ignored by non-test commands. Existing `[port]`/`[[pipeline]]`/`[protocol]` sections are ignored by `test`.
- Harness creates its own `SessionManager` per run — never shares with the server's long-lived manager. `close_all()` only affects harness sessions.
- Harness forces `auto_reconnect = false` on all devices. Do not change — deterministic test behavior requires fail-fast on disconnect.
- `read_lines` in harness checks ring buffer first (catches data from predecessor steps), then subscribes for new lines. Order matters — subscribe-only misses data that arrived before the step started.
- TOML `[[step]]` params must use inline tables (`params = { data = "...", expect = "..." }`), not `[step.params]` sub-tables — TOML array-of-tables ordering makes sub-tables unreliable.
- `on_fail = "continue"` is optimistic: dependent steps still execute. Exception: if `open_port` failed and a dependent targets the same device, it's auto-skipped.

## When Adding MCP Tools

Every new MCP tool in `mcp.rs` must:
1. Validate all string inputs (check empty, length limits).
2. Cap numeric params to sane ranges (see existing patterns in `handle_open_port`, `handle_read_lines`).
3. Use `RegexBuilder::new().size_limit(1 << 20).dfa_size_limit(1 << 20).build()` — never bare `Regex::new()`.
4. Add the tool to `tool_definitions()`, `handle_tool()` match, and a `handle_*` method.
5. Use `serde_json::to_string` (compact), not `to_string_pretty`.

## Security Constraints

- Port paths: allowlist `/dev/tty*`, `/dev/serial/*`, `/dev/cu.*`, `COMx`. Reject `..` and relative paths.
- Regex: max 1024 chars, 1MB compiled size.
- read_lines: max 1000. snapshot: default 500, max 5000. send_and_expect: timeout max 30s, collected_lines max 200.
- Sessions: max 16 concurrent, one per port. Baud rate: 1–3,000,000.
- HTTP API key: set via `--api-key` or `SERIALINK_API_KEY` env var. Header only (`X-API-Key`), no query param.
- SSE transport: localhost-only (no auth middleware). Hard error if binding to non-loopback.
- HTTP on non-loopback: requires `--api-key` (hard error without it).
- Serve flags: exactly one of `--mcp`, `--sse`, `--http` required (hard error if zero or multiple).
- Hex input: max 6144 chars (CLI --hex, MCP send_data, HTTP send). Caps at ~3KB decoded bytes.
- Frame size: max_frame_size default 1024, hard cap 65535. Prevents OOM from malformed length fields.
- Harness: max 16 devices, 256 steps, 300s overall timeout, 30s per-step timeout. Step ID max 64 chars (alphanumeric + `_` + `-`). JSON payload max 64KB (HTTP). DAG cycle detection via Kahn's algorithm prevents infinite loops.

## Testing Strategy

326 tests across 5 suites:
- `src/` inline unit tests (135 lib + 135 bin): pipeline transforms, engine, discovery, protocol, harness DAG/executor/schema, config.
- `tests/http_api_test.rs` (37): HTTP API endpoints, auth, validation, harness endpoints.
- `tests/pipeline_transforms.rs` (19): From conversions, TOML deserialization, regex security, transform ordering.
- Next targets: integration tests with real serial ports via `socat -d -d pty,raw,echo=0 pty,raw,echo=0` for virtual port pairs.

## Code Style

- `cargo fmt` before commit.
- `cargo clippy -- -D warnings -A dead_code` must pass.
- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`.
