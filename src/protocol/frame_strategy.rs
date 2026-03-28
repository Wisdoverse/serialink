use anyhow::Result;
use base64::Engine;
use bytes::BytesMut;
use chrono::Utc;
use std::collections::HashMap;

use crate::protocol::frame_parser::{FrameError, FrameParser};
use crate::protocol::types::{
    DecodedFrame, ProtocolDecoder, RawFrame, META_FRAME, META_FRAME_ERROR, META_FRAME_SUMMARY,
    META_MODE, META_PROTOCOL,
};
use crate::serial::port::TimestampedLine;
use crate::serial::read_strategy::ReadStrategy;

/// Frame-oriented read strategy for binary protocols.
///
/// Reads raw bytes from the serial port and uses a `FrameParser` to extract
/// discrete frames, optionally decoding them with a `ProtocolDecoder`.
pub struct FrameReadStrategy {
    parser: FrameParser,
    decoder: Option<Box<dyn ProtocolDecoder>>,
    protocol_name: String,
    buf: BytesMut,
}

impl FrameReadStrategy {
    pub fn new(
        parser: FrameParser,
        decoder: Option<Box<dyn ProtocolDecoder>>,
        protocol_name: String,
    ) -> Self {
        Self {
            parser,
            decoder,
            protocol_name,
            buf: BytesMut::with_capacity(4096),
        }
    }

    /// Convert a raw frame into a `TimestampedLine` with binary metadata.
    fn frame_to_line(&self, raw: &RawFrame) -> TimestampedLine {
        let content = base64::engine::general_purpose::STANDARD.encode(&raw.data);
        let mut metadata = HashMap::new();
        metadata.insert(META_MODE.to_string(), "binary".to_string());
        metadata.insert(META_PROTOCOL.to_string(), self.protocol_name.clone());

        // Try protocol decoder
        let decoded: Option<DecodedFrame> =
            self.decoder.as_ref().and_then(|dec| dec.decode(&raw.data));

        match decoded {
            Some(df) => {
                metadata.insert(META_FRAME_SUMMARY.to_string(), df.summary.clone());
                // Build frame JSON including checksum_valid
                let mut frame_obj = serde_json::Map::new();
                frame_obj.insert(
                    "fields".to_string(),
                    serde_json::Value::Object(df.fields.clone()),
                );
                if let Some(valid) = raw.checksum_valid {
                    frame_obj.insert("checksum_valid".to_string(), serde_json::Value::Bool(valid));
                }
                if let Ok(json) = serde_json::to_string(&frame_obj) {
                    metadata.insert(META_FRAME.to_string(), json);
                }
            }
            None => {
                let n = raw.data.len();
                metadata.insert(
                    META_FRAME_SUMMARY.to_string(),
                    format!("raw frame ({} bytes)", n),
                );
                let hex: String = raw.data.iter().map(|b| format!("{:02x}", b)).collect();
                let mut frame_obj = serde_json::Map::new();
                frame_obj.insert("hex".to_string(), serde_json::Value::String(hex));
                frame_obj.insert(
                    "length".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(n)),
                );
                if let Some(valid) = raw.checksum_valid {
                    frame_obj.insert("checksum_valid".to_string(), serde_json::Value::Bool(valid));
                }
                if let Ok(json) = serde_json::to_string(&frame_obj) {
                    metadata.insert(META_FRAME.to_string(), json);
                }
            }
        }

        TimestampedLine {
            timestamp: raw.timestamp,
            content,
            raw: raw.data.clone(),
            metadata,
        }
    }

    /// Create a diagnostic line for frame errors (Timeout/Overflow).
    fn error_to_line(&self, err: &FrameError) -> TimestampedLine {
        let msg = err.to_string();
        let mut metadata = HashMap::new();
        metadata.insert(META_MODE.to_string(), "binary".to_string());
        metadata.insert(META_PROTOCOL.to_string(), self.protocol_name.clone());
        metadata.insert(META_FRAME_ERROR.to_string(), msg.clone());

        TimestampedLine {
            timestamp: Utc::now(),
            content: format!("[frame error: {}]", msg),
            raw: Vec::new(),
            metadata,
        }
    }
}

impl ReadStrategy for FrameReadStrategy {
    fn read_frames(
        &mut self,
        port: &mut dyn serialport::SerialPort,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<TimestampedLine>> {
        let mut read_buf = [0u8; 4096];
        let mut lines = Vec::new();

        // Read in a loop for a short burst to collect available data.
        for _ in 0..10 {
            if cancel.is_cancelled() {
                break;
            }
            match std::io::Read::read(port, &mut read_buf) {
                Ok(n) if n > 0 => {
                    self.buf.extend_from_slice(&read_buf[..n]);
                }
                Ok(_) => break,
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                Err(e) => return Err(e.into()),
            }
        }

        // Extract frames from the buffer.
        loop {
            match self.parser.decode(&mut self.buf) {
                Ok(Some(frame)) => {
                    lines.push(self.frame_to_line(&frame));
                }
                Ok(None) => break,
                Err(e) => {
                    lines.push(self.error_to_line(&e));
                    // After error, parser has already reset its state.
                    // Continue trying to decode remaining data.
                    if self.buf.is_empty() {
                        break;
                    }
                }
            }
        }

        Ok(lines)
    }

    fn reset(&mut self) {
        self.parser.reset();
        self.buf.clear();
    }
}
