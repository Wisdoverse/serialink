use crate::config::ProtocolConfig;
use crate::protocol::types::{ChecksumType, FrameConfig, FramingRule};

/// Resolve a built-in protocol preset by name.
///
/// Returns `None` for unknown names.
pub fn resolve_preset(name: &str) -> Option<ProtocolConfig> {
    match name {
        "modbus_rtu" => Some(ProtocolConfig {
            frame: FrameConfig {
                name: "modbus_rtu".to_string(),
                framing: FramingRule::ModbusRtuGap { baud_rate: None },
                checksum: Some(ChecksumType::Crc16Modbus),
                frame_timeout_ms: 100,
                max_frame_size: 256,
            },
            decoder: Some("modbus_rtu".to_string()),
        }),
        "modbus_ascii" => Some(ProtocolConfig {
            frame: FrameConfig {
                name: "modbus_ascii".to_string(),
                framing: FramingRule::Delimited {
                    start: vec![0x3A],
                    end: vec![0x0D, 0x0A],
                },
                checksum: Some(ChecksumType::Lrc),
                frame_timeout_ms: 500,
                max_frame_size: 513,
            },
            decoder: Some("modbus_ascii".to_string()),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_modbus_rtu_preset() {
        let cfg = resolve_preset("modbus_rtu").expect("should resolve modbus_rtu");
        assert_eq!(cfg.frame.name, "modbus_rtu");
        assert_eq!(cfg.frame.frame_timeout_ms, 100);
        assert_eq!(cfg.frame.max_frame_size, 256);
        assert_eq!(cfg.frame.checksum, Some(ChecksumType::Crc16Modbus));
        assert_eq!(cfg.decoder, Some("modbus_rtu".to_string()));
        match cfg.frame.framing {
            FramingRule::ModbusRtuGap { baud_rate } => assert_eq!(baud_rate, None),
            _ => panic!("expected ModbusRtuGap framing"),
        }
    }

    #[test]
    fn resolve_modbus_ascii_preset() {
        let cfg = resolve_preset("modbus_ascii").expect("should resolve modbus_ascii");
        assert_eq!(cfg.frame.name, "modbus_ascii");
        assert_eq!(cfg.frame.frame_timeout_ms, 500);
        assert_eq!(cfg.frame.max_frame_size, 513);
        assert_eq!(cfg.frame.checksum, Some(ChecksumType::Lrc));
        assert_eq!(cfg.decoder, Some("modbus_ascii".to_string()));
        match cfg.frame.framing {
            FramingRule::Delimited { ref start, ref end } => {
                assert_eq!(start, &[0x3A]);
                assert_eq!(end, &[0x0D, 0x0A]);
            }
            _ => panic!("expected Delimited framing"),
        }
    }

    #[test]
    fn resolve_unknown_returns_none() {
        assert!(resolve_preset("unknown").is_none());
        assert!(resolve_preset("").is_none());
        assert!(resolve_preset("modbus").is_none());
    }
}
