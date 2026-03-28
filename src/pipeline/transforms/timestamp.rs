use chrono::Utc;

use crate::pipeline::transform::{DataChunk, Transform};

/// Stamps each chunk with the current UTC time and adds an ISO 8601
/// `host_timestamp` entry to metadata.
pub struct TimestampTransform;

impl TimestampTransform {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TimestampTransform {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Transform for TimestampTransform {
    async fn process(&self, mut input: DataChunk) -> Vec<DataChunk> {
        let now = Utc::now();
        input.timestamp = now;
        input
            .metadata
            .insert("host_timestamp".to_string(), now.to_rfc3339());
        vec![input]
    }

    fn name(&self) -> &str {
        "timestamp"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transform::DataChunk;

    #[tokio::test]
    async fn test_timestamp_updates() {
        let t = TimestampTransform::new();
        let old_chunk = DataChunk::new("test");
        let old_ts = old_chunk.timestamp;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let out = t.process(old_chunk).await;
        assert!(out[0].timestamp >= old_ts);
    }

    #[tokio::test]
    async fn test_adds_host_timestamp_metadata() {
        let t = TimestampTransform::new();
        let out = t.process(DataChunk::new("test")).await;
        assert!(out[0].metadata.contains_key("host_timestamp"));
        // Should be a valid RFC3339 string
        let ts = out[0].metadata.get("host_timestamp").unwrap();
        assert!(ts.contains("T"));
    }

    #[tokio::test]
    async fn test_preserves_content() {
        let t = TimestampTransform::new();
        let out = t.process(DataChunk::new("keep this")).await;
        assert_eq!(out[0].content, "keep this");
    }
}
