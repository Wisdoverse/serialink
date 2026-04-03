# Multi-Device Orchestration (Test Harness Mode)

## Overview

Add a DAG-based test harness that orchestrates multiple serial devices simultaneously. Steps are serialink primitives (open_port, send_and_expect, etc.) with dependency-driven parallel execution. Dual interface: TOML files for humans/CI, JSON payloads for AI agents via MCP/HTTP.

## Goals

- CI/CD hardware regression testing with multiple devices
- Agent-native: AI agents dynamically build and submit test DAGs via API
- Zero new abstractions: every step action is an existing serialink operation
- Deterministic: DAG dependencies define execution order, no implicit sequencing

## Non-Goals

- Scripting language or expression evaluator (no capture/assert expressions in v1)
- Long-running daemon mode (harness runs to completion and exits)
- Device hot-plug detection during test execution
- Harness composition (no importing one harness into another)
- Parallel harness runs (one harness per process)

## Architecture

New module `src/harness/` with 4 files:

```
src/harness/
  mod.rs          — Public API: run_harness(HarnessConfig, SessionManager) -> HarnessReport
  schema.rs       — HarnessConfig, DeviceConfig, StepConfig, HarnessReport types
  dag.rs          — DAG construction, cycle detection, topological sort, parallel scheduling
  executor.rs     — Step execution engine: maps action names to SessionManager calls
```

### Layer Integration

```
Interface (CLI test / MCP run_harness / HTTP POST /api/harness/run)
    |
    v
Harness Engine (dag.rs + executor.rs)
    |
    v
SessionManager (existing — manages device sessions)
    |
    v
SerialConnection (existing — per-device read/write)
```

Harness depends on `serial/manager.rs` and `serial/port.rs`. Each harness run creates its **own isolated SessionManager** — never shares with the server's long-lived manager. This prevents conflicts with manually opened sessions and ensures `close_all()` only affects harness sessions.

Per-device protocol override uses `create_session`'s existing `protocol_override` parameter. Pipeline remains global (per-manager).

## Data Structures

### HarnessConfig

```rust
pub struct HarnessConfig {
    pub name: String,
    pub timeout: Option<u64>,          // overall timeout in seconds, default 60
    pub devices: Vec<DeviceConfig>,
    pub steps: Vec<StepConfig>,
}

pub struct DeviceConfig {
    pub name: String,                  // alias: "dut", "monitor"
    pub port: String,                  // /dev/ttyUSB0
    pub baud_rate: Option<u32>,        // default 115200
    pub protocol: Option<ProtocolConfig>, // optional binary protocol
    // auto_reconnect is forced to false by harness executor (deterministic test behavior)
}

pub struct StepConfig {
    pub id: String,
    pub device: Option<String>,        // None for delay steps
    pub depends_on: Option<Vec<String>>,
    pub action: String,                // "open_port", "send_and_expect", etc.
    pub params: Option<serde_json::Value>, // action-specific parameters
    pub on_fail: Option<OnFail>,       // default: Abort
}

pub enum OnFail {
    Abort,     // terminate harness immediately
    Continue,  // mark failed, continue execution (dependents still run)
    Ignore,    // don't record in results, dependents still run
}
```

### OnFail Semantics with DAG Dependencies

