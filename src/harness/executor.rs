use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use regex::RegexBuilder;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{error, warn};

use crate::harness::dag;
use crate::harness::schema::*;
use crate::protocol::format;
use crate::serial::manager::SessionManager;
use crate::serial::port::{PortConfig, SerialConnection, TimestampedLine};
use crate::serial::validate_port_path;

const MAX_OVERALL_TIMEOUT_S: u64 = 300;
const DEFAULT_OVERALL_TIMEOUT_S: u64 = 60;
const MAX_STEP_TIMEOUT_S: u64 = 30;
const MAX_HEX_INPUT_LEN: usize = 6144;
const MAX_TEXT_INPUT_LEN: usize = 6144;
const DEFAULT_READ_COUNT: usize = 10;
const MAX_READ_COUNT: usize = 1000;
const DEFAULT_READ_TIMEOUT_S: u64 = 5;
const DEFAULT_SNAPSHOT_COUNT: usize = 500;
const MAX_SNAPSHOT_COUNT: usize = 5000;

/// Run a full harness test plan and return a report.
pub async fn run_harness(config: &HarnessConfig) -> HarnessReport {
    let start = Instant::now();
    let harness_name = config.harness.name.clone();
    let device_names: Vec<String> = config.devices.iter().map(|d| d.name.clone()).collect();

    // 0. Reject empty configs early
    if config.devices.is_empty() {
        return make_error_report(
            &harness_name,
            &device_names,
            start,
            HarnessResult::Fail,
            "_validation",
            "harness has no devices defined",
        );
    }
    if config.steps.is_empty() {
        return make_error_report(
            &harness_name,
            &device_names,
            start,
            HarnessResult::Fail,
            "_validation",
            "harness has no steps defined",
        );
    }

    // 0b. Duplicate device name detection (before building any maps)
    {
        let mut seen_names = HashSet::new();
        for dev in &config.devices {
            if !seen_names.insert(&dev.name) {
                return make_error_report(
                    &harness_name,
                    &device_names,
                    start,
                    HarnessResult::Fail,
                    "_validation",
                    &format!("duplicate device name {:?}", dev.name),
                );
            }
        }
    }

    // 1. Validate DAG
    let device_set: HashSet<String> = config.devices.iter().map(|d| d.name.clone()).collect();
    let sorted_steps = match dag::validate_and_sort(&config.steps, &device_set) {
        Ok(s) => s,
        Err(e) => {
            return make_error_report(
                &harness_name,
                &device_names,
                start,
                HarnessResult::Fail,
                "_validation",
                &format!("DAG validation failed: {e}"),
            );
        }
    };

    // 2. Validate all device port paths
    for dev in &config.devices {
        if let Err(e) = validate_port_path(&dev.port) {
            return make_error_report(
                &harness_name,
                &device_names,
                start,
                HarnessResult::Fail,
                "_validation",
                &format!("invalid port path for device {:?}: {e}", dev.name),
            );
        }
    }

    // 3. Isolated session manager
    let manager = SessionManager::new(None, None);

    // 4. Device config lookup
    let device_configs: HashMap<String, &DeviceConfig> =
        config.devices.iter().map(|d| (d.name.clone(), d)).collect();

    // 5. Device alias -> session_id map
    let sessions: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));

    // Track which devices had open_port failures (for auto-skip)
    let failed_devices: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // 6. Compute parallel groups
    let groups = dag::parallel_groups(&sorted_steps);

    // Build step lookup by id
    let step_map: HashMap<String, StepConfig> = sorted_steps
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    // 7. Execute groups with overall timeout
    let timeout_s = config
        .harness
        .timeout
        .unwrap_or(DEFAULT_OVERALL_TIMEOUT_S)
        .min(MAX_OVERALL_TIMEOUT_S);

    // Shared reports so timeout branch can read partial results
    let shared_reports: Arc<Mutex<Vec<StepReport>>> = Arc::new(Mutex::new(Vec::new()));

    let (step_reports, overall_result) = match tokio::time::timeout(
        Duration::from_secs(timeout_s),
        execute_groups(
            &groups,
            &step_map,
            &device_configs,
            &manager,
            &sessions,
            &failed_devices,
            &shared_reports,
        ),
    )
    .await
    {
        Ok((result, _was_aborted)) => {
            let reports = shared_reports.lock().await.clone();
            (reports, result)
        }
        Err(_) => {
            warn!(
                "harness overall timeout after {}s, returning partial results",
                timeout_s
            );
            let reports = shared_reports.lock().await.clone();
            (reports, HarnessResult::Timeout)
        }
    };

    // 11. Cleanup — always
    manager.close_all().await;

    HarnessReport {
        harness: harness_name,
        result: overall_result,
        duration_ms: start.elapsed().as_millis() as u64,
        devices: device_names,
        steps: step_reports,
    }
}

