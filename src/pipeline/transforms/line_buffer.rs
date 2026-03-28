use tokio::sync::Mutex;

use crate::pipeline::transform::{DataChunk, Transform};

/// Splits incoming data into individual lines, buffering partial lines
/// until a complete newline-terminated line is received.
pub struct LineBufferTransform {
    /// Partial line buffer from previous calls.
    buffer: Mutex<String>,
    /// Encoding name (currently only UTF-8 is supported, but stored for
    /// future extensibility).
    encoding: String,
}

impl LineBufferTransform {
    /// Create a new `LineBufferTransform` with the given encoding.
    pub fn new(encoding: String) -> Self {
        Self {
            buffer: Mutex::new(String::new()),
            encoding,
        }
    }

    /// Returns the configured encoding name.
    pub fn encoding(&self) -> &str {
        &self.encoding
    }
}

#[async_trait::async_trait]
impl Transform for LineBufferTransform {
    async fn process(&self, input: DataChunk) -> Vec<DataChunk> {
        let mut buffer = self.buffer.lock().await;

        // Prepend any previously buffered partial line.
        buffer.push_str(&input.content);

        let mut chunks = Vec::new();
        let buf_contents = buffer.clone();

        // Split on newlines (\n handles both \r\n after we strip \r).
        let mut lines: Vec<&str> = buf_contents.split('\n').collect();

        // If the input did NOT end with a newline, the last element is a
        // partial line that we need to buffer for the next call.
        let remainder = if buf_contents.ends_with('\n') {
            // The split will produce an empty trailing element; discard it.
            lines.pop();
            String::new()
        } else {
            // Last element is a partial line — buffer it.
            lines.pop().unwrap_or("").to_string()
        };

        for line in lines {
            // Strip trailing \r if the original separator was \r\n.
            let line = line.strip_suffix('\r').unwrap_or(line);

            let mut chunk = DataChunk {
                timestamp: input.timestamp,
                content: line.to_string(),
                raw: line.as_bytes().to_vec(),
                metadata: input.metadata.clone(),
            };
            chunk
                .metadata
                .insert("encoding".to_string(), self.encoding.clone());
            chunks.push(chunk);
        }

        // Store the remainder for next time.
        *buffer = remainder;

        chunks
    }

    fn name(&self) -> &str {
        "line_buffer"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transform::DataChunk;

    #[tokio::test]
    async fn test_complete_lines() {
        let t = LineBufferTransform::new("utf-8".into());
        let input = DataChunk::new("hello\nworld\n");
        let output = t.process(input).await;
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].content, "hello");
        assert_eq!(output[1].content, "world");
    }

    #[tokio::test]
    async fn test_partial_line_buffering() {
        let t = LineBufferTransform::new("utf-8".into());
        // First call: partial line
        let out1 = t.process(DataChunk::new("hel")).await;
        assert_eq!(out1.len(), 0); // nothing complete yet
                                   // Second call: completes the line
        let out2 = t.process(DataChunk::new("lo\n")).await;
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].content, "hello");
    }

    #[tokio::test]
    async fn test_crlf_handling() {
        let t = LineBufferTransform::new("utf-8".into());
        let input = DataChunk::new("line1\r\nline2\r\n");
        let output = t.process(input).await;
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].content, "line1");
        assert_eq!(output[1].content, "line2");
    }

    #[tokio::test]
    async fn test_empty_input() {
        let t = LineBufferTransform::new("utf-8".into());
        let output = t.process(DataChunk::new("")).await;
        assert_eq!(output.len(), 0);
    }

    #[tokio::test]
    async fn test_multiple_partial_accumulation() {
        let t = LineBufferTransform::new("utf-8".into());
        let _ = t.process(DataChunk::new("aa")).await;
        let _ = t.process(DataChunk::new("bb")).await;
        let out = t.process(DataChunk::new("cc\n")).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "aabbcc");
    }

    #[tokio::test]
    async fn test_mixed_complete_and_partial() {
        let t = LineBufferTransform::new("utf-8".into());
        let out = t.process(DataChunk::new("line1\npartial")).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "line1");
        // Complete the partial
        let out2 = t.process(DataChunk::new(" end\n")).await;
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].content, "partial end");
    }
}
