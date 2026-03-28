use anyhow::Result;
use serde::{Deserialize, Serialize};

// Re-export the canonical pipeline config types. These are the only
// PipelineStepConfig definitions in the codebase — do not duplicate them here.
#[allow(unused_imports)]
pub use crate::pipeline::engine::{FilterModeConfig, LogFormatConfig, PipelineStepConfig};
use crate::protocol::types::FrameConfig;

/// Protocol configuration for binary/frame-oriented sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolConfig {
    #[serde(flatten)]
    pub frame: FrameConfig,
    /// Optional decoder name: "modbus_rtu", "modbus_ascii", or omitted for raw frames.
    pub decoder: Option<String>,
}

/// Top-level TOML configuration file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialinkConfig {
    pub port: Option<PortConfig>,
    #[serde(default)]
    pub pipeline: Vec<PipelineStepConfig>,
    pub serve: Option<ServeConfig>,
    pub protocol: Option<ProtocolConfig>,
}

/// Serial port configuration as it appears in a TOML config file.
///
/// Uses primitive types (u8, String) for serde compatibility. Convert to
/// `serial::port::PortConfig` via `into_port_config()` before use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortConfig {
    pub path: String,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServeConfig {
    pub mcp: Option<bool>,
    pub http: Option<bool>,
    pub port: Option<u16>,
}

fn default_baud_rate() -> u32 {
    115200
}

fn default_data_bits() -> u8 {
    8
}

fn default_stop_bits() -> u8 {
    1
}

fn default_parity() -> String {
    "none".to_string()
}

fn default_true() -> bool {
    true
}

fn default_reconnect_interval() -> u64 {
    2000
}

impl Default for PortConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            baud_rate: default_baud_rate(),
            data_bits: default_data_bits(),
            stop_bits: default_stop_bits(),
            parity: default_parity(),
            auto_reconnect: default_true(),
            reconnect_interval_ms: default_reconnect_interval(),
        }
    }
}

pub fn load_config(path: &str) -> Result<SerialinkConfig> {
    let content = std::fs::read_to_string(path)?;
    let config: SerialinkConfig = toml::from_str(&content)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::SessionMode;

    #[test]
    fn toml_with_protocol_section_deserializes() {
        let toml_str = r#"
[protocol]
name = "modbus_rtu"
frame_timeout_ms = 100
max_frame_size = 256
decoder = "modbus_rtu"

[protocol.framing]
type = "modbus_rtu_gap"
"#;
        let config: SerialinkConfig = toml::from_str(toml_str).unwrap();
        let proto = config.protocol.expect("protocol should be Some");
        assert_eq!(proto.frame.name, "modbus_rtu");
        assert_eq!(proto.frame.frame_timeout_ms, 100);
        assert_eq!(proto.decoder, Some("modbus_rtu".to_string()));
    }

    #[test]
    fn toml_without_protocol_section_is_none() {
        let toml_str = r#"
[[pipeline]]
type = "timestamp"
format = "iso8601"
"#;
        let config: SerialinkConfig = toml::from_str(toml_str).unwrap();
        assert!(config.protocol.is_none());
    }

    #[test]
    fn port_config_default_has_text_mode() {
        let port = crate::serial::port::PortConfig::default();
        assert_eq!(port.mode, SessionMode::Text);
    }
}
