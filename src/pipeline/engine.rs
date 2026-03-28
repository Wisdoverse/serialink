use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::transform::{DataChunk, Transform};
use super::transforms;

/// Configuration for a single pipeline step, used by `Pipeline::from_config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineStepConfig {
    LineBuffer {
        #[serde(default = "default_encoding")]
        encoding: String,
    },
    Timestamp,
    RegexFilter {
        pattern: String,
        #[serde(default)]
        mode: FilterModeConfig,
    },
    LogLevel {
        #[serde(default)]
        format: LogFormatConfig,
    },
}

fn default_encoding() -> String {
    "utf-8".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterModeConfig {
    #[default]
    Include,
    Exclude,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormatConfig {
    EspIdf,
    Syslog,
    #[default]
    Generic,
}

/// A data-processing pipeline composed of ordered transforms.
pub struct Pipeline {
    transforms: Vec<Box<dyn Transform>>,
}

impl Pipeline {
    /// Create a new, empty pipeline.
    pub fn new() -> Self {
        Self {
            transforms: Vec::new(),
        }
    }

    /// Append a transform to the end of the pipeline.
    pub fn add_transform(&mut self, t: Box<dyn Transform>) {
        self.transforms.push(t);
    }

    /// Process a single input chunk through all transforms in order.
    ///
    /// Each transform may produce zero or more output chunks. Every output
    /// chunk from one stage is fed into the next stage. The final collected
    /// outputs are returned.
    pub async fn process(&self, input: DataChunk) -> Vec<DataChunk> {
        let mut chunks = vec![input];

        for transform in &self.transforms {
            let mut next_chunks = Vec::new();
            for chunk in chunks {
                let results = transform.process(chunk).await;
                next_chunks.extend(results);
            }
            chunks = next_chunks;

            // Short-circuit if there is nothing left to process.
            if chunks.is_empty() {
                break;
            }
        }

        chunks
    }

    /// Build a pipeline from a list of step configurations.
    pub fn from_config(steps: &[PipelineStepConfig]) -> Result<Self> {
        let mut pipeline = Self::new();

        for step in steps {
            let transform: Box<dyn Transform> = match step {
                PipelineStepConfig::LineBuffer { encoding } => Box::new(
                    transforms::line_buffer::LineBufferTransform::new(encoding.clone()),
                ),
                PipelineStepConfig::Timestamp => {
                    Box::new(transforms::timestamp::TimestampTransform::new())
                }
                PipelineStepConfig::RegexFilter { pattern, mode } => {
                    let filter_mode = match mode {
                        FilterModeConfig::Include => transforms::regex_filter::FilterMode::Include,
                        FilterModeConfig::Exclude => transforms::regex_filter::FilterMode::Exclude,
                    };
                    Box::new(
                        transforms::regex_filter::RegexFilterTransform::new(pattern, filter_mode)
                            .map_err(|e| anyhow!("invalid regex pattern '{}': {}", pattern, e))?,
                    )
                }
                PipelineStepConfig::LogLevel { format } => {
                    let log_format = match format {
                        LogFormatConfig::EspIdf => transforms::log_level::LogFormat::EspIdf,
                        LogFormatConfig::Syslog => transforms::log_level::LogFormat::Syslog,
                        LogFormatConfig::Generic => transforms::log_level::LogFormat::Generic,
                    };
                    Box::new(transforms::log_level::LogLevelTransform::new(log_format))
                }
            };
            pipeline.add_transform(transform);
        }

        Ok(pipeline)
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transform::DataChunk;

    #[tokio::test]
    async fn test_empty_pipeline_passthrough() {
        let pipeline = Pipeline::new();
        let chunk = DataChunk::new("hello");
        let output = pipeline.process(chunk).await;
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].content, "hello");
    }

    #[tokio::test]
    async fn test_pipeline_with_filter() {
        let mut pipeline = Pipeline::new();
        pipeline.add_transform(Box::new(
            crate::pipeline::transforms::regex_filter::RegexFilterTransform::new(
                "ERROR",
                crate::pipeline::transforms::regex_filter::FilterMode::Include,
            )
            .unwrap(),
        ));
        let out1 = pipeline.process(DataChunk::new("ERROR: bad")).await;
        assert_eq!(out1.len(), 1);
        let out2 = pipeline.process(DataChunk::new("INFO: fine")).await;
        assert_eq!(out2.len(), 0);
    }

    #[tokio::test]
    async fn test_pipeline_chaining() {
        let mut pipeline = Pipeline::new();
        // First: log level parser
        pipeline.add_transform(Box::new(
            crate::pipeline::transforms::log_level::LogLevelTransform::new(
                crate::pipeline::transforms::log_level::LogFormat::Generic,
            ),
        ));
        // Second: filter only errors
        pipeline.add_transform(Box::new(
            crate::pipeline::transforms::regex_filter::RegexFilterTransform::new(
                "ERROR",
                crate::pipeline::transforms::regex_filter::FilterMode::Include,
            )
            .unwrap(),
        ));
        let out = pipeline.process(DataChunk::new("ERROR: crash")).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
    }

    #[tokio::test]
    async fn test_from_config() {
        let steps = vec![
            PipelineStepConfig::Timestamp,
            PipelineStepConfig::LogLevel {
                format: LogFormatConfig::Generic,
            },
        ];
        let pipeline = Pipeline::from_config(&steps).unwrap();
        let out = pipeline.process(DataChunk::new("ERROR: test")).await;
        assert_eq!(out.len(), 1);
        assert!(out[0].metadata.contains_key("host_timestamp"));
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
    }

    #[tokio::test]
    async fn test_from_config_with_filter() {
        let steps = vec![PipelineStepConfig::RegexFilter {
            pattern: "ERROR".to_string(),
            mode: FilterModeConfig::Include,
        }];
        let pipeline = Pipeline::from_config(&steps).unwrap();
        let out = pipeline.process(DataChunk::new("INFO: skip")).await;
        assert_eq!(out.len(), 0);
    }

    #[tokio::test]
    async fn test_from_config_invalid_regex() {
        let steps = vec![PipelineStepConfig::RegexFilter {
            pattern: "[invalid".to_string(),
            mode: FilterModeConfig::Include,
        }];
        assert!(Pipeline::from_config(&steps).is_err());
    }
}
