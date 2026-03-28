use anyhow::{anyhow, Result};
use regex::RegexBuilder;

use crate::pipeline::transform::{DataChunk, Transform};

/// Maximum length for a regex pattern string (chars).
const MAX_PATTERN_LEN: usize = 1024;

/// Whether matching lines are kept or discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    /// Keep only lines that match the pattern.
    Include,
    /// Drop lines that match the pattern.
    Exclude,
}

/// Filters data chunks based on a regular expression.
pub struct RegexFilterTransform {
    regex: regex::Regex,
    mode: FilterMode,
}

impl RegexFilterTransform {
    /// Create a new filter from a regex pattern string and filter mode.
    ///
    /// Enforces a 1024-char pattern length limit and 1 MB compiled size
    /// limit to prevent ReDoS from untrusted input.
    pub fn new(pattern: &str, mode: FilterMode) -> Result<Self> {
        if pattern.len() > MAX_PATTERN_LEN {
            return Err(anyhow!(
                "regex pattern too long ({} chars, max {})",
                pattern.len(),
                MAX_PATTERN_LEN
            ));
        }
        let regex = RegexBuilder::new(pattern)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20)
            .build()?;
        Ok(Self { regex, mode })
    }
}

#[async_trait::async_trait]
impl Transform for RegexFilterTransform {
    async fn process(&self, input: DataChunk) -> Vec<DataChunk> {
        let matches = self.regex.is_match(&input.content);

        let keep = match self.mode {
            FilterMode::Include => matches,
            FilterMode::Exclude => !matches,
        };

        if keep {
            vec![input]
        } else {
            vec![]
        }
    }

    fn name(&self) -> &str {
        "regex_filter"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transform::DataChunk;

    #[tokio::test]
    async fn test_include_matching() {
        let t = RegexFilterTransform::new("ERROR", FilterMode::Include).unwrap();
        let out = t.process(DataChunk::new("ERROR: something failed")).await;
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn test_include_non_matching() {
        let t = RegexFilterTransform::new("ERROR", FilterMode::Include).unwrap();
        let out = t.process(DataChunk::new("INFO: all good")).await;
        assert_eq!(out.len(), 0);
    }

    #[tokio::test]
    async fn test_exclude_matching() {
        let t = RegexFilterTransform::new("VERBOSE", FilterMode::Exclude).unwrap();
        let out = t.process(DataChunk::new("VERBOSE: noisy log")).await;
        assert_eq!(out.len(), 0);
    }

    #[tokio::test]
    async fn test_exclude_non_matching() {
        let t = RegexFilterTransform::new("VERBOSE", FilterMode::Exclude).unwrap();
        let out = t.process(DataChunk::new("ERROR: keep this")).await;
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn test_regex_pattern() {
        let t = RegexFilterTransform::new(r"^\[(\d+)\]", FilterMode::Include).unwrap();
        let out1 = t.process(DataChunk::new("[123] matched")).await;
        assert_eq!(out1.len(), 1);
        let out2 = t.process(DataChunk::new("no match")).await;
        assert_eq!(out2.len(), 0);
    }

    #[test]
    fn test_invalid_regex() {
        let result = RegexFilterTransform::new("[invalid", FilterMode::Include);
        assert!(result.is_err());
    }
}
