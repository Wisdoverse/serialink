use serde_json::{Map, Value};

use super::types::{DecodedFrame, ProtocolDecoder};

// ---------------------------------------------------------------------------
// Shared PDU decoding
// ---------------------------------------------------------------------------

fn exception_name(code: u8) -> &'static str {
    match code {
        0x01 => "illegal_function",
        0x02 => "illegal_data_address",
        0x03 => "illegal_data_value",
        0x04 => "server_device_failure",
        _ => "unknown",
    }
}

fn u16_be(pdu: &[u8], offset: usize) -> Option<u16> {
    if offset + 1 < pdu.len() {
        Some(u16::from_be_bytes([pdu[offset], pdu[offset + 1]]))
    } else {
        None
    }
}

/// Decode a Modbus PDU: `[slave_id, function_code, data...]`.
///
/// Returns `None` only when the PDU is shorter than 2 bytes.
pub fn decode_pdu(pdu: &[u8]) -> Option<DecodedFrame> {
    if pdu.len() < 2 {
        return None;
    }

    let slave_id = pdu[0];
    let raw_fc = pdu[1];

    let mut fields: Map<String, Value> = Map::new();
    fields.insert("slave_id".into(), Value::Number(slave_id.into()));
    fields.insert("function_code".into(), Value::Number(raw_fc.into()));

    // Exception responses: bit 7 set
    if raw_fc >= 0x80 {
        let orig_fc = raw_fc & 0x7F;
        let exc_code = pdu.get(2).copied().unwrap_or(0);
        let exc_name = exception_name(exc_code);

        fields.insert(
            "original_function_code".into(),
            Value::Number(orig_fc.into()),
        );
        fields.insert("exception_code".into(), Value::Number(exc_code.into()));
        fields.insert("exception_name".into(), Value::String(exc_name.into()));
        fields.insert(
            "function_name".into(),
            Value::String(format!("Exception: {}", exc_name)),
        );

        return Some(DecodedFrame {
            summary: format!("Exception: {}", exc_name),
            fields,
            checksum_valid: None,
        });
    }

    // Known function codes
    match raw_fc {
        // ---- Read Coils -------------------------------------------------------
        0x01 => {
            fields.insert("function_name".into(), Value::String("Read Coils".into()));
            // Determine request vs response
            // Request: slave + fc + start(2) + count(2) = 6 bytes
            if pdu.len() == 6 {
                let start = u16_be(pdu, 2).unwrap_or(0);
                let count = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("start_address".into(), Value::Number(start.into()));
                fields.insert("coil_count".into(), Value::Number(count.into()));
                Some(DecodedFrame {
                    summary: "Read Coils".into(),
                    fields,
                    checksum_valid: None,
                })
            } else if pdu.len() >= 3 {
                // Response: slave + fc + byte_count + data...
                let byte_count = pdu[2] as usize;
                let data: Vec<Value> = pdu
                    .get(3..3 + byte_count)
                    .unwrap_or(&[])
                    .iter()
                    .map(|b| Value::Number((*b).into()))
                    .collect();
                fields.insert("byte_count".into(), Value::Number(pdu[2].into()));
                fields.insert("data".into(), Value::Array(data));
                Some(DecodedFrame {
                    summary: "Read Coils".into(),
                    fields,
                    checksum_valid: None,
                })
            } else {
                Some(DecodedFrame {
                    summary: "Read Coils".into(),
                    fields,
                    checksum_valid: None,
                })
            }
        }

        // ---- Read Discrete Inputs -------------------------------------------
        0x02 => {
            fields.insert(
                "function_name".into(),
                Value::String("Read Discrete Inputs".into()),
            );
            if pdu.len() == 6 {
                let start = u16_be(pdu, 2).unwrap_or(0);
                let count = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("start_address".into(), Value::Number(start.into()));
                fields.insert("input_count".into(), Value::Number(count.into()));
                Some(DecodedFrame {
                    summary: "Read Discrete Inputs".into(),
                    fields,
                    checksum_valid: None,
                })
            } else if pdu.len() >= 3 {
                let byte_count = pdu[2] as usize;
                let data: Vec<Value> = pdu
                    .get(3..3 + byte_count)
                    .unwrap_or(&[])
                    .iter()
                    .map(|b| Value::Number((*b).into()))
                    .collect();
                fields.insert("byte_count".into(), Value::Number(pdu[2].into()));
                fields.insert("data".into(), Value::Array(data));
                Some(DecodedFrame {
                    summary: "Read Discrete Inputs".into(),
                    fields,
                    checksum_valid: None,
                })
            } else {
                Some(DecodedFrame {
                    summary: "Read Discrete Inputs".into(),
                    fields,
                    checksum_valid: None,
                })
            }
        }

        // ---- Read Holding Registers -----------------------------------------
        0x03 => {
            fields.insert(
                "function_name".into(),
                Value::String("Read Holding Registers".into()),
            );
            if pdu.len() == 6 {
                let start = u16_be(pdu, 2).unwrap_or(0);
                let count = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("start_address".into(), Value::Number(start.into()));
                fields.insert("register_count".into(), Value::Number(count.into()));
                Some(DecodedFrame {
                    summary: "Read Holding Registers".into(),
                    fields,
                    checksum_valid: None,
                })
            } else if pdu.len() >= 3 {
                let byte_count = pdu[2] as usize;
                let data: Vec<Value> = pdu
                    .get(3..3 + byte_count)
                    .unwrap_or(&[])
                    .iter()
                    .map(|b| Value::Number((*b).into()))
                    .collect();
                fields.insert("byte_count".into(), Value::Number(pdu[2].into()));
                fields.insert("data".into(), Value::Array(data));
                Some(DecodedFrame {
                    summary: "Read Holding Registers".into(),
                    fields,
                    checksum_valid: None,
                })
            } else {
                Some(DecodedFrame {
                    summary: "Read Holding Registers".into(),
                    fields,
                    checksum_valid: None,
                })
            }
        }

        // ---- Read Input Registers -------------------------------------------
        0x04 => {
            fields.insert(
                "function_name".into(),
                Value::String("Read Input Registers".into()),
            );
            if pdu.len() == 6 {
                let start = u16_be(pdu, 2).unwrap_or(0);
                let count = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("start_address".into(), Value::Number(start.into()));
                fields.insert("register_count".into(), Value::Number(count.into()));
                Some(DecodedFrame {
                    summary: "Read Input Registers".into(),
                    fields,
                    checksum_valid: None,
                })
            } else if pdu.len() >= 3 {
                let byte_count = pdu[2] as usize;
                let data: Vec<Value> = pdu
                    .get(3..3 + byte_count)
                    .unwrap_or(&[])
                    .iter()
                    .map(|b| Value::Number((*b).into()))
                    .collect();
                fields.insert("byte_count".into(), Value::Number(pdu[2].into()));
                fields.insert("data".into(), Value::Array(data));
                Some(DecodedFrame {
                    summary: "Read Input Registers".into(),
                    fields,
                    checksum_valid: None,
                })
            } else {
                Some(DecodedFrame {
                    summary: "Read Input Registers".into(),
                    fields,
                    checksum_valid: None,
                })
            }
        }

        // ---- Write Single Coil ----------------------------------------------
        0x05 => {
            fields.insert(
                "function_name".into(),
                Value::String("Write Single Coil".into()),
            );
            if pdu.len() >= 6 {
                let address = u16_be(pdu, 2).unwrap_or(0);
                let raw_value = u16_be(pdu, 4).unwrap_or(0);
                let coil_value = raw_value == 0xFF00;
                fields.insert("address".into(), Value::Number(address.into()));
                fields.insert("value".into(), Value::Bool(coil_value));
            }
            Some(DecodedFrame {
                summary: "Write Single Coil".into(),
                fields,
                checksum_valid: None,
            })
        }

        // ---- Write Single Register ------------------------------------------
        0x06 => {
            fields.insert(
                "function_name".into(),
                Value::String("Write Single Register".into()),
            );
            if pdu.len() >= 6 {
                let address = u16_be(pdu, 2).unwrap_or(0);
                let value = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("address".into(), Value::Number(address.into()));
                fields.insert("value".into(), Value::Number(value.into()));
            }
            Some(DecodedFrame {
                summary: "Write Single Register".into(),
                fields,
                checksum_valid: None,
            })
        }

        // ---- Write Multiple Coils -------------------------------------------
        0x0F => {
            fields.insert(
                "function_name".into(),
                Value::String("Write Multiple Coils".into()),
            );
            if pdu.len() >= 6 {
                let start = u16_be(pdu, 2).unwrap_or(0);
                let count = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("start_address".into(), Value::Number(start.into()));
                fields.insert("coil_count".into(), Value::Number(count.into()));
            }
            Some(DecodedFrame {
                summary: "Write Multiple Coils".into(),
                fields,
                checksum_valid: None,
            })
        }

        // ---- Write Multiple Registers ---------------------------------------
        0x10 => {
            fields.insert(
                "function_name".into(),
                Value::String("Write Multiple Registers".into()),
            );
            if pdu.len() >= 6 {
                let start = u16_be(pdu, 2).unwrap_or(0);
                let count = u16_be(pdu, 4).unwrap_or(0);
                fields.insert("start_address".into(), Value::Number(start.into()));
                fields.insert("register_count".into(), Value::Number(count.into()));
            }
            Some(DecodedFrame {
                summary: "Write Multiple Registers".into(),
                fields,
                checksum_valid: None,
            })
        }

        // ---- Unknown function code ------------------------------------------
        _ => {
            let raw_data: String = pdu
                .get(2..)
                .unwrap_or(&[])
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");
            fields.insert(
                "function_name".into(),
                Value::String(format!("Unknown Function 0x{:02X}", raw_fc)),
            );
            fields.insert("raw_data".into(), Value::String(raw_data));
            Some(DecodedFrame {
                summary: format!("Unknown Function 0x{:02X}", raw_fc),
                fields,
                checksum_valid: None,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// ModbusRtuDecoder
// ---------------------------------------------------------------------------

/// Decodes Modbus RTU frames.
///
/// Frame layout: `[slave_id, function_code, data..., crc_lo, crc_hi]`
pub struct ModbusRtuDecoder;

impl ProtocolDecoder for ModbusRtuDecoder {
    fn protocol(&self) -> &str {
        "modbus_rtu"
    }

    fn decode(&self, frame: &[u8]) -> Option<DecodedFrame> {
        // Need at least: slave(1) + fc(1) + crc(2) = 4 bytes
        if frame.len() < 4 {
            return None;
        }
        // Strip 2-byte CRC
        let pdu = &frame[..frame.len() - 2];
        if pdu.len() < 2 {
            return None;
        }
        decode_pdu(pdu)
    }
}

// ---------------------------------------------------------------------------
// ModbusAsciiDecoder
// ---------------------------------------------------------------------------

/// Decodes Modbus ASCII frames.
///
/// Frame layout: `[0x3A, hex_chars..., lrc_hi, lrc_lo, 0x0D, 0x0A]`
pub struct ModbusAsciiDecoder;

impl ProtocolDecoder for ModbusAsciiDecoder {
    fn protocol(&self) -> &str {
        "modbus_ascii"
    }

    fn decode(&self, frame: &[u8]) -> Option<DecodedFrame> {
        // Minimum: ':' + 2 hex (slave) + 2 hex (fc) + 2 hex (LRC) + CR + LF = 9 bytes
        if frame.len() < 9 {
            return None;
        }
        // Strip leading ':' (0x3A)
        let after_colon = &frame[1..];
        // Strip trailing CR LF (last 2 bytes)
        let without_crlf = after_colon
            .strip_suffix(&[0x0D, 0x0A])
            .unwrap_or(after_colon);
        // Strip last 2 hex chars (LRC)
        if without_crlf.len() < 2 {
            return None;
        }
        let hex_data = &without_crlf[..without_crlf.len() - 2];

        // Decode hex pairs to bytes
        if hex_data.len() % 2 != 0 {
            return None;
        }
        let mut pdu_bytes: Vec<u8> = Vec::with_capacity(hex_data.len() / 2);
        for chunk in hex_data.chunks(2) {
            let hi = char::from(chunk[0]).to_digit(16)?;
            let lo = char::from(chunk[1]).to_digit(16)?;
            pdu_bytes.push(((hi << 4) | lo) as u8);
        }

        decode_pdu(&pdu_bytes)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- decode_pdu: Read Holding Registers request ---------------------------

    #[test]
    fn read_holding_registers_request() {
        let pdu = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x0A];
        let frame = decode_pdu(&pdu).expect("should decode");
        assert_eq!(frame.summary, "Read Holding Registers");
        assert_eq!(frame.fields["slave_id"], Value::Number(1.into()));
        assert_eq!(frame.fields["function_code"], Value::Number(3.into()));
        assert_eq!(frame.fields["start_address"], Value::Number(0.into()));
        assert_eq!(frame.fields["register_count"], Value::Number(10.into()));
    }

    // -- decode_pdu: Read Holding Registers response --------------------------

    #[test]
    fn read_holding_registers_response() {
        // slave=1, fc=03, byte_count=4, data=[0x00,0x01,0x00,0x02]
        let pdu = [0x01u8, 0x03, 0x04, 0x00, 0x01, 0x00, 0x02];
        let frame = decode_pdu(&pdu).expect("should decode");
        assert_eq!(frame.summary, "Read Holding Registers");
        assert_eq!(frame.fields["byte_count"], Value::Number(4.into()));
        let data = frame.fields["data"].as_array().expect("data is array");
        assert_eq!(data.len(), 4);
        assert_eq!(data[0], Value::Number(0.into()));
        assert_eq!(data[1], Value::Number(1.into()));
        assert_eq!(data[2], Value::Number(0.into()));
        assert_eq!(data[3], Value::Number(2.into()));
    }

    // -- decode_pdu: Write Single Coil ----------------------------------------

    #[test]
    fn write_single_coil_true() {
        let pdu = [0x01u8, 0x05, 0x00, 0x01, 0xFF, 0x00];
        let frame = decode_pdu(&pdu).expect("should decode");
        assert_eq!(frame.summary, "Write Single Coil");
        assert_eq!(frame.fields["address"], Value::Number(1.into()));
        assert_eq!(frame.fields["value"], Value::Bool(true));
    }

    #[test]
    fn write_single_coil_false() {
        let pdu = [0x01u8, 0x05, 0x00, 0x01, 0x00, 0x00];
        let frame = decode_pdu(&pdu).expect("should decode");
        assert_eq!(frame.fields["value"], Value::Bool(false));
    }

    // -- decode_pdu: Exception response ---------------------------------------

    #[test]
    fn exception_response() {
        let pdu = [0x01u8, 0x83, 0x02];
        let frame = decode_pdu(&pdu).expect("should decode");
        assert_eq!(frame.summary, "Exception: illegal_data_address");
        assert_eq!(frame.fields["exception_code"], Value::Number(2.into()));
        assert_eq!(
            frame.fields["exception_name"],
            Value::String("illegal_data_address".into())
        );
    }

    // -- decode_pdu: Unknown function code ------------------------------------

    #[test]
    fn unknown_function_code() {
        let pdu = [0x01u8, 0x42, 0xAA, 0xBB];
        let frame = decode_pdu(&pdu).expect("should decode");
        assert!(frame.summary.starts_with("Unknown Function"));
        assert!(frame.fields.contains_key("raw_data"));
    }

    // -- decode_pdu: Truncated (< 2 bytes) ------------------------------------

    #[test]
    fn truncated_pdu_returns_none() {
        assert!(decode_pdu(&[0x01u8]).is_none());
        assert!(decode_pdu(&[]).is_none());
    }

    // -- ModbusRtuDecoder: strips 2-byte CRC ----------------------------------

    #[test]
    fn rtu_decoder_strips_crc() {
        // Bare PDU: Read Holding Registers request
        let bare = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x0A];
        // With CRC appended (0xC5, 0xCD is the actual CRC16/Modbus for this frame)
        let with_crc = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD];

        let decoder = ModbusRtuDecoder;
        let from_bare = decode_pdu(&bare).expect("bare should decode");
        let from_rtu = decoder.decode(&with_crc).expect("rtu should decode");

        assert_eq!(from_bare.summary, from_rtu.summary);
        assert_eq!(
            from_bare.fields["start_address"],
            from_rtu.fields["start_address"]
        );
        assert_eq!(
            from_bare.fields["register_count"],
            from_rtu.fields["register_count"]
        );
    }

    #[test]
    fn rtu_decoder_too_short_returns_none() {
        let decoder = ModbusRtuDecoder;
        assert!(decoder.decode(&[0x01, 0x03, 0xC5]).is_none()); // 3 bytes → pdu = 1 byte < 2
        assert!(decoder.decode(&[0x01, 0x03]).is_none()); // 2 bytes → pdu = 0 bytes < 2
    }

    // -- ModbusAsciiDecoder: strips delimiters and decodes hex ----------------

    #[test]
    fn ascii_decoder_read_holding_registers() {
        // ASCII frame for: slave=01, fc=03, start=0000, count=000A
        // PDU bytes: 01 03 00 00 00 0A  → hex string "01030000000A" (12 chars = 6 bytes)
        // LRC = (~(0x01+0x03+0x00+0x00+0x00+0x0A) + 1) & 0xFF
        //     = (~0x0E + 1) & 0xFF = (0xF1 + 1) & 0xFF = 0xF2
        // Frame bytes: ':' + "01030000000A" + "F2" + CR + LF
        let frame: Vec<u8> = b":01030000000AF2\r\n".to_vec();
        let decoder = ModbusAsciiDecoder;
        let result = decoder.decode(&frame).expect("should decode");
        assert_eq!(result.summary, "Read Holding Registers");
        assert_eq!(result.fields["slave_id"], Value::Number(1.into()));
        assert_eq!(result.fields["start_address"], Value::Number(0.into()));
        assert_eq!(result.fields["register_count"], Value::Number(10.into()));
    }

    #[test]
    fn ascii_decoder_too_short_returns_none() {
        let decoder = ModbusAsciiDecoder;
        // Too short to be a valid ASCII frame
        assert!(decoder.decode(b":0103\r\n").is_none());
    }

    #[test]
    fn ascii_decoder_invalid_hex_returns_none() {
        let decoder = ModbusAsciiDecoder;
        // Contains non-hex chars in data region
        let frame: Vec<u8> = b":GG0300000AF2\r\n".to_vec();
        assert!(decoder.decode(&frame).is_none());
    }

    // -- Slave ID 0 (broadcast) decodes normally ------------------------------

    #[test]
    fn slave_id_zero_broadcast() {
        // Broadcast: slave=0
        let pdu = [0x00u8, 0x06, 0x00, 0x01, 0x00, 0x64];
        let frame = decode_pdu(&pdu).expect("should decode broadcast");
        assert_eq!(frame.fields["slave_id"], Value::Number(0.into()));
        assert_eq!(frame.summary, "Write Single Register");
    }

    // -- Protocol names -------------------------------------------------------

    #[test]
    fn protocol_names() {
        assert_eq!(ModbusRtuDecoder.protocol(), "modbus_rtu");
        assert_eq!(ModbusAsciiDecoder.protocol(), "modbus_ascii");
    }
}