/// Execute all groups sequentially, running steps within each group concurrently.
/// Pushes reports into `shared_reports` as they complete.
/// Returns (overall_result, was_aborted).
async fn execute_groups(
    groups: &[Vec<String>],
    step_map: &HashMap<String, StepConfig>,
    device_configs: &HashMap<String, &DeviceConfig>,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    failed_devices: &Arc<Mutex<HashSet<String>>>,
    shared_reports: &Arc<Mutex<Vec<StepReport>>>,
) -> (HarnessResult, bool) {
    let mut overall_result = HarnessResult::Pass;

    for group in groups {
        let mut join_set = JoinSet::new();

        for step_id in group {
            let step = match step_map.get(step_id) {
                Some(s) => s.clone(),
                None => {
                    error!(step_id = %step_id, "step not found in step_map");
                    let mut reports = shared_reports.lock().await;
                    reports.push(StepReport {
                        id: step_id.clone(),
                        action: "unknown".to_string(),
                        result: StepResult::Fail,
                        duration_ms: 0,
                        error: Some(format!("step {:?} not found in step_map", step_id)),
                        output: None,
                    });
                    overall_result = HarnessResult::Fail;
                    continue;
                }
            };

            // Auto-skip: if this step requires a device whose open_port failed
            if let Some(ref dev_name) = step.device {
                if step.action != "open_port" {
                    let failed = failed_devices.lock().await;
                    if failed.contains(dev_name) {
                        let mut reports = shared_reports.lock().await;
                        reports.push(StepReport {
                            id: step.id.clone(),
                            action: step.action.clone(),
                            result: StepResult::Skipped,
                            duration_ms: 0,
                            error: Some(format!("device {:?} failed to open", dev_name)),
                            output: None,
                        });
                        continue;
                    }
                }
            }

            let manager = manager.clone();
            let sessions = sessions.clone();
            let failed_devices = failed_devices.clone();
            let device_configs: HashMap<String, DeviceConfig> = device_configs
                .iter()
                .map(|(k, v)| (k.clone(), (*v).clone()))
                .collect();

            join_set.spawn(async move {
                let step_start = Instant::now();
                let result = execute_step(&step, &manager, &sessions, &device_configs).await;

                let report = match result {
                    Ok(output) => StepReport {
                        id: step.id.clone(),
                        action: step.action.clone(),
                        result: StepResult::Pass,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        error: None,
                        output,
                    },
                    Err(e) => {
                        // If open_port failed, mark device as failed
                        if step.action == "open_port" {
                            if let Some(ref dev_name) = step.device {
                                let mut failed = failed_devices.lock().await;
                                failed.insert(dev_name.clone());
                            }
                        }
                        StepReport {
                            id: step.id.clone(),
                            action: step.action.clone(),
                            result: StepResult::Fail,
                            duration_ms: step_start.elapsed().as_millis() as u64,
                            error: Some(e.to_string()),
                            output: None,
                        }
                    }
                };

                (report, step.on_fail.clone())
            });
        }

        // Collect results from this group
        let mut should_abort = false;
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((mut report, on_fail)) => {
                    if report.result == StepResult::Fail {
                        let is_default = on_fail.is_none();
                        let on_fail = on_fail.unwrap_or(OnFail::Abort);
                        match on_fail {
                            OnFail::Abort => {
                                overall_result = HarnessResult::Aborted;
                                should_abort = true;
                                if is_default {
                                    error!(step = %report.id, "step failed with on_fail=abort (default)");
                                } else {
                                    error!(step = %report.id, "step failed with on_fail=abort (explicit)");
                                }
                            }
                            OnFail::Continue => {
                                overall_result = HarnessResult::Fail;
                                warn!(step = %report.id, "step failed with on_fail=continue");
                            }
                            OnFail::Ignore => {
                                report.result = StepResult::Ignored;
                                warn!(step = %report.id, "step failed but ignored (on_fail=ignore)");
                            }
                        }
                    }
                    let mut reports = shared_reports.lock().await;
                    reports.push(report);
                }
                Err(e) => {
                    // JoinError — task panicked or was cancelled
                    error!("step task panicked or cancelled: {e}");
                    overall_result = HarnessResult::Fail;
                    let mut reports = shared_reports.lock().await;
                    reports.push(StepReport {
                        id: "_task_panic".to_string(),
                        action: "unknown".to_string(),
                        result: StepResult::Fail,
                        duration_ms: 0,
                        error: Some(format!("task panicked or cancelled: {e}")),
                        output: None,
                    });
                }
            }
        }

        if should_abort {
            // Cancel remaining groups
            break;
        }
    }

    let was_aborted = overall_result == HarnessResult::Aborted;
    (overall_result, was_aborted)
}