When a step fails with `on_fail = "continue"` or `on_fail = "ignore"`:
- Dependent steps **still execute** (optimistic, like GitHub Actions `continue-on-error`)
- Exception: if a failed step was `open_port` and a dependent step targets the same device, the dependent is **auto-skipped** (the device session doesn't exist)
- `on_fail = "abort"` terminates the entire harness immediately, cancelling in-flight steps

### HarnessReport

```rust
pub struct HarnessReport {
    pub harness: String,
    pub result: HarnessResult,         // Pass, Fail, Aborted, Timeout
    pub duration_ms: u64,
    pub devices: Vec<String>,
    pub steps: Vec<StepReport>,
}

pub struct StepReport {
    pub id: String,
    pub action: String,
    pub result: StepResult,            // Pass, Fail, Skipped, Ignored
    pub duration_ms: u64,
    pub error: Option<String>,         // error message if failed
    pub output: Option<serde_json::Value>, // action-specific output
}
```

## TOML Format

Uses the global `--config` flag. Harness sections (`[harness]`, `[[device]]`, `[[step]]`) extend the existing TOML schema. Non-test commands ignore these sections.

```toml
[harness]
name = "smoke_test"
timeout = 60

[[device]]
name = "dut"
port = "/dev/ttyUSB0"
baud_rate = 115200

[[device]]
name = "monitor"
port = "/dev/ttyUSB1"
baud_rate = 9600

[[step]]
id = "open_dut"
device = "dut"
action = "open_port"

[[step]]
id = "open_mon"
device = "monitor"
action = "open_port"

[[step]]
id = "init"
depends_on = ["open_dut"]
device = "dut"
action = "send_and_expect"
on_fail = "abort"
params = { data = "AT\r\n", expect = "OK", timeout = 5 }

[[step]]
id = "check_boot"
depends_on = ["open_mon", "init"]
device = "monitor"
action = "read_lines"
on_fail = "continue"
params = { count = 10, filter = "BOOT_OK", timeout = 10 }
```

Note: use inline tables for `params` (not `[step.params]` sub-tables) to avoid TOML array-of-tables ordering issues.

## Valid Actions

Every action maps 1:1 to an existing serialink operation:

| Action | Device required | Params | Maps to |
|--------|----------------|--------|---------|
| `open_port` | yes | (none, uses device config) | `SessionManager::create_session` |
| `close_port` | yes | (none) | `SessionManager::close_session` |
| `send_and_expect` | yes | `data`, `expect`, `timeout`, `hex` | `SerialConnection::send_and_expect` |
| `write_data` | yes | `data`, `hex` | `SerialConnection::write_data` |
| `read_lines` | yes | `count`, `filter`, `timeout` | `SerialConnection::subscribe` + collect wrapper (see below) |
| `snapshot` | yes | `count` | `SerialConnection::get_buffer_snapshot` |
| `delay` | no | `ms` | `tokio::time::sleep` |

Validation: reject unknown action names at parse time.

### read_lines Implementation Note

`read_lines` is the one action without a direct 1:1 method call. The executor needs a small wrapper (~40 lines) in `executor.rs` that:
1. **First checks the ring buffer** (`get_recent_lines`) for lines matching the filter — catches data that arrived before this step started
2. If not enough matches found, subscribes to the broadcast channel for new lines
3. Collects lines matching the optional `filter` regex
4. Returns when `count` lines collected OR `timeout` seconds elapsed (whichever first)
5. Reports "no_match" in step output if zero lines matched within timeout

This buffer-first-then-subscribe approach is critical: in a DAG, data may arrive during a predecessor step's execution, before this step's subscription starts.

## DAG Execution

1. **Parse**: TOML file or JSON payload -> `HarnessConfig`
2. **Validate**:
   - All device references in steps exist in `[[device]]`
   - All `depends_on` references exist as step ids
   - No cycles (Kahn's algorithm — if remaining nodes after sort > 0, cycle exists)
   - All actions are in the valid set
   - Step ids are unique
3. **Build DAG**: adjacency list from `depends_on`
4. **Execute** via `tokio::JoinSet`:
   - Start all steps with zero in-degree (no dependencies)
   - When a step completes, decrement in-degree of dependents
   - Start any dependent whose in-degree reaches zero
   - On `on_fail = "abort"`: cancel all in-flight steps, skip remaining
   - On `on_fail = "continue"`: record failure, continue scheduling dependents (auto-skip if device session missing)
   - On `on_fail = "ignore"`: don't record, continue scheduling
5. **Overall timeout**: `tokio::time::timeout` wraps the entire execution
6. **Cleanup**: close all devices opened during the run (even on abort/timeout). Cleanup runs on the harness's **isolated** SessionManager (not the server's). Use `tokio::spawn` for async cleanup on abort/timeout paths — sync Drop cannot run async close_all().

### Implementation Notes

- **Device alias registry**: executor maintains a `HashMap<String, String>` mapping device name -> session_id, populated by `open_port` steps.
- **send_and_expect logic**: currently duplicated in CLI/MCP/HTTP handlers. Executor should extract this into a reusable function or call `SerialConnection` methods directly (subscribe + write + match loop).
- **Parallel open_port**: `create_session` holds the manager lock across the full operation. Parallel `open_port` steps will serialize on the lock. This is acceptable — serial port opens are fast, and the lock prevents races.
- **auto_reconnect**: harness executor forces `auto_reconnect = false` for all devices (deterministic test behavior, fail-fast on disconnect).
- **HTTP status**: harness endpoint always returns 200 with structured `HarnessReport` payload (matches existing HTTP pattern where failures are in the JSON body, not HTTP status). Only 422 for validation errors (bad config, DAG cycle).

## Interfaces

### CLI

```bash
serialink --config harness.toml test          # JSON report to stdout
serialink --config harness.toml test --human  # human-readable table
```

New `test` subcommand in clap. Reuses global `--config` flag. Exit codes follow existing semantics:
- 0: all steps passed
- 1: one or more steps failed (PATTERN_NOT_MATCHED)
- 2: connection error
- 3: overall timeout
- 4: invalid config (bad TOML, cycle in DAG, unknown action)
- 5: internal error

### MCP Tool

New tool `run_harness`:
- Input: full harness config as JSON (same structure as TOML, just JSON-serialized)
- Output: `HarnessReport` JSON
- Follows existing MCP tool patterns (input validation, regex limits, etc.)

### HTTP API

New endpoint:
- `POST /api/harness/run` — body is harness config JSON, response is `HarnessReport`
- Requires API key (same auth as other endpoints)
- Response includes appropriate HTTP status (200 pass, 422 validation error, 500 internal)

## Security Constraints

- Max devices per harness: 16 (matches existing MAX_SESSIONS)
- Max steps per harness: 256
- Overall timeout: max 300 seconds (5 minutes)
- Per-step timeout: max 30 seconds (matches existing send_and_expect limit)
- Port path validation: same allowlist as existing (`/dev/tty*`, `/dev/serial/*`, etc.)
- Regex in expect patterns: same 1024 char / 1MB compiled limits
- JSON payload size: max 64KB
- Step id: max 64 chars, alphanumeric + underscore + hyphen

## Testing Strategy

### Unit Tests (in `src/harness/`)

**Schema (schema.rs):**
- Valid TOML with all fields deserializes correctly
- Missing optional fields (timeout, depends_on, params) use defaults
- Invalid on_fail value rejected
- Empty steps/devices arrays handled
- JSON round-trip equivalence with TOML

**DAG (dag.rs):**
- Linear chain: A -> B -> C executes in order
- Diamond dependency: A->B, A->C, B->D, C->D
- No dependencies: all steps parallel
- Single step: trivial case
- Self-cycle detection (A depends on A)
- Two-node cycle (A->B->A)
- Deep cycle (A->B->C->A)
- Missing device reference rejected
- Missing depends_on reference rejected
- Duplicate step ids rejected
- Unknown action name rejected

**Executor (executor.rs):**
- Each action type dispatches to correct SessionManager/SerialConnection method
- read_lines collect wrapper: timeout, count limit, filter regex

### Integration Tests (`tests/harness_test.rs`)

Gate behind `#[ignore]` unless `SERIALINK_INTEGRATION=1` env var (socat required):
- Full harness execution with socat virtual port pairs
- Parallel steps execute concurrently
- Sequential steps respect DAG order
- on_fail=abort cancels in-flight steps
- on_fail=continue records failure + dependents run
- on_fail=continue auto-skips when device session missing
- Overall timeout triggers cleanup
- All devices closed on success, abort, and timeout
- Cleanup guard works on panic (scopeguard/Drop)

### HTTP Tests (extend `tests/http_api_test.rs`)

- POST /api/harness/run with valid payload returns HarnessReport
- POST /api/harness/run without API key returns 401
- POST /api/harness/run with invalid config returns 422
- POST /api/harness/run with cycle in DAG returns 422

### CLI Tests

- `serialink --config harness.toml test` outputs JSON report
- `serialink --config harness.toml test --human` outputs human-readable table
- Exit codes match spec (0 pass, 1 fail, 3 timeout, 4 invalid config)

## Pre-built Binary Releases

`release.yml` already implements 6-platform cross-compilation and GitHub Release creation. Mark as complete in README roadmap. No code changes needed.
