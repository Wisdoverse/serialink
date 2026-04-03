//! End-to-end harness tests using socat virtual serial port pairs.
//!
//! These tests create real PTY pairs via `socat -d -d pty,raw,echo=0 pty,raw,echo=0`,
//! then exercise the full harness executor through them.

use serde_json::json;
use serialink::harness::{executor::run_harness, schema::*};
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Skip test if socat is not available.
macro_rules! require_socat {
    () => {
        match SocatPair::try_new() {
            Some(pair) => pair,
            None => {
                eprintln!("skipping: socat not available");
                return;
            }
        }
    };
}

// ── socat helper ─────────────────────────────────────────────────────

struct SocatPair {
    port_a: String,
    port_b: String,
    child: Child,
}

impl SocatPair {
    /// Create a new socat PTY pair. Returns None if socat is not installed.
    fn try_new() -> Option<Self> {
        let mut child = match Command::new("socat")
            .args(["-d", "-d", "pty,raw,echo=0", "pty,raw,echo=0"])
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return None,
        };

        let stderr = child.stderr.take().unwrap();
        let mut reader = std::io::BufReader::new(stderr);
        let mut ports = Vec::new();

        let start = std::time::Instant::now();
        let mut line = String::new();
        while ports.len() < 2 && start.elapsed() < Duration::from_secs(5) {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) > 0 {
                if let Some(idx) = line.find("/dev/pts/") {
                    ports.push(line[idx..].trim().to_string());
                }
            }
        }

        if ports.len() < 2 {
            let _ = child.kill();
            return None;
        }

        // Give socat a moment to stabilize
        std::thread::sleep(Duration::from_millis(100));

        Some(Self {
            port_a: ports[0].clone(),
            port_b: ports[1].clone(),
            child,
        })
    }
}

impl Drop for SocatPair {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── helpers ──────────────────────────────────────────────────────────

fn device(name: &str, port: &str) -> DeviceConfig {
    DeviceConfig {
        name: name.into(),
        port: port.into(),
        baud_rate: Some(115200),
        protocol: None,
    }
}

fn step(
    id: &str,
    device: Option<&str>,
    action: &str,
    deps: &[&str],
    params: Option<serde_json::Value>,
    on_fail: Option<OnFail>,
) -> StepConfig {
    StepConfig {
        id: id.into(),
        device: device.map(String::from),
        depends_on: if deps.is_empty() {
            None
        } else {
            Some(deps.iter().map(|s| s.to_string()).collect())
        },
        action: action.into(),
        params,
        on_fail,
    }
}

// ── 1. Open and close port ──────────────────────────────────────────

#[tokio::test]
async fn e2e_open_and_close_port() {
    let pair = require_socat!();

    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "e2e_open_close".into(),
            timeout: Some(10),
        },
        devices: vec![device("dut", &pair.port_a)],
        steps: vec![
            step("open", Some("dut"), "open_port", &[], None, None),
            step("close", Some("dut"), "close_port", &["open"], None, None),
        ],
    };

    let report = run_harness(&config).await;

    assert_eq!(
        report.result,
        HarnessResult::Pass,
        "expected Pass, got {:?} — steps: {:#?}",
        report.result,
        report.steps
    );
    assert_eq!(report.steps.len(), 2);
    for s in &report.steps {
        assert_eq!(
            s.result,
            StepResult::Pass,
            "step {:?} failed: {:?}",
            s.id,
            s.error
        );
    }
}

// ── 2. Write data ───────────────────────────────────────────────────

#[tokio::test]
async fn e2e_write_data() {
    let pair = require_socat!();

    // Background reader on port_b — collects whatever arrives
    let port_b = pair.port_b.clone();
    let reader_handle = std::thread::spawn(move || {
        let mut port = serialport::new(&port_b, 115200)
            .timeout(Duration::from_secs(3))
            .open()
            .expect("Failed to open port_b for reading");
        let mut buf = [0u8; 256];
        match port.read(&mut buf) {
            Ok(n) => buf[..n].to_vec(),
            Err(_) => vec![],
        }
    });

    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "e2e_write".into(),
            timeout: Some(10),
        },
        devices: vec![device("dut", &pair.port_a)],
        steps: vec![
            step("open", Some("dut"), "open_port", &[], None, None),
            step(
                "write",
                Some("dut"),
                "write_data",
                &["open"],
                Some(json!({"data": "hello\\n"})),
                None,
            ),
            step("close", Some("dut"), "close_port", &["write"], None, None),
        ],
    };

    let report = run_harness(&config).await;

    assert_eq!(
        report.result,
        HarnessResult::Pass,
        "harness failed: {:#?}",
        report.steps
    );

    let received = reader_handle.join().expect("reader thread panicked");
    assert!(!received.is_empty(), "expected data on port_b, got nothing");
    assert!(
        received.starts_with(b"hello"),
        "expected 'hello', got {:?}",
        String::from_utf8_lossy(&received)
    );
}