/// Execute a single step action.
async fn execute_step(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    device_configs: &HashMap<String, DeviceConfig>,
) -> Result<Option<serde_json::Value>> {
    match step.action.as_str() {
        "open_port" => action_open_port(step, manager, sessions, device_configs).await,
        "close_port" => action_close_port(step, manager, sessions).await,
        "write_data" => action_write_data(step, manager, sessions, device_configs).await,
        "send_and_expect" => action_send_and_expect(step, manager, sessions, device_configs).await,
        "read_lines" => action_read_lines(step, manager, sessions, device_configs).await,
        "snapshot" => action_snapshot(step, manager, sessions, device_configs).await,
        "delay" => action_delay(step).await,
        other => Err(anyhow!("unknown action: {other}")),
    }
}

// ── Action implementations ────────────────────────────────────────────

async fn action_open_port(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    device_configs: &HashMap<String, DeviceConfig>,
) -> Result<Option<serde_json::Value>> {
    let dev_name = step
        .device
        .as_ref()
        .ok_or_else(|| anyhow!("open_port requires a device"))?;
    let dev_cfg = device_configs
        .get(dev_name)
        .ok_or_else(|| anyhow!("unknown device: {dev_name}"))?;

    let port_config = PortConfig {
        baud_rate: dev_cfg.baud_rate.unwrap_or(115200),
        auto_reconnect: false,
        ..Default::default()
    };

    let session_id = manager
        .create_session(dev_cfg.port.clone(), port_config, dev_cfg.protocol.clone())
        .await?;

    let mut sess = sessions.lock().await;
    sess.insert(dev_name.clone(), session_id.clone());

    Ok(Some(serde_json::json!({ "session_id": session_id })))
}

async fn action_close_port(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
) -> Result<Option<serde_json::Value>> {
    let dev_name = step
        .device
        .as_ref()
        .ok_or_else(|| anyhow!("close_port requires a device"))?;

    let session_id = {
        let sess = sessions.lock().await;
        sess.get(dev_name)
            .cloned()
            .ok_or_else(|| anyhow!("no open session for device {:?}", dev_name))?
    };

    manager.close_session(&session_id).await?;

    {
        let mut sess = sessions.lock().await;
        sess.remove(dev_name);
    }

    Ok(None)
}

