use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

use crate::serial::port::TimestampedLine;

/// Strategy for reading and framing serial data.
///
/// Implementations run inside `spawn_blocking` — no `.await` allowed.
pub trait ReadStrategy: Send + 'static {
    /// Read available data from the port and return framed lines/frames.
    fn read_frames(
        &mut self,
        port: &mut dyn serialport::SerialPort,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<TimestampedLine>>;

    /// Reset internal state (called on reconnect).
    fn reset(&mut self);
}

/// Line-oriented read strategy — extracts newline-delimited text lines.
///
/// This is the original `blocking_read_lines` logic extracted into a strategy.
pub struct LineReadStrategy {
    remainder: Vec<u8>,
}

impl LineReadStrategy {
    pub fn new() -> Self {
        Self {
            remainder: Vec::new(),
        }
    }
}

impl Default for LineReadStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadStrategy for LineReadStrategy {
    fn read_frames(
        &mut self,
        port: &mut dyn serialport::SerialPort,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<TimestampedLine>> {
        let mut buf = [0u8; 4096];
        let mut lines = Vec::new();

        // Read in a loop for a short burst to collect available data.
        for _ in 0..10 {
            if cancel.is_cancelled() {
                break;
            }
            match std::io::Read::read(port, &mut buf) {
                Ok(n) if n > 0 => {
                    self.remainder.extend_from_slice(&buf[..n]);
                }
                Ok(_) => break,
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                Err(e) => return Err(e.into()),
            }
        }

        // Split on newline boundaries, keeping incomplete trailing data in remainder.
        if !self.remainder.is_empty() {
            let now = Utc::now();
            let mut start = 0;
            for i in 0..self.remainder.len() {
                if self.remainder[i] == b'\n' {
                    let raw = self.remainder[start..=i].to_vec();
                    let content = String::from_utf8_lossy(&raw).trim_end().to_string();
                    lines.push(TimestampedLine {
                        timestamp: now,
                        content,
                        raw,
                        metadata: HashMap::new(),
                    });
                    start = i + 1;
                }
            }
            // Keep only the incomplete trailing bytes for next read.
            if start > 0 {
                self.remainder.drain(..start);
            }
        }

        Ok(lines)
    }

    fn reset(&mut self) {
        self.remainder.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock serial port that returns predefined data, then TimedOut.
    struct MockPort {
        data: Vec<u8>,
        pos: usize,
    }

    impl MockPort {
        fn new(data: Vec<u8>) -> Self {
            Self { data, pos: 0 }
        }
    }

    impl std::io::Read for MockPort {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "no more data",
                ));
            }
            let remaining = &self.data[self.pos..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl std::io::Write for MockPort {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl serialport::SerialPort for MockPort {
        fn name(&self) -> Option<String> {
            Some("mock".into())
        }
        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(115200)
        }
        fn data_bits(&self) -> serialport::Result<serialport::DataBits> {
            Ok(serialport::DataBits::Eight)
        }
        fn flow_control(&self) -> serialport::Result<serialport::FlowControl> {
            Ok(serialport::FlowControl::None)
        }
        fn parity(&self) -> serialport::Result<serialport::Parity> {
            Ok(serialport::Parity::None)
        }
        fn stop_bits(&self) -> serialport::Result<serialport::StopBits> {
            Ok(serialport::StopBits::One)
        }
        fn timeout(&self) -> std::time::Duration {
            std::time::Duration::from_millis(100)
        }
        fn set_baud_rate(&mut self, _: u32) -> serialport::Result<()> {
            Ok(())
        }
        fn set_data_bits(&mut self, _: serialport::DataBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_flow_control(&mut self, _: serialport::FlowControl) -> serialport::Result<()> {
            Ok(())
        }
        fn set_parity(&mut self, _: serialport::Parity) -> serialport::Result<()> {
            Ok(())
        }
        fn set_stop_bits(&mut self, _: serialport::StopBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_timeout(&mut self, _: std::time::Duration) -> serialport::Result<()> {
            Ok(())
        }
        fn write_request_to_send(&mut self, _: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn write_data_terminal_ready(&mut self, _: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn clear(&self, _: serialport::ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }
        fn try_clone(&self) -> serialport::Result<Box<dyn serialport::SerialPort>> {
            Err(serialport::Error::new(
                serialport::ErrorKind::Unknown,
                "mock cannot clone",
            ))
        }
        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }
        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn complete_lines_produce_correct_output() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut strategy = LineReadStrategy::new();
        let mut port = MockPort::new(b"hello\nworld\n".to_vec());

        let lines = strategy.read_frames(&mut port, &cancel).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content, "hello");
        assert_eq!(lines[0].raw, b"hello\n");
        assert_eq!(lines[1].content, "world");
        assert_eq!(lines[1].raw, b"world\n");
    }

    #[test]
    fn partial_lines_persist_in_remainder() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut strategy = LineReadStrategy::new();

        // First read: partial line
        let mut port1 = MockPort::new(b"hel".to_vec());
        let lines1 = strategy.read_frames(&mut port1, &cancel).unwrap();
        assert!(lines1.is_empty());

        // Second read: complete the line
        let mut port2 = MockPort::new(b"lo\n".to_vec());
        let lines2 = strategy.read_frames(&mut port2, &cancel).unwrap();
        assert_eq!(lines2.len(), 1);
        assert_eq!(lines2[0].content, "hello");
        assert_eq!(lines2[0].raw, b"hello\n");
    }

    #[test]
    fn reset_clears_remainder() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut strategy = LineReadStrategy::new();

        // Accumulate partial data
        let mut port1 = MockPort::new(b"partial".to_vec());
        let _ = strategy.read_frames(&mut port1, &cancel).unwrap();

        // Reset
        strategy.reset();

        // New data should not include old partial
        let mut port2 = MockPort::new(b"fresh\n".to_vec());
        let lines = strategy.read_frames(&mut port2, &cancel).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content, "fresh");
    }

    #[test]
    fn empty_read_returns_empty_vec() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut strategy = LineReadStrategy::new();
        let mut port = MockPort::new(Vec::new());

        let lines = strategy.read_frames(&mut port, &cancel).unwrap();
        assert!(lines.is_empty());
    }
}
