use crate::protocol::types::*;
use crate::serial::port::TimestampedLine;
use base64::Engine;

/// Format a binary-mode line as structured JSON.
/// Always derives content_base64 from line.raw (not line.content which may be mutated).
pub fn format_binary_line(line: &TimestampedLine) -> serde_json::Value {
    let content_base64 = base64::engine::general_purpose::STANDARD.encode(&line.raw);
    let protocol = line.metadata.get(META_PROTOCOL).cloned();

    // Parse the frame JSON from metadata, or create a raw fallback
    let frame = if let Some(frame_json) = line.metadata.get(META_FRAME) {
        serde_json::from_str(frame_json).unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::json!(null)
    };

    let mut obj = serde_json::json!({
        "timestamp": line.timestamp.to_rfc3339(),
        "content_base64": content_base64,
    });

    if let Some(p) = protocol {
        obj["protocol"] = serde_json::json!(p);
    }
    if !frame.is_null() {
        obj["frame"] = frame;
    }
    // Frame error case
    if let Some(err) = line.metadata.get(META_FRAME_ERROR) {
        obj["frame_error"] = serde_json::json!(err);
    }

    obj
}

/// Check if a line is binary mode
pub fn is_binary_line(line: &TimestampedLine) -> bool {
    line.metadata.get(META_MODE).is_some_and(|v| v == "binary")
}

/// Get the matchable content for regex filtering/matching.
/// For binary lines: returns frame_summary. For text: returns content.
pub fn matchable_content(line: &TimestampedLine) -> &str {
    if is_binary_line(line) {
        line.metadata
            .get(META_FRAME_SUMMARY)
            .map_or("", |s| s.as_str())
    } else {
        &line.content
    }
}

/// Parse a hex string into bytes. Strips spaces, validates pairs.
pub fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() % 2 != 0 {
        return Err("Hex string must have an even number of characters".to_string());
    }
    if cleaned.is_empty() {
        return Err("Hex string is empty".to_string());
    }
    let mut bytes = Vec::with_capacity(cleaned.len() / 2);
    for i in (0..cleaned.len()).step_by(2) {
        let byte_str = &cleaned[i..i + 2];
        let byte = u8::from_str_radix(byte_str, 16)
            .map_err(|_| format!("Invalid hex byte: '{}'", byte_str))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// Format raw bytes as space-separated hex for plain text output.
pub fn format_hex_bytes(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_is_binary_line() {
        let mut metadata = HashMap::new();
        metadata.insert(META_MODE.to_string(), "binary".to_string());
        let line = TimestampedLine {
            timestamp: chrono::Utc::now(),
            content: String::new(),
            raw: vec![0x01, 0x02],
            metadata,
        };
        assert!(is_binary_line(&line));
    }

    #[test]
    fn test_is_not_binary_line() {
        let line = TimestampedLine {
            timestamp: chrono::Utc::now(),
            content: "hello".to_string(),
            raw: b"hello".to_vec(),
            metadata: HashMap::new(),
        };
        assert!(!is_binary_line(&line));
    }

    #[test]
    fn test_matchable_content_text() {
        let line = TimestampedLine {
            timestamp: chrono::Utc::now(),
            content: "hello world".to_string(),
            raw: b"hello world".to_vec(),
            metadata: HashMap::new(),
        };
        assert_eq!(matchable_content(&line), "hello world");
    }

    #[test]
    fn test_matchable_content_binary() {
        let mut metadata = HashMap::new();
        metadata.insert(META_MODE.to_string(), "binary".to_string());
        metadata.insert(META_FRAME_SUMMARY.to_string(), "READ addr=1".to_string());
        let line = TimestampedLine {
            timestamp: chrono::Utc::now(),
            content: String::new(),
            raw: vec![0x01, 0x03],
            metadata,
        };
        assert_eq!(matchable_content(&line), "READ addr=1");
    }

    #[test]
    fn test_format_binary_line_basic() {
        let mut metadata = HashMap::new();
        metadata.insert(META_MODE.to_string(), "binary".to_string());
        metadata.insert(META_PROTOCOL.to_string(), "modbus_rtu".to_string());
        let line = TimestampedLine {
            timestamp: chrono::Utc::now(),
            content: String::new(),
            raw: vec![0x01, 0x03, 0x00, 0x01],
            metadata,
        };
        let json = format_binary_line(&line);
        assert!(json["content_base64"].is_string());
        assert_eq!(json["protocol"], "modbus_rtu");
        assert!(json["timestamp"].is_string());
    }

    #[test]
    fn test_parse_hex_valid() {
        assert_eq!(
            parse_hex("01 03 00 01").unwrap(),
            vec![0x01, 0x03, 0x00, 0x01]
        );
        assert_eq!(parse_hex("0103").unwrap(), vec![0x01, 0x03]);
        assert_eq!(parse_hex("FF").unwrap(), vec![0xFF]);
    }

    #[test]
    fn test_parse_hex_invalid() {
        assert!(parse_hex("0G").is_err());
        assert!(parse_hex("123").is_err()); // odd length
        assert!(parse_hex("").is_err());
    }

    #[test]
    fn test_format_hex_bytes() {
        assert_eq!(format_hex_bytes(&[0x01, 0x03, 0xFF]), "01 03 FF");
        assert_eq!(format_hex_bytes(&[]), "");
    }
}