// ── 3. Send and expect (full round-trip) ────────────────────────────

#[tokio::test]
async fn e2e_send_and_expect() {
    let pair = require_socat!();

    // Background responder: reads from port_b, writes PONG back
    let port_b = pair.port_b.clone();
    let responder = std::thread::spawn(move || {
        let mut port = serialport::new(&port_b, 115200)
            .timeout(Duration::from_secs(5))
            .open()
            .expect("Failed to open port_b for responding");

        let mut buf = [0u8; 256];
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                // Small delay to simulate device processing
                std::thread::sleep(Duration::from_millis(50));
                port.write_all(b"PONG\r\n").unwrap();
            }
            _ => {}
        }
    });

    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "e2e_send_expect".into(),
            timeout: Some(10),
        },
        devices: vec![device("dut", &pair.port_a)],
        steps: vec![
            step("open", Some("dut"), "open_port", &[], None, None),
            step(
                "ping_pong",
                Some("dut"),
                "send_and_expect",
                &["open"],
                Some(json!({
                    "data": "PING\\r\\n",
                    "expect": "PONG",
                    "timeout": 5
                })),
                None,
            ),
            step(
                "close",
                Some("dut"),
                "close_port",
                &["ping_pong"],
                None,
                None,
            ),
        ],
    };

    let report = run_harness(&config).await;

    assert_eq!(
        report.result,
        HarnessResult::Pass,
        "harness failed: {:#?}",
        report.steps
    );

    let ping_step = report
        .steps
        .iter()
        .find(|s| s.id == "ping_pong")
        .expect("ping_pong step not found");
    assert_eq!(
        ping_step.result,
        StepResult::Pass,
        "send_and_expect failed: {:?}",
        ping_step.error
    );

    // Verify match output
    let output = ping_step.output.as_ref().expect("expected output");
    assert_eq!(output["matched"], true);

    responder.join().expect("responder thread panicked");
}

// ── 4. Read lines from buffer ───────────────────────────────────────

#[tokio::test]
async fn e2e_read_lines_from_buffer() {
    let pair = require_socat!();

    // Writer thread: push 3 lines into port_b -> socat -> port_a
    let port_b = pair.port_b.clone();
    let writer = std::thread::spawn(move || {
        // Small delay so the harness open_port finishes first
        std::thread::sleep(Duration::from_millis(300));
        let mut port = serialport::new(&port_b, 115200)
            .timeout(Duration::from_secs(3))
            .open()
            .expect("Failed to open port_b for writing");
        port.write_all(b"LINE1\nLINE2\nLINE3\n").unwrap();
    });

    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "e2e_read_lines".into(),
            timeout: Some(10),
        },
        devices: vec![device("dut", &pair.port_a)],
        steps: vec![
            step("open", Some("dut"), "open_port", &[], None, None),
            // Delay to allow the writer thread to send data
            step(
                "wait",
                None,
                "delay",
                &["open"],
                Some(json!({"ms": 800})),
                None,
            ),
            step(
                "read",
                Some("dut"),
                "read_lines",
                &["wait"],
                Some(json!({"count": 3, "timeout": 5})),
                None,
            ),
            step("close", Some("dut"), "close_port", &["read"], None, None),
        ],
    };

    let report = run_harness(&config).await;

    assert_eq!(
        report.result,
        HarnessResult::Pass,
        "harness failed: {:#?}",
        report.steps
    );

    let read_step = report
        .steps
        .iter()
        .find(|s| s.id == "read")
        .expect("read step not found");
    assert_eq!(
        read_step.result,
        StepResult::Pass,
        "read_lines failed: {:?}",
        read_step.error
    );

    let output = read_step.output.as_ref().expect("expected output");
    let count = output["count"].as_u64().unwrap();
    assert_eq!(count, 3, "expected 3 lines, got {count}");

    let lines = output["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["content"], "LINE1");
    assert_eq!(lines[1]["content"], "LINE2");
    assert_eq!(lines[2]["content"], "LINE3");

    writer.join().expect("writer thread panicked");
}

