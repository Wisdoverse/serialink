use serde::{Deserialize, Serialize};

/// Top-level harness configuration, deserialized from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    pub harness: HarnessMetadata,
    #[serde(default, rename = "device")]
    pub devices: Vec<DeviceConfig>,
    #[serde(default, rename = "step")]
    pub steps: Vec<StepConfig>,
}

/// Metadata about the harness itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessMetadata {
    pub name: String,
    pub timeout: Option<u64>,
}

/// A device (serial port) referenced by steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub name: String,
    pub port: String,
    pub baud_rate: Option<u32>,
    pub protocol: Option<crate::config::ProtocolConfig>,
}

/// A single step in the harness execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepConfig {
    pub id: String,
    pub device: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub action: String,
    pub params: Option<serde_json::Value>,
    pub on_fail: Option<OnFail>,
}

/// Behaviour when a step fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnFail {
    Abort,
    Continue,
    Ignore,
}

/// Overall harness outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessResult {
    Pass,
    Fail,
    Aborted,
    Timeout,
}

/// Outcome of a single step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepResult {
    Pass,
    Fail,
    Skipped,
    Ignored,
}

/// Final report emitted after harness execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessReport {
    pub harness: String,
    pub result: HarnessResult,
    pub duration_ms: u64,
    pub devices: Vec<String>,
    pub steps: Vec<StepReport>,
}

/// Report for a single step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub id: String,
    pub action: String,
    pub result: StepResult,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_full_harness_config() {
        let toml_str = r#"
[harness]
name = "smoke-test"
timeout = 30

[[device]]
name = "dut"
port = "/dev/ttyUSB0"
baud_rate = 115200

[[device]]
name = "aux"
port = "/dev/ttyUSB1"

[[step]]
id = "open"
device = "dut"
action = "open_port"
on_fail = "abort"

[[step]]
id = "check"
device = "dut"
depends_on = ["open"]
action = "send_and_expect"
on_fail = "continue"

[step.params]
data = "AT\r\n"
expect = "OK"
"#;
        let cfg: HarnessConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.harness.name, "smoke-test");
        assert_eq!(cfg.harness.timeout, Some(30));
        assert_eq!(cfg.devices.len(), 2);
        assert_eq!(cfg.devices[0].baud_rate, Some(115200));
        assert_eq!(cfg.devices[1].baud_rate, None);
        assert_eq!(cfg.steps.len(), 2);
        assert_eq!(cfg.steps[0].on_fail, Some(OnFail::Abort));
        assert_eq!(cfg.steps[1].depends_on.as_ref().unwrap(), &["open"]);
        assert!(cfg.steps[1].params.is_some());
    }

    #[test]
    fn deserialize_minimal_config() {
        let toml_str = r#"
[harness]
name = "minimal"

[[step]]
id = "s1"
action = "list_ports"
"#;
        let cfg: HarnessConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.harness.name, "minimal");
        assert_eq!(cfg.harness.timeout, None);
        assert!(cfg.devices.is_empty());
        assert_eq!(cfg.steps.len(), 1);
        assert_eq!(cfg.steps[0].device, None);
        assert_eq!(cfg.steps[0].depends_on, None);
        assert_eq!(cfg.steps[0].params, None);
        assert_eq!(cfg.steps[0].on_fail, None);
    }

    #[test]
    fn deserialize_on_fail_variants() {
        for (input, expected) in [
            ("\"abort\"", OnFail::Abort),
            ("\"continue\"", OnFail::Continue),
            ("\"ignore\"", OnFail::Ignore),
        ] {
            let parsed: OnFail = serde_json::from_str(input).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn json_round_trip() {
        let toml_str = r#"
[harness]
name = "round-trip"
timeout = 10

[[device]]
name = "dut"
port = "/dev/ttyUSB0"
baud_rate = 9600

[[step]]
id = "s1"
action = "open_port"
device = "dut"
on_fail = "abort"
"#;
        let cfg: HarnessConfig = toml::from_str(toml_str).unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg2: HarnessConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.harness.name, "round-trip");
        assert_eq!(cfg2.harness.timeout, Some(10));
        assert_eq!(cfg2.devices.len(), 1);
        assert_eq!(cfg2.steps[0].on_fail, Some(OnFail::Abort));
    }

    #[test]
    fn harness_report_serializes() {
        let report = HarnessReport {
            harness: "smoke".into(),
            result: HarnessResult::Pass,
            duration_ms: 1234,
            devices: vec!["dut".into()],
            steps: vec![
                StepReport {
                    id: "s1".into(),
                    action: "open_port".into(),
                    result: StepResult::Pass,
                    duration_ms: 100,
                    error: None,
                    output: None,
                },
                StepReport {
                    id: "s2".into(),
                    action: "send".into(),
                    result: StepResult::Fail,
                    duration_ms: 500,
                    error: Some("timeout".into()),
                    output: Some(serde_json::json!({"lines": 3})),
                },
            ],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"result\":\"pass\""));
        assert!(json.contains("\"result\":\"fail\""));
        // skip_serializing_if: s1 should not have "error" or "output" keys
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        let s1 = &val["steps"][0];
        assert!(s1.get("error").is_none());
        assert!(s1.get("output").is_none());
        // s2 should have them
        let s2 = &val["steps"][1];
        assert_eq!(s2["error"], "timeout");
        assert_eq!(s2["output"]["lines"], 3);
    }
}