async fn action_write_data(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    device_configs: &HashMap<String, DeviceConfig>,
) -> Result<Option<serde_json::Value>> {
    let (conn, _session_id) = get_connection(step, manager, sessions, device_configs).await?;
    let data = extract_data_bytes(step.params.as_ref())?;
    conn.write_data(&data).await?;
    Ok(Some(serde_json::json!({ "bytes_written": data.len() })))
}

async fn action_send_and_expect(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    device_configs: &HashMap<String, DeviceConfig>,
) -> Result<Option<serde_json::Value>> {
    let (conn, _session_id) = get_connection(step, manager, sessions, device_configs).await?;
    let params = step
        .params
        .as_ref()
        .ok_or_else(|| anyhow!("send_and_expect requires params"))?;

    let data = extract_data_bytes(Some(params))?;

    let expect_pattern = params
        .get("expect")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("send_and_expect requires 'expect' param"))?;

    if expect_pattern.len() > 1024 {
        return Err(anyhow!("expect pattern too long (max 1024 chars)"));
    }

    let regex = RegexBuilder::new(expect_pattern)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|e| anyhow!("invalid expect regex: {e}"))?;

    let timeout_dur = params
        .get("timeout")
        .and_then(|v| v.as_f64())
        .map(|t| Duration::from_secs_f64(t.clamp(0.01, MAX_STEP_TIMEOUT_S as f64)))
        .unwrap_or(Duration::from_secs(5));

    // Subscribe before writing so we don't miss responses
    let mut rx = conn.subscribe();

    conn.write_data(&data).await?;

    let mut collected: Vec<TimestampedLine> = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_dur;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!(
                "timeout waiting for pattern {:?} after {:.2}s (collected {} lines)",
                expect_pattern,
                timeout_dur.as_secs_f64(),
                collected.len()
            ));
        }

        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(line)) => {
                let content = format::matchable_content(&line);
                let matched = regex.is_match(content);
                collected.push(line);
                if matched {
                    return Ok(Some(serde_json::json!({
                        "matched": true,
                        "lines_checked": collected.len(),
                    })));
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                warn!(step_id = %step.id, lagged = n, "broadcast lagged, dropped messages");
                continue;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Err(anyhow!(
                    "broadcast channel closed while waiting for pattern"
                ));
            }
            Err(_) => {
                return Err(anyhow!(
                    "timeout waiting for pattern {:?} after {:.2}s (collected {} lines)",
                    expect_pattern,
                    timeout_dur.as_secs_f64(),
                    collected.len()
                ));
            }
        }
    }
}

