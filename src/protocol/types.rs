use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Metadata key constants
// ---------------------------------------------------------------------------

pub const META_MODE: &str = "_mode";
pub const META_FRAME: &str = "frame";
pub const META_FRAME_ERROR: &str = "frame_error";
pub const META_PROTOCOL: &str = "protocol";
pub const META_FRAME_SUMMARY: &str = "frame_summary";

// ---------------------------------------------------------------------------
// SessionMode
// ---------------------------------------------------------------------------

/// Whether a session operates in text (line-oriented) or binary (frame-oriented) mode.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    #[default]
    Text,
    Binary,
}

// ---------------------------------------------------------------------------
// Endian
// ---------------------------------------------------------------------------

/// Byte order for multi-byte length fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Endian {
    #[default]
    Big,
    Little,
}

// ---------------------------------------------------------------------------
// ChecksumType
// ---------------------------------------------------------------------------

/// Supported checksum / CRC algorithms for frame validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumType {
    Crc16Modbus,
    Crc8,
    Xor,
    Sum8,
    Lrc,
}

// ---------------------------------------------------------------------------
// FramingRule
// ---------------------------------------------------------------------------

/// How to delimit individual frames from the raw byte stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FramingRule {
    FixedSize {
        size: usize,
    },
    LengthPrefixed {
        start: Vec<u8>,
        length_offset: usize,
        length_size: usize,
        #[serde(default)]
        length_endian: Endian,
        #[serde(default)]
        length_includes_header: bool,
        #[serde(default)]
        trailer_size: usize,
    },
    Delimited {
        start: Vec<u8>,
        end: Vec<u8>,
    },
    ModbusRtuGap {
        #[serde(default)]
        baud_rate: Option<u32>,
    },
}

// ---------------------------------------------------------------------------
// FrameConfig
// ---------------------------------------------------------------------------

fn default_frame_timeout_ms() -> u64 {
    500
}

fn default_max_frame_size() -> usize {
    1024
}

/// Configuration describing how to extract and validate frames from raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameConfig {
    pub name: String,
    pub framing: FramingRule,
    #[serde(default)]
    pub checksum: Option<ChecksumType>,
    #[serde(default = "default_frame_timeout_ms")]
    pub frame_timeout_ms: u64,
    #[serde(default = "default_max_frame_size")]
    pub max_frame_size: usize,
}

// ---------------------------------------------------------------------------
// RawFrame
// ---------------------------------------------------------------------------

/// A raw frame extracted from the byte stream, before protocol decoding.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub data: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub checksum_valid: Option<bool>,
}

// ---------------------------------------------------------------------------
// DecodedFrame
// ---------------------------------------------------------------------------

/// A protocol-decoded frame with human-readable summary and structured fields.
#[derive(Debug, Clone, Serialize)]
pub struct DecodedFrame {
    pub summary: String,
    pub fields: serde_json::Map<String, serde_json::Value>,
    pub checksum_valid: Option<bool>,
}

// ---------------------------------------------------------------------------
// ProtocolDecoder trait
// ---------------------------------------------------------------------------

/// Trait for protocol-specific frame decoders (e.g. Modbus, custom protocols).
pub trait ProtocolDecoder: Send + Sync {
    /// Attempt to decode a raw frame into structured fields.
    fn decode(&self, frame: &[u8]) -> Option<DecodedFrame>;

    /// The protocol name (e.g. "modbus_rtu").
    fn protocol(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_config_roundtrips_through_json() {
        let config = FrameConfig {
            name: "test".to_string(),
            framing: FramingRule::FixedSize { size: 8 },
            checksum: Some(ChecksumType::Crc16Modbus),
            frame_timeout_ms: 200,
            max_frame_size: 2048,
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: FrameConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.frame_timeout_ms, 200);
        assert_eq!(parsed.max_frame_size, 2048);
        assert_eq!(parsed.checksum, Some(ChecksumType::Crc16Modbus));
    }

    #[test]
    fn framing_rule_length_prefixed_deserializes_all_fields() {
        let json = r#"{
            "type": "length_prefixed",
            "start": [0, 1],
            "length_offset": 2,
            "length_size": 2,
            "length_endian": "little",
            "length_includes_header": true,
            "trailer_size": 2
        }"#;

        let rule: FramingRule = serde_json::from_str(json).unwrap();
        match rule {
            FramingRule::LengthPrefixed {
                start,
                length_offset,
                length_size,
                length_endian,
                length_includes_header,
                trailer_size,
            } => {
                assert_eq!(start, vec![0, 1]);
                assert_eq!(length_offset, 2);
                assert_eq!(length_size, 2);
                assert_eq!(length_endian, Endian::Little);
                assert!(length_includes_header);
                assert_eq!(trailer_size, 2);
            }
            _ => panic!("expected LengthPrefixed"),
        }
    }

    #[test]
    fn framing_rule_modbus_rtu_gap_none_baud() {
        let json = r#"{"type": "modbus_rtu_gap"}"#;
        let rule: FramingRule = serde_json::from_str(json).unwrap();
        match rule {
            FramingRule::ModbusRtuGap { baud_rate } => {
                assert_eq!(baud_rate, None);
            }
            _ => panic!("expected ModbusRtuGap"),
        }
    }

    #[test]
    fn session_mode_default_is_text() {
        assert_eq!(SessionMode::default(), SessionMode::Text);
    }

    #[test]
    fn endian_default_is_big() {
        assert_eq!(Endian::default(), Endian::Big);
    }

    #[test]
    fn framing_rule_unknown_type_produces_error() {
        let json = r#"{"type": "unknown_type", "foo": 1}"#;
        let result = serde_json::from_str::<FramingRule>(json);
        assert!(result.is_err());
    }

    #[test]
    fn frame_config_toml_deserialization() {
        let toml_str = r#"
name = "modbus"
frame_timeout_ms = 100
max_frame_size = 512

[framing]
type = "fixed_size"
size = 8
"#;

        let config: FrameConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "modbus");
        assert_eq!(config.frame_timeout_ms, 100);
        assert_eq!(config.max_frame_size, 512);
        assert!(config.checksum.is_none());
        match config.framing {
            FramingRule::FixedSize { size } => assert_eq!(size, 8),
            _ => panic!("expected FixedSize"),
        }
    }

    #[test]
    fn frame_config_defaults_applied() {
        let json = r#"{
            "name": "minimal",
            "framing": {"type": "fixed_size", "size": 4}
        }"#;
        let config: FrameConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.frame_timeout_ms, 500);
        assert_eq!(config.max_frame_size, 1024);
        assert!(config.checksum.is_none());
    }
}
