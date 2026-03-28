use bytes::BytesMut;
use chrono::Utc;
use std::time::Instant;

use crate::protocol::checksum;
use crate::protocol::types::{Endian, FrameConfig, FramingRule, RawFrame};

// ---------------------------------------------------------------------------
// FrameError
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum FrameError {
    Timeout,
    Overflow,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Timeout => write!(f, "frame timeout"),
            FrameError::Overflow => write!(f, "frame overflow"),
        }
    }
}

impl std::error::Error for FrameError {}

// ---------------------------------------------------------------------------
// Internal decode state (for LengthPrefixed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum DecodeState {
    Head,
    Data(usize), // expected total frame size
}

// ---------------------------------------------------------------------------
// FrameParser
// ---------------------------------------------------------------------------

pub struct FrameParser {
    config: FrameConfig,
    state: DecodeState,
    partial_start: Option<Instant>,
    /// For ModbusRtuGap: when we last saw data arrive.
    last_data_time: Option<Instant>,
    /// For ModbusRtuGap: buffer length on the previous decode call, to detect new data.
    prev_buf_len: usize,
}

impl FrameParser {
    pub fn new(config: FrameConfig) -> Self {
        Self {
            config,
            state: DecodeState::Head,
            partial_start: None,
            last_data_time: None,
            prev_buf_len: 0,
        }
    }

    /// Reset parser state (call on reconnect).
    pub fn reset(&mut self) {
        self.state = DecodeState::Head;
        self.partial_start = None;
        self.last_data_time = None;
        self.prev_buf_len = 0;
    }

    /// Attempt to decode one frame from the buffer.
    pub fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<RawFrame>, FrameError> {
        // --- Timeout check ---
        if let Some(start) = self.partial_start {
            if start.elapsed().as_millis() as u64 > self.config.frame_timeout_ms {
                buf.clear();
                self.partial_start = None;
                self.state = DecodeState::Head;
                self.last_data_time = None;
                self.prev_buf_len = 0;
                return Err(FrameError::Timeout);
            }
        }

        if buf.is_empty() {
            return Ok(None);
        }

        // Track partial start
        if self.partial_start.is_none() {
            self.partial_start = Some(Instant::now());
        }

        // --- Overflow check (pre-decode) ---
        if buf.len() > self.config.max_frame_size {
            return self.handle_overflow(buf);
        }

        let result = match &self.config.framing {
            FramingRule::FixedSize { size } => self.decode_fixed(*size, buf),
            FramingRule::LengthPrefixed { .. } => self.decode_length_prefixed(buf),
            FramingRule::Delimited { .. } => self.decode_delimited(buf),
            FramingRule::ModbusRtuGap { .. } => self.decode_modbus_gap(buf),
        };

        // If we emitted a frame, clear partial_start (will re-set on next call if data remains)
        if let Ok(Some(_)) = &result {
            if buf.is_empty() {
                self.partial_start = None;
            } else {
                // More data remains; reset partial timer for next frame
                self.partial_start = Some(Instant::now());
            }
        }

        result
    }

    // -------------------------------------------------------------------
    // FixedSize
    // -------------------------------------------------------------------

    fn decode_fixed(
        &mut self,
        size: usize,
        buf: &mut BytesMut,
    ) -> Result<Option<RawFrame>, FrameError> {
        if buf.len() < size {
            return Ok(None);
        }
        let data = buf.split_to(size).to_vec();
        Ok(Some(self.wrap_frame(data)))
    }

    // -------------------------------------------------------------------
    // LengthPrefixed
    // -------------------------------------------------------------------

