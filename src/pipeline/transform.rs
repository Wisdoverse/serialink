use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::serial::port::TimestampedLine;

/// A chunk of data flowing through the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataChunk {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub content: String,
    pub raw: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

impl DataChunk {
    /// Create a new DataChunk with the given content, timestamped to now.
    pub fn new(content: impl Into<String>) -> Self {
        let content = content.into();
        let raw = content.as_bytes().to_vec();
        Self {
            timestamp: chrono::Utc::now(),
            content,
            raw,
            metadata: HashMap::new(),
        }
    }
}

impl From<TimestampedLine> for DataChunk {
    fn from(line: TimestampedLine) -> Self {
        Self {
            timestamp: line.timestamp,
            content: line.content,
            raw: line.raw,
            metadata: line.metadata,
        }
    }
}

impl From<DataChunk> for TimestampedLine {
    fn from(chunk: DataChunk) -> Self {
        Self {
            timestamp: chunk.timestamp,
            content: chunk.content,
            raw: chunk.raw,
            metadata: chunk.metadata,
        }
    }
}

/// Trait for pipeline transforms that process data chunks.
#[async_trait::async_trait]
pub trait Transform: Send + Sync {
    /// Process a single input chunk, returning zero or more output chunks.
    async fn process(&self, input: DataChunk) -> Vec<DataChunk>;

    /// The name of this transform (used for logging/debugging).
    fn name(&self) -> &str;
}
