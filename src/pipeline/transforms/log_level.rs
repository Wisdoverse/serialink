use regex::Regex;

use crate::pipeline::transform::{DataChunk, Transform};

/// The log format to use when parsing log levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// ESP-IDF format: `E (123) tag: message` — first char is the level.
    EspIdf,
    /// Syslog format: `<level> message`.
    Syslog,
    /// Generic: scan for ERROR, WARN, INFO, DEBUG, TRACE keywords.
    Generic,
}

/// Parses log levels from common formats and adds a `log_level` metadata key.
pub struct LogLevelTransform {
    format: LogFormat,
    /// Pre-compiled regex for ESP-IDF format.
    esp_idf_re: Regex,
    /// Pre-compiled regex for syslog format.
    syslog_re: Regex,
}

impl LogLevelTransform {
    pub fn new(format: LogFormat) -> Self {
        Self {
            format,
            esp_idf_re: Regex::new(r"^([EWIDV])\s*\(").unwrap(),
            syslog_re: Regex::new(r"(?i)^<(emerg|alert|crit|err|warning|notice|info|debug)>")
                .unwrap(),
        }
    }

    /// Try to parse the log level from an ESP-IDF formatted line.
    fn parse_esp_idf(&self, content: &str) -> Option<&'static str> {
        self.esp_idf_re
            .captures(content)
            .and_then(|caps| match caps.get(1)?.as_str() {
                "E" => Some("error"),
                "W" => Some("warn"),
                "I" => Some("info"),
                "D" => Some("debug"),
                "V" => Some("verbose"),
                _ => None,
            })
    }

    /// Try to parse the log level from a syslog formatted line.
    fn parse_syslog(&self, content: &str) -> Option<&'static str> {
        self.syslog_re.captures(content).and_then(|caps| {
            let level = caps.get(1)?.as_str().to_lowercase();
            match level.as_str() {
                "emerg" | "alert" | "crit" | "err" => Some("error"),
                "warning" | "notice" => Some("warn"),
                "info" => Some("info"),
                "debug" => Some("debug"),
                _ => None,
            }
        })
    }

    /// Try to parse the log level by scanning for common keywords.
    fn parse_generic(&self, content: &str) -> Option<&'static str> {
        // Use uppercase comparison for keyword matching.
        let upper = content.to_uppercase();

        // Check in order of specificity/severity.
        if upper.contains("ERROR") {
            Some("error")
        } else if upper.contains("WARN") {
            Some("warn")
        } else if upper.contains("INFO") {
            Some("info")
        } else if upper.contains("DEBUG") {
            Some("debug")
        } else if upper.contains("TRACE") {
            Some("trace")
        } else {
            None
        }
    }

    /// Parse the log level using the configured format.
    fn parse_level(&self, content: &str) -> Option<&'static str> {
        match self.format {
            LogFormat::EspIdf => self.parse_esp_idf(content),
            LogFormat::Syslog => self.parse_syslog(content),
            LogFormat::Generic => self.parse_generic(content),
        }
    }
}

#[async_trait::async_trait]
impl Transform for LogLevelTransform {
    async fn process(&self, mut input: DataChunk) -> Vec<DataChunk> {
        if let Some(level) = self.parse_level(&input.content) {
            input
                .metadata
                .insert("log_level".to_string(), level.to_string());
        }
        vec![input]
    }

    fn name(&self) -> &str {
        "log_level"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transform::DataChunk;

    #[tokio::test]
    async fn test_esp_idf_error() {
        let t = LogLevelTransform::new(LogFormat::EspIdf);
        let out = t
            .process(DataChunk::new("E (1234) wifi: connection failed"))
            .await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
    }

    #[tokio::test]
    async fn test_esp_idf_info() {
        let t = LogLevelTransform::new(LogFormat::EspIdf);
        let out = t.process(DataChunk::new("I (567) main: started")).await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "info");
    }

    #[tokio::test]
    async fn test_esp_idf_warn() {
        let t = LogLevelTransform::new(LogFormat::EspIdf);
        let out = t.process(DataChunk::new("W (89) heap: low memory")).await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "warn");
    }

    #[tokio::test]
    async fn test_esp_idf_debug_verbose() {
        let t = LogLevelTransform::new(LogFormat::EspIdf);
        let d = t.process(DataChunk::new("D (1) tag: debug msg")).await;
        assert_eq!(d[0].metadata.get("log_level").unwrap(), "debug");
        let v = t.process(DataChunk::new("V (1) tag: verbose msg")).await;
        assert_eq!(v[0].metadata.get("log_level").unwrap(), "verbose");
    }

    #[tokio::test]
    async fn test_esp_idf_no_match() {
        let t = LogLevelTransform::new(LogFormat::EspIdf);
        let out = t.process(DataChunk::new("random line")).await;
        assert!(out[0].metadata.get("log_level").is_none());
    }

    #[tokio::test]
    async fn test_generic_error() {
        let t = LogLevelTransform::new(LogFormat::Generic);
        let out = t
            .process(DataChunk::new("[2024-01-01] ERROR: failed"))
            .await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
    }

    #[tokio::test]
    async fn test_generic_warn() {
        let t = LogLevelTransform::new(LogFormat::Generic);
        let out = t.process(DataChunk::new("WARNING: check config")).await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "warn");
    }

    #[tokio::test]
    async fn test_generic_case_insensitive() {
        let t = LogLevelTransform::new(LogFormat::Generic);
        let out = t.process(DataChunk::new("error in lower case")).await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
    }

    #[tokio::test]
    async fn test_generic_no_level() {
        let t = LogLevelTransform::new(LogFormat::Generic);
        let out = t.process(DataChunk::new("just a normal line")).await;
        assert!(out[0].metadata.get("log_level").is_none());
    }

    #[tokio::test]
    async fn test_syslog_err() {
        let t = LogLevelTransform::new(LogFormat::Syslog);
        let out = t.process(DataChunk::new("<err> something broke")).await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
    }

    #[tokio::test]
    async fn test_syslog_info() {
        let t = LogLevelTransform::new(LogFormat::Syslog);
        let out = t.process(DataChunk::new("<info> system ready")).await;
        assert_eq!(out[0].metadata.get("log_level").unwrap(), "info");
    }
}