    fn decode_length_prefixed(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<RawFrame>, FrameError> {
        // Extract framing params (we know it's LengthPrefixed because dispatch matched).
        let (
            start,
            length_offset,
            length_size,
            length_endian,
            length_includes_header,
            trailer_size,
        ) = match &self.config.framing {
            FramingRule::LengthPrefixed {
                start,
                length_offset,
                length_size,
                length_endian,
                length_includes_header,
                trailer_size,
            } => (
                start.clone(),
                *length_offset,
                *length_size,
                length_endian.clone(),
                *length_includes_header,
                *trailer_size,
            ),
            _ => unreachable!(),
        };

        loop {
            match self.state.clone() {
                DecodeState::Head => {
                    if buf.is_empty() {
                        return Ok(None);
                    }

                    // Find start sequence
                    let start_pos = find_subsequence(buf, &start);
                    match start_pos {
                        None => {
                            // No start found; discard everything
                            buf.clear();
                            return Ok(None);
                        }
                        Some(pos) => {
                            // Discard bytes before start (resynchronize)
                            if pos > 0 {
                                let _ = buf.split_to(pos);
                            }

                            // Check if we have enough bytes for the header
                            let header_needed = length_offset + length_size;
                            if buf.len() < header_needed {
                                return Ok(None);
                            }

                            // Read length field
                            let len_bytes = &buf[length_offset..length_offset + length_size];
                            let payload_len = read_uint(len_bytes, &length_endian);

                            // Calculate total frame size
                            let total = if length_includes_header {
                                payload_len + trailer_size
                            } else {
                                (length_offset + length_size) + payload_len + trailer_size
                            };

                            self.state = DecodeState::Data(total);
                            // Fall through to Data handling
                        }
                    }
                }
                DecodeState::Data(total) => {
                    if buf.len() < total {
                        return Ok(None);
                    }
                    let data = buf.split_to(total).to_vec();
                    self.state = DecodeState::Head;
                    return Ok(Some(self.wrap_frame(data)));
                }
            }
        }
    }

    // -------------------------------------------------------------------
    // Delimited
    // -------------------------------------------------------------------

    fn decode_delimited(&mut self, buf: &mut BytesMut) -> Result<Option<RawFrame>, FrameError> {
        let (start, end) = match &self.config.framing {
            FramingRule::Delimited { start, end } => (start.clone(), end.clone()),
            _ => unreachable!(),
        };

        // Find start
        let start_pos = match find_subsequence(buf, &start) {
            None => {
                buf.clear();
                return Ok(None);
            }
            Some(pos) => pos,
        };

        // Discard bytes before start
        if start_pos > 0 {
            let _ = buf.split_to(start_pos);
        }

        // Find end (search after the start sequence)
        let search_from = start.len();
        if buf.len() < search_from {
            return Ok(None);
        }

        let end_pos = find_subsequence(&buf[search_from..], &end);
        match end_pos {
            None => Ok(None),
            Some(rel_pos) => {
                let total = search_from + rel_pos + end.len();
                let data = buf.split_to(total).to_vec();
                Ok(Some(self.wrap_frame(data)))
            }
        }
    }

    // -------------------------------------------------------------------
    // ModbusRtuGap
    // -------------------------------------------------------------------

    fn decode_modbus_gap(&mut self, buf: &mut BytesMut) -> Result<Option<RawFrame>, FrameError> {
        let baud_rate = match &self.config.framing {
            FramingRule::ModbusRtuGap { baud_rate } => *baud_rate,
            _ => unreachable!(),
        };

        let new_data = buf.len() != self.prev_buf_len;
        self.prev_buf_len = buf.len();

        if new_data {
            match self.last_data_time {
                None => {
                    self.last_data_time = Some(Instant::now());
                    return Ok(None);
                }
                Some(_) => {
                    // New data arrived — update the timestamp; the gap hasn't happened yet.
                    self.last_data_time = Some(Instant::now());
                    return Ok(None);
                }
            }
        }

        // No new data — check if gap threshold exceeded
        if let Some(last) = self.last_data_time {
            let threshold = modbus_gap_threshold(baud_rate);
            if last.elapsed() >= threshold {
                let data = buf.split_to(buf.len()).to_vec();
                self.last_data_time = None;
                self.prev_buf_len = 0;
                return Ok(Some(self.wrap_frame(data)));
            }
        }

        Ok(None)
    }

    // -------------------------------------------------------------------
    // Overflow handling
    // -------------------------------------------------------------------

    fn handle_overflow(&mut self, buf: &mut BytesMut) -> Result<Option<RawFrame>, FrameError> {
        match &self.config.framing {
            FramingRule::LengthPrefixed { start, .. } | FramingRule::Delimited { start, .. } => {
                // Try to resynchronize: find next start marker after position 0
                if let Some(pos) = find_subsequence(&buf[1..], start) {
                    let _ = buf.split_to(pos + 1);
                } else {
                    buf.clear();
                }
            }
            _ => {
                buf.clear();
            }
        }
        self.state = DecodeState::Head;
        self.partial_start = None;
        self.last_data_time = None;
        self.prev_buf_len = 0;
        Err(FrameError::Overflow)
    }

    // -------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------

    fn wrap_frame(&self, data: Vec<u8>) -> RawFrame {
        let checksum_valid = self
            .config
            .checksum
            .as_ref()
            .map(|ct| checksum::validate(&data, ct));

        RawFrame {
            data,
            timestamp: Utc::now(),
            checksum_valid,
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Find the first occurrence of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Read an unsigned integer from `bytes` (1, 2, or 4 bytes) using the given endianness.
fn read_uint(bytes: &[u8], endian: &Endian) -> usize {
    match bytes.len() {
        1 => bytes[0] as usize,
        2 => match endian {
            Endian::Big => u16::from_be_bytes([bytes[0], bytes[1]]) as usize,
            Endian::Little => u16::from_le_bytes([bytes[0], bytes[1]]) as usize,
        },
        4 => match endian {
            Endian::Big => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
            Endian::Little => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
        },
        _ => {
            // Fallback: big-endian arbitrary length
            let mut val: usize = 0;
            for &b in bytes {
                val = (val << 8) | (b as usize);
            }
            val
        }
    }
}

/// Calculate the Modbus RTU inter-frame gap threshold.
pub fn modbus_gap_threshold(baud_rate: Option<u32>) -> std::time::Duration {
    let base = match baud_rate {
        Some(br) if br <= 19200 => 3.5 * 11.0 / (br as f64),
        _ => 0.00175, // 1.75ms
    };
    let with_margin = base * 2.0;
    let millis = (with_margin * 1000.0).max(5.0);
    std::time::Duration::from_secs_f64(millis / 1000.0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::checksum::compute;
    use crate::protocol::types::{ChecksumType, FrameConfig, FramingRule};

    fn fixed_config(size: usize) -> FrameConfig {
        FrameConfig {
            name: "test".into(),
            framing: FramingRule::FixedSize { size },
            checksum: None,
            frame_timeout_ms: 5000,
            max_frame_size: 1024,
        }
    }

    fn length_prefixed_config(
        start: Vec<u8>,
        length_offset: usize,
        length_size: usize,
        length_endian: Endian,
        length_includes_header: bool,
        trailer_size: usize,
    ) -> FrameConfig {
        FrameConfig {
            name: "test".into(),
            framing: FramingRule::LengthPrefixed {
                start,
                length_offset,
                length_size,
                length_endian,
                length_includes_header,
                trailer_size,
            },
            checksum: None,
            frame_timeout_ms: 5000,
            max_frame_size: 1024,
        }
    }

    fn delimited_config(start: Vec<u8>, end: Vec<u8>) -> FrameConfig {
        FrameConfig {
            name: "test".into(),
            framing: FramingRule::Delimited { start, end },
            checksum: None,
            frame_timeout_ms: 5000,
            max_frame_size: 1024,
        }
    }

    // ---------------------------------------------------------------
    // FixedSize tests
    // ---------------------------------------------------------------

    #[test]
    fn fixed_size_two_frames_from_16_bytes() {
        let mut parser = FrameParser::new(fixed_config(8));
        let mut buf =
            BytesMut::from(&[1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16][..]);

        let f1 = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1.data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(f1.checksum_valid.is_none());

        let f2 = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f2.data, vec![9, 10, 11, 12, 13, 14, 15, 16]);

        assert!(buf.is_empty());
    }

    #[test]
    fn fixed_size_partial_returns_none() {
        let mut parser = FrameParser::new(fixed_config(8));
        let mut buf = BytesMut::from(&[1u8, 2, 3][..]);
        assert!(parser.decode(&mut buf).unwrap().is_none());
        assert_eq!(buf.len(), 3); // data preserved
    }

    // ---------------------------------------------------------------
    // LengthPrefixed tests
    // ---------------------------------------------------------------

    #[test]
    fn length_prefixed_basic() {
        // start=[0xAA], length at offset 1, 1 byte big-endian, not header-included, trailer 0
        // Frame: [0xAA, 0x03, 0x01, 0x02, 0x03]
        //   start=0xAA, length_offset=1, length_size=1 => length field = 0x03
        //   total = (1+1) + 3 + 0 = 5
        let config = length_prefixed_config(vec![0xAA], 1, 1, Endian::Big, false, 0);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0xAA, 0x03, 0x01, 0x02, 0x03][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0xAA, 0x03, 0x01, 0x02, 0x03]);
        assert!(buf.is_empty());
    }

    #[test]
    fn length_prefixed_includes_header() {
        // start=[0xAA], length at offset 1, 1 byte big-endian, header-included, trailer 0
        // Frame: [0xAA, 0x05, 0x01, 0x02, 0x03]
        //   length field = 0x05 (includes header)
        //   total = 5 + 0 = 5
        let config = length_prefixed_config(vec![0xAA], 1, 1, Endian::Big, true, 0);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0xAA, 0x05, 0x01, 0x02, 0x03][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0xAA, 0x05, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn length_prefixed_little_endian_2byte() {
        // start=[0xBB], length at offset 1, 2 bytes little-endian, not header-included, trailer 0
        // Frame: [0xBB, 0x02, 0x00, 0xAA, 0xBB]
        //   length field = 0x0002 (LE)
        //   total = (1+2) + 2 + 0 = 5
        let config = length_prefixed_config(vec![0xBB], 1, 2, Endian::Little, false, 0);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0xBB, 0x02, 0x00, 0xAA, 0xBB][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0xBB, 0x02, 0x00, 0xAA, 0xBB]);
    }

    #[test]
    fn length_prefixed_resynchronization() {
        // Garbage bytes [0xFF, 0xFF] before the actual frame
        let config = length_prefixed_config(vec![0xAA], 1, 1, Endian::Big, false, 0);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0xFF, 0xFF, 0xAA, 0x02, 0x01, 0x02][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0xAA, 0x02, 0x01, 0x02]);
        assert!(buf.is_empty());
    }

    #[test]
    fn length_prefixed_partial_head_then_complete() {
        let config = length_prefixed_config(vec![0xAA], 1, 1, Endian::Big, false, 0);
        let mut parser = FrameParser::new(config);

        // First call: only start byte + partial header
        let mut buf = BytesMut::from(&[0xAA][..]);
        assert!(parser.decode(&mut buf).unwrap().is_none());

        // Second call: extend buffer with rest
        buf.extend_from_slice(&[0x02, 0x01, 0x02]);
        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0xAA, 0x02, 0x01, 0x02]);
    }

    // ---------------------------------------------------------------
    // Delimited tests
    // ---------------------------------------------------------------

    #[test]
    fn delimited_basic() {
        // start=[0x3A], end=[0x0D, 0x0A]
        let config = delimited_config(vec![0x3A], vec![0x0D, 0x0A]);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0x3A, 0x41, 0x42, 0x43, 0x0D, 0x0A][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0x3A, 0x41, 0x42, 0x43, 0x0D, 0x0A]);
        assert!(buf.is_empty());
    }

    #[test]
    fn delimited_garbage_before_start() {
        let config = delimited_config(vec![0x3A], vec![0x0D, 0x0A]);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0xFF, 0xFE, 0x3A, 0x41, 0x0D, 0x0A][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![0x3A, 0x41, 0x0D, 0x0A]);
    }

    #[test]
    fn delimited_partial_returns_none() {
        let config = delimited_config(vec![0x3A], vec![0x0D, 0x0A]);
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0x3A, 0x41, 0x42][..]);

        assert!(parser.decode(&mut buf).unwrap().is_none());
    }

    // ---------------------------------------------------------------
    // Checksum tests
    // ---------------------------------------------------------------

    #[test]
    fn checksum_valid_crc16() {
        let payload = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
        let crc = compute(&payload, &ChecksumType::Crc16Modbus);
        let mut frame_data = payload.clone();
        frame_data.extend_from_slice(&crc);

        let config = FrameConfig {
            name: "test".into(),
            framing: FramingRule::FixedSize {
                size: frame_data.len(),
            },
            checksum: Some(ChecksumType::Crc16Modbus),
            frame_timeout_ms: 5000,
            max_frame_size: 1024,
        };
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&frame_data[..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.checksum_valid, Some(true));
    }

    #[test]
    fn checksum_invalid_crc16() {
        let frame_data = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xFF, 0xFF];

        let config = FrameConfig {
            name: "test".into(),
            framing: FramingRule::FixedSize {
                size: frame_data.len(),
            },
            checksum: Some(ChecksumType::Crc16Modbus),
            frame_timeout_ms: 5000,
            max_frame_size: 1024,
        };
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&frame_data[..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.checksum_valid, Some(false));
    }

    #[test]
    fn no_checksum_returns_none_valid() {
        let mut parser = FrameParser::new(fixed_config(4));
        let mut buf = BytesMut::from(&[1u8, 2, 3, 4][..]);

        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert!(frame.checksum_valid.is_none());
    }

    // ---------------------------------------------------------------
    // Partial frame across two decode calls
    // ---------------------------------------------------------------

    #[test]
    fn partial_frame_across_two_calls() {
        let mut parser = FrameParser::new(fixed_config(8));

        let mut buf = BytesMut::from(&[1u8, 2, 3, 4][..]);
        assert!(parser.decode(&mut buf).unwrap().is_none());

        buf.extend_from_slice(&[5, 6, 7, 8]);
        let frame = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    // ---------------------------------------------------------------
    // Overflow
    // ---------------------------------------------------------------

    #[test]
    fn overflow_returns_error() {
        let config = FrameConfig {
            name: "test".into(),
            framing: FramingRule::FixedSize { size: 100 },
            checksum: None,
            frame_timeout_ms: 5000,
            max_frame_size: 16,
        };
        let mut parser = FrameParser::new(config);
        let mut buf = BytesMut::from(&[0u8; 20][..]);

        let result = parser.decode(&mut buf);
        assert!(matches!(result, Err(FrameError::Overflow)));
    }

    // ---------------------------------------------------------------
    // Empty buffer
    // ---------------------------------------------------------------

    #[test]
    fn empty_buffer_returns_none() {
        let mut parser = FrameParser::new(fixed_config(8));
        let mut buf = BytesMut::new();
        assert!(parser.decode(&mut buf).unwrap().is_none());
    }

    // ---------------------------------------------------------------
    // Multiple frames in single buffer
    // ---------------------------------------------------------------

    #[test]
    fn multiple_frames_sequential_extraction() {
        let config = delimited_config(vec![0x3A], vec![0x0D, 0x0A]);
        let mut parser = FrameParser::new(config);
        // Two delimited frames back to back
        let mut buf = BytesMut::from(&[0x3A, 0x41, 0x0D, 0x0A, 0x3A, 0x42, 0x0D, 0x0A][..]);

        let f1 = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1.data, vec![0x3A, 0x41, 0x0D, 0x0A]);

        let f2 = parser.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f2.data, vec![0x3A, 0x42, 0x0D, 0x0A]);

        assert!(buf.is_empty());
    }

    // ---------------------------------------------------------------
    // ModbusRtuGap threshold calculation
    // ---------------------------------------------------------------

    #[test]
    fn modbus_gap_threshold_low_baud() {
        let d = modbus_gap_threshold(Some(9600));
        // 3.5 * 11 / 9600 = ~0.004010s, *2 = ~0.008021s = ~8.02ms
        assert!(d.as_millis() >= 8);
        assert!(d.as_millis() <= 10);
    }

    #[test]
    fn modbus_gap_threshold_high_baud() {
        let d = modbus_gap_threshold(Some(115200));
        // > 19200 => 1.75ms * 2 = 3.5ms, clamped to 5ms minimum
        assert_eq!(d.as_millis(), 5);
    }

    #[test]
    fn modbus_gap_threshold_none_baud() {
        let d = modbus_gap_threshold(None);
        assert_eq!(d.as_millis(), 5);
    }

    // ---------------------------------------------------------------
    // Reset
    // ---------------------------------------------------------------

    #[test]
    fn reset_clears_state() {
        let config = length_prefixed_config(vec![0xAA], 1, 1, Endian::Big, false, 0);
        let mut parser = FrameParser::new(config);

        // Start parsing but don't finish
        let mut buf = BytesMut::from(&[0xAA, 0x05][..]);
        let _ = parser.decode(&mut buf);

        parser.reset();

        // After reset, should be back in Head state
        let mut buf2 = BytesMut::from(&[0xAA, 0x02, 0x01, 0x02][..]);
        let frame = parser.decode(&mut buf2).unwrap().unwrap();
        assert_eq!(frame.data, vec![0xAA, 0x02, 0x01, 0x02]);
    }
}