async fn action_read_lines(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    device_configs: &HashMap<String, DeviceConfig>,
) -> Result<Option<serde_json::Value>> {
    let (conn, _session_id) = get_connection(step, manager, sessions, device_configs).await?;
    let params = step.params.as_ref();

    let count = params
        .and_then(|p| p.get("count"))
        .and_then(|v| v.as_u64())
        .map(|c| (c as usize).min(MAX_READ_COUNT))
        .unwrap_or(DEFAULT_READ_COUNT);

    let timeout_dur = params
        .and_then(|p| p.get("timeout"))
        .and_then(|v| v.as_f64())
        .map(|t| Duration::from_secs_f64(t.clamp(0.01, MAX_STEP_TIMEOUT_S as f64)))
        .unwrap_or(Duration::from_secs(DEFAULT_READ_TIMEOUT_S));

    let filter_regex = if let Some(pattern) = params
        .and_then(|p| p.get("filter"))
        .and_then(|v| v.as_str())
    {
        if pattern.len() > 1024 {
            return Err(anyhow!("filter pattern too long (max 1024 chars)"));
        }
        Some(
            RegexBuilder::new(pattern)
                .size_limit(1 << 20)
                .dfa_size_limit(1 << 20)
                .build()
                .map_err(|e| anyhow!("invalid filter regex: {e}"))?,
        )
    } else {
        None
    };

    let mut collected: Vec<TimestampedLine> = Vec::new();

    // Buffer-first: check existing lines
    let recent = conn.get_recent_lines(count).await;
    for line in recent {
        if collected.len() >= count {
            break;
        }
        if let Some(ref re) = filter_regex {
            if !re.is_match(format::matchable_content(&line)) {
                continue;
            }
        }
        collected.push(line);
    }

    // If we already have enough, return immediately
    if collected.len() >= count {
        let lines: Vec<serde_json::Value> = collected
            .iter()
            .map(|l| serde_json::json!({ "timestamp": l.timestamp.to_rfc3339(), "content": l.content }))
            .collect();
        return Ok(Some(
            serde_json::json!({ "lines": lines, "count": lines.len() }),
        ));
    }

    // Subscribe for new lines
    let mut rx = conn.subscribe();
    let deadline = tokio::time::Instant::now() + timeout_dur;

    loop {
        if collected.len() >= count {
            break;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(line)) => {
                if let Some(ref re) = filter_regex {
                    if !re.is_match(format::matchable_content(&line)) {
                        continue;
                    }
                }
                collected.push(line);
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                warn!(step_id = %step.id, lagged = n, "broadcast lagged, dropped messages");
                continue;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            Err(_) => break, // timeout — return what we have
        }
    }

    let lines: Vec<serde_json::Value> = collected
        .iter()
        .map(|l| serde_json::json!({ "timestamp": l.timestamp.to_rfc3339(), "content": l.content }))
        .collect();
    Ok(Some(
        serde_json::json!({ "lines": lines, "count": lines.len() }),
    ))
}

async fn action_snapshot(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    device_configs: &HashMap<String, DeviceConfig>,
) -> Result<Option<serde_json::Value>> {
    let (conn, _session_id) = get_connection(step, manager, sessions, device_configs).await?;

    let count = step
        .params
        .as_ref()
        .and_then(|p| p.get("count"))
        .and_then(|v| v.as_u64())
        .map(|c| (c as usize).min(MAX_SNAPSHOT_COUNT))
        .unwrap_or(DEFAULT_SNAPSHOT_COUNT);

    let lines = conn.get_recent_lines(count).await;
    let formatted: Vec<serde_json::Value> = lines
        .iter()
        .map(|l| serde_json::json!({ "timestamp": l.timestamp.to_rfc3339(), "content": l.content }))
        .collect();

    Ok(Some(
        serde_json::json!({ "lines": formatted, "count": formatted.len() }),
    ))
}