// ── 5. Multi-device DAG ─────────────────────────────────────────────

#[tokio::test]
async fn e2e_multi_device_dag() {
    let pair1 = require_socat!();
    let pair2 = require_socat!();

    // Background bridge: read from pair1.port_b, write to pair2.port_b
    // Simulates cross-device data flow (DUT output -> monitor input)
    let p1b = pair1.port_b.clone();
    let p2b = pair2.port_b.clone();
    let bridge = std::thread::spawn(move || {
        let mut src = serialport::new(&p1b, 115200)
            .timeout(Duration::from_secs(5))
            .open()
            .expect("Failed to open pair1.port_b");
        let mut dst = serialport::new(&p2b, 115200)
            .timeout(Duration::from_secs(5))
            .open()
            .expect("Failed to open pair2.port_b");

        let mut buf = [0u8; 256];
        match src.read(&mut buf) {
            Ok(n) if n > 0 => {
                dst.write_all(&buf[..n]).unwrap();
            }
            _ => {}
        }
    });

    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "e2e_multi_device".into(),
            timeout: Some(15),
        },
        devices: vec![
            device("dut", &pair1.port_a),
            device("monitor", &pair2.port_a),
        ],
        steps: vec![
            // Parallel opens — no deps on each other
            step("open_dut", Some("dut"), "open_port", &[], None, None),
            step("open_mon", Some("monitor"), "open_port", &[], None, None),
            // Write on DUT after it's open
            step(
                "write_dut",
                Some("dut"),
                "write_data",
                &["open_dut"],
                Some(json!({"data": "DATA\\n"})),
                None,
            ),
            // Read on monitor — expects the bridged data
            step(
                "read_mon",
                Some("monitor"),
                "read_lines",
                &["open_mon", "write_dut"],
                Some(json!({"count": 1, "timeout": 5})),
                None,
            ),
            step(
                "close_dut",
                Some("dut"),
                "close_port",
                &["write_dut"],
                None,
                None,
            ),
            step(
                "close_mon",
                Some("monitor"),
                "close_port",
                &["read_mon"],
                None,
                None,
            ),
        ],
    };

    let report = run_harness(&config).await;

    assert_eq!(
        report.result,
        HarnessResult::Pass,
        "harness failed: {:#?}",
        report.steps
    );

    // Verify monitor received data
    let read_step = report
        .steps
        .iter()
        .find(|s| s.id == "read_mon")
        .expect("read_mon step not found");
    assert_eq!(
        read_step.result,
        StepResult::Pass,
        "read_mon failed: {:?}",
        read_step.error
    );

    let output = read_step.output.as_ref().expect("expected output");
    let count = output["count"].as_u64().unwrap();
    assert!(
        count >= 1,
        "expected at least 1 line on monitor, got {count}"
    );

    bridge.join().expect("bridge thread panicked");
}

// ── 6. Harness timeout with real port ───────────────────────────────

#[tokio::test]
async fn e2e_harness_timeout_with_real_port() {
    let pair = require_socat!();

    // No responder — send_and_expect will wait for a pattern that never comes
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "e2e_timeout".into(),
            timeout: Some(3),
        },
        devices: vec![device("dut", &pair.port_a)],
        steps: vec![
            step("open", Some("dut"), "open_port", &[], None, None),
            step(
                "wait_forever",
                Some("dut"),
                "send_and_expect",
                &["open"],
                Some(json!({
                    "data": "PING",
                    "expect": "NEVER_COMING",
                    "timeout": 1
                })),
                None,
            ),
        ],
    };

    let report = run_harness(&config).await;

    // The send_and_expect step should fail (timeout), and default on_fail=abort
    // causes HarnessResult::Aborted
    assert!(
        report.result == HarnessResult::Aborted || report.result == HarnessResult::Fail,
        "expected Aborted or Fail, got {:?}",
        report.result
    );

    let timeout_step = report
        .steps
        .iter()
        .find(|s| s.id == "wait_forever")
        .expect("wait_forever step not found");
    assert_eq!(
        timeout_step.result,
        StepResult::Fail,
        "expected Fail, got {:?}",
        timeout_step.result
    );
    assert!(
        timeout_step.error.as_ref().unwrap().contains("timeout"),
        "expected timeout error: {:?}",
        timeout_step.error
    );
}
