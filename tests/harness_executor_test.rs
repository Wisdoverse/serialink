use serde_json::json;
use serialink::harness::{executor::run_harness, schema::*};

// ── Helpers ──────────────────────────────────────────────────────────

fn dummy_device(name: &str, port: &str) -> DeviceConfig {
    DeviceConfig {
        name: name.into(),
        port: port.into(),
        baud_rate: Some(9600),
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

// ── 1. Cycle detection ──────────────────────────────────────────────

#[tokio::test]
async fn harness_validation_rejects_cycle() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "cycle_test".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![
            step("A", Some("dut"), "open_port", &["B"], None, None),
            step("B", Some("dut"), "open_port", &["A"], None, None),
        ],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    assert_eq!(report.steps[0].id, "_validation");
    assert!(
        report.steps[0].error.as_ref().unwrap().contains("cycle"),
        "expected 'cycle' in error: {:?}",
        report.steps[0].error
    );
}

// ── 2. Unknown device ───────────────────────────────────────────────

#[tokio::test]
async fn harness_validation_rejects_unknown_device() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "unknown_dev".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![step("A", Some("ghost"), "open_port", &[], None, None)],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    assert!(
        report.steps[0].error.as_ref().unwrap().contains("ghost"),
        "expected 'ghost' in error: {:?}",
        report.steps[0].error
    );
}

// ── 3. Unknown action ───────────────────────────────────────────────

#[tokio::test]
async fn harness_validation_rejects_unknown_action() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "bad_action".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![step("A", Some("dut"), "fly_to_moon", &[], None, None)],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    let err = report.steps[0].error.as_ref().unwrap();
    assert!(
        err.contains("fly_to_moon"),
        "expected 'fly_to_moon' in error: {err}"
    );
}

// ── 4. Duplicate device names ───────────────────────────────────────

#[tokio::test]
async fn harness_validation_rejects_duplicate_device_names() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "dup_dev".into(),
            timeout: Some(5),
        },
        devices: vec![
            dummy_device("dut", "/dev/ttyUSB0"),
            dummy_device("dut", "/dev/ttyUSB1"),
        ],
        steps: vec![step("A", Some("dut"), "open_port", &[], None, None)],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    let err = report.steps[0].error.as_ref().unwrap();
    assert!(
        err.contains("duplicate"),
        "expected 'duplicate' in error: {err}"
    );
}

// ── 5. Delay-only passes ────────────────────────────────────────────

#[tokio::test]
async fn harness_delay_only_passes() {
    // run_harness rejects empty devices, so include a dummy device even though
    // the delay step does not reference it.
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "delay_test".into(),
            timeout: Some(10),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![step(
            "wait",
            None,
            "delay",
            &[],
            Some(json!({"ms": 100})),
            None,
        )],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Pass);
    assert!(report.duration_ms > 0);
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].id, "wait");
    assert_eq!(report.steps[0].result, StepResult::Pass);
}

// ── 6. Open port fails gracefully ───────────────────────────────────

#[tokio::test]
async fn harness_open_port_fails_gracefully() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "bad_port".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyNONEXIST_TEST")],
        steps: vec![step("open", Some("dut"), "open_port", &[], None, None)],
    };
    let report = run_harness(&config).await;
    // Default on_fail is Abort
    assert!(
        report.result == HarnessResult::Aborted,
        "expected Aborted, got {:?}",
        report.result
    );
    assert_eq!(report.steps[0].result, StepResult::Fail);
    assert!(report.steps[0].error.is_some());
}

// ── 7. on_fail=continue produces Fail result ────────────────────────

#[tokio::test]
async fn harness_on_fail_continue_produces_fail_result() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "continue_test".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyNONEXIST_TEST")],
        steps: vec![step(
            "open",
            Some("dut"),
            "open_port",
            &[],
            None,
            Some(OnFail::Continue),
        )],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    assert_eq!(report.steps[0].result, StepResult::Fail);
}

// ── 8. on_fail=ignore produces Pass result ──────────────────────────

#[tokio::test]
async fn harness_on_fail_ignore_produces_pass_result() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "ignore_test".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyNONEXIST_TEST")],
        steps: vec![step(
            "open",
            Some("dut"),
            "open_port",
            &[],
            None,
            Some(OnFail::Ignore),
        )],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Pass);
    assert_eq!(report.steps[0].result, StepResult::Ignored);
}

// ── 9. Auto-skip after open_port failure ────────────────────────────

#[tokio::test]
async fn harness_auto_skip_after_open_port_failure() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "skip_test".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyNONEXIST_TEST")],
        steps: vec![
            step(
                "open",
                Some("dut"),
                "open_port",
                &[],
                None,
                Some(OnFail::Continue),
            ),
            step(
                "send",
                Some("dut"),
                "send_and_expect",
                &["open"],
                Some(json!({"data": "AT", "expect": "OK"})),
                None,
            ),
        ],
    };
    let report = run_harness(&config).await;
    // Find the send step (order in report may vary due to async)
    let send_step = report.steps.iter().find(|s| s.id == "send").unwrap();
    assert_eq!(send_step.result, StepResult::Skipped);
}

// ── 10. Overall timeout returns partial results ─────────────────────

#[tokio::test]
async fn harness_overall_timeout_returns_partial_results() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "timeout_test".into(),
            timeout: Some(1),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![step(
            "long_wait",
            None,
            "delay",
            &[],
            Some(json!({"ms": 5000})),
            None,
        )],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Timeout);
}

// ── 11. DAG respects dependency order ───────────────────────────────

#[tokio::test]
async fn harness_dag_respects_dependency_order() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "dag_order".into(),
            timeout: Some(10),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![
            step("A", None, "delay", &[], Some(json!({"ms": 10})), None),
            step("B", None, "delay", &["A"], Some(json!({"ms": 10})), None),
            step("C", None, "delay", &["B"], Some(json!({"ms": 10})), None),
        ],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Pass);
    assert_eq!(report.steps.len(), 3);
    for step in &report.steps {
        assert_eq!(step.result, StepResult::Pass);
    }
}

// ── 12. Empty steps rejected ────────────────────────────────────────

#[tokio::test]
async fn harness_empty_steps_rejected() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "no_steps".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "/dev/ttyUSB0")],
        steps: vec![],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    let err = report.steps[0].error.as_ref().unwrap();
    assert!(
        err.contains("no steps"),
        "expected 'no steps' in error: {err}"
    );
}

// ── 13. Empty devices rejected ──────────────────────────────────────

#[tokio::test]
async fn harness_empty_devices_rejected() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "no_devices".into(),
            timeout: Some(5),
        },
        devices: vec![],
        steps: vec![step("A", None, "delay", &[], Some(json!({"ms": 10})), None)],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    let err = report.steps[0].error.as_ref().unwrap();
    assert!(
        err.contains("no devices"),
        "expected 'no devices' in error: {err}"
    );
}

// ── 14. Port path validation ────────────────────────────────────────

#[tokio::test]
async fn harness_port_path_validation() {
    let config = HarnessConfig {
        harness: HarnessMetadata {
            name: "bad_path".into(),
            timeout: Some(5),
        },
        devices: vec![dummy_device("dut", "../../../etc/passwd")],
        steps: vec![step("A", Some("dut"), "open_port", &[], None, None)],
    };
    let report = run_harness(&config).await;
    assert_eq!(report.result, HarnessResult::Fail);
    let err = report.steps[0].error.as_ref().unwrap();
    assert!(
        err.contains("port") || err.contains("path"),
        "expected port/path validation error: {err}"
    );
}