async fn action_delay(step: &StepConfig) -> Result<Option<serde_json::Value>> {
    let ms = step
        .params
        .as_ref()
        .and_then(|p| p.get("ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Cap at overall max timeout to prevent abuse
    let ms = ms.min(MAX_OVERALL_TIMEOUT_S * 1000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
    Ok(Some(serde_json::json!({ "delayed_ms": ms })))
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Look up the SerialConnection for a step's device.
async fn get_connection(
    step: &StepConfig,
    manager: &SessionManager,
    sessions: &Arc<Mutex<HashMap<String, String>>>,
    _device_configs: &HashMap<String, DeviceConfig>,
) -> Result<(Arc<SerialConnection>, String)> {
    let dev_name = step
        .device
        .as_ref()
        .ok_or_else(|| anyhow!("step {:?} requires a device", step.id))?;

    let session_id = {
        let sess = sessions.lock().await;
        sess.get(dev_name)
            .cloned()
            .ok_or_else(|| anyhow!("no open session for device {:?}", dev_name))?
    };

    let conn = manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| anyhow!("session {} not found in manager", session_id))?;

    Ok((conn, session_id))
}

/// Extract data bytes from step params. Supports text (with escape sequences) and hex mode.
fn extract_data_bytes(params: Option<&serde_json::Value>) -> Result<Vec<u8>> {
    let params = params.ok_or_else(|| anyhow!("params required for data operation"))?;

    let data_str = params
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("'data' param is required"))?;

    let is_hex = params.get("hex").and_then(|v| v.as_bool()).unwrap_or(false);

    if is_hex {
        if data_str.len() > MAX_HEX_INPUT_LEN {
            return Err(anyhow!(
                "hex input too long: {} chars (max {})",
                data_str.len(),
                MAX_HEX_INPUT_LEN
            ));
        }
        format::parse_hex(data_str).map_err(|e| anyhow!("{e}"))
    } else {
        if data_str.len() > MAX_TEXT_INPUT_LEN {
            return Err(anyhow!(
                "text input too long: {} chars (max {})",
                data_str.len(),
                MAX_TEXT_INPUT_LEN
            ));
        }
        Ok(unescape_text(data_str))
    }
}

/// Unescape common escape sequences in text data: \r, \n, \t, \\.
fn unescape_text(s: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('r') => result.push(b'\r'),
                Some('n') => result.push(b'\n'),
                Some('t') => result.push(b'\t'),
                Some('\\') => result.push(b'\\'),
                Some(other) => {
                    result.push(b'\\');
                    let mut buf = [0u8; 4];
                    result.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
                }
                None => result.push(b'\\'),
            }
        } else {
            let mut buf = [0u8; 4];
            result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    result
}

/// Build a minimal error report for validation failures.
fn make_error_report(
    harness_name: &str,
    device_names: &[String],
    start: Instant,
    result: HarnessResult,
    step_id: &str,
    error_msg: &str,
) -> HarnessReport {
    HarnessReport {
        harness: harness_name.to_string(),
        result,
        duration_ms: start.elapsed().as_millis() as u64,
        devices: device_names.to_vec(),
        steps: vec![StepReport {
            id: step_id.to_string(),
            action: "validation".to_string(),
            result: StepResult::Fail,
            duration_ms: 0,
            error: Some(error_msg.to_string()),
            output: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_basic_sequences() {
        assert_eq!(unescape_text(r"AT\r\n"), b"AT\r\n");
        assert_eq!(unescape_text(r"hello\tworld"), b"hello\tworld");
        assert_eq!(unescape_text(r"back\\slash"), b"back\\slash");
    }

    #[test]
    fn unescape_no_escapes() {
        assert_eq!(unescape_text("plain text"), b"plain text");
    }

    #[test]
    fn unescape_trailing_backslash() {
        assert_eq!(unescape_text("end\\"), b"end\\");
    }

    #[test]
    fn unescape_unknown_escape() {
        assert_eq!(unescape_text(r"\x"), b"\\x");
    }

    #[test]
    fn extract_data_text() {
        let params = serde_json::json!({ "data": "AT\\r\\n" });
        let bytes = extract_data_bytes(Some(&params)).unwrap();
        // Note: JSON string "AT\\r\\n" becomes Rust string "AT\r\n" (literal backslash-r-backslash-n)
        // which unescape_text converts to AT<CR><LF>
        assert_eq!(bytes, b"AT\r\n");
    }

    #[test]
    fn extract_data_hex() {
        let params = serde_json::json!({ "data": "48 45 4C 4C 4F", "hex": true });
        let bytes = extract_data_bytes(Some(&params)).unwrap();
        assert_eq!(bytes, b"HELLO");
    }

    #[test]
    fn extract_data_hex_too_long() {
        let long = "AB ".repeat(MAX_HEX_INPUT_LEN + 1);
        let params = serde_json::json!({ "data": long, "hex": true });
        assert!(extract_data_bytes(Some(&params)).is_err());
    }

    #[test]
    fn extract_data_missing_params() {
        assert!(extract_data_bytes(None).is_err());
    }

    #[test]
    fn extract_data_missing_data_field() {
        let params = serde_json::json!({ "hex": true });
        assert!(extract_data_bytes(Some(&params)).is_err());
    }
}
