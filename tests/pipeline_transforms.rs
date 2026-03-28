//! Integration tests for pipeline transforms, From conversions, and config deserialization.

use serialink::pipeline::engine::{
    FilterModeConfig, LogFormatConfig, Pipeline, PipelineStepConfig,
};
use serialink::pipeline::transform::{DataChunk, Transform};
use serialink::pipeline::transforms::regex_filter::{FilterMode, RegexFilterTransform};
use serialink::serial::port::TimestampedLine;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// From conversions between TimestampedLine and DataChunk
// ---------------------------------------------------------------------------

#[test]
fn timestamped_line_to_datachunk_preserves_all_fields() {
    let line = TimestampedLine {
        timestamp: chrono::Utc::now(),
        content: "hello".into(),
        raw: vec![104, 101, 108, 108, 111],
        metadata: HashMap::new(),
    };
    let ts = line.timestamp;
    let chunk: DataChunk = line.into();
    assert_eq!(chunk.timestamp, ts);
    assert_eq!(chunk.content, "hello");
    assert_eq!(chunk.raw, vec![104, 101, 108, 108, 111]);
    assert!(chunk.metadata.is_empty());
}

#[test]
fn datachunk_to_timestamped_line_preserves_metadata() {
    let mut meta = HashMap::new();
    meta.insert("log_level".into(), "error".into());
    let chunk = DataChunk {
        timestamp: chrono::Utc::now(),
        content: "ERROR: crash".into(),
        raw: b"ERROR: crash".to_vec(),
        metadata: meta,
    };
    let line: TimestampedLine = chunk.into();
    assert_eq!(line.metadata.get("log_level").unwrap(), "error");
    assert_eq!(line.content, "ERROR: crash");
}

#[test]
fn round_trip_preserves_divergent_raw() {
    // Simulate non-UTF-8 serial data where raw differs from content
    let line = TimestampedLine {
        timestamp: chrono::Utc::now(),
        content: "lossy\u{FFFD}data".into(),
        raw: vec![0x6C, 0x6F, 0x73, 0x73, 0x79, 0xFF, 0x64, 0x61, 0x74, 0x61],
        metadata: HashMap::new(),
    };
    let chunk: DataChunk = line.into();
    assert_eq!(
        chunk.raw,
        vec![0x6C, 0x6F, 0x73, 0x73, 0x79, 0xFF, 0x64, 0x61, 0x74, 0x61]
    );
    assert_eq!(chunk.content, "lossy\u{FFFD}data");

    let back: TimestampedLine = chunk.into();
    assert_eq!(
        back.raw,
        vec![0x6C, 0x6F, 0x73, 0x73, 0x79, 0xFF, 0x64, 0x61, 0x74, 0x61]
    );
}

// ---------------------------------------------------------------------------
// TimestampedLine JSON serialization
// ---------------------------------------------------------------------------

#[test]
fn json_omits_metadata_when_empty() {
    let line = TimestampedLine {
        timestamp: chrono::Utc::now(),
        content: "test".into(),
        raw: b"test".to_vec(),
        metadata: HashMap::new(),
    };
    let json = serde_json::to_string(&line).unwrap();
    assert!(!json.contains("metadata"));
}

#[test]
fn json_includes_metadata_when_populated() {
    let mut meta = HashMap::new();
    meta.insert("log_level".into(), "info".into());
    let line = TimestampedLine {
        timestamp: chrono::Utc::now(),
        content: "test".into(),
        raw: b"test".to_vec(),
        metadata: meta,
    };
    let json = serde_json::to_string(&line).unwrap();
    assert!(json.contains("metadata"));
    assert!(json.contains("log_level"));
}

// ---------------------------------------------------------------------------
// RegexFilterTransform security constraints
// ---------------------------------------------------------------------------

#[test]
fn regex_filter_rejects_pattern_over_1024_chars() {
    let long_pattern = "a".repeat(1025);
    let result = RegexFilterTransform::new(&long_pattern, FilterMode::Include);
    assert!(result.is_err());
    let err_msg = format!("{}", result.err().unwrap());
    assert!(err_msg.contains("too long"));
}

#[test]
fn regex_filter_accepts_pattern_at_1024_chars() {
    let pattern = "a".repeat(1024);
    let result = RegexFilterTransform::new(&pattern, FilterMode::Include);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Pipeline::from_config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn from_config_with_all_step_types() {
    let steps = vec![
        PipelineStepConfig::Timestamp,
        PipelineStepConfig::LogLevel {
            format: LogFormatConfig::Generic,
        },
        PipelineStepConfig::RegexFilter {
            pattern: "ERROR".into(),
            mode: FilterModeConfig::Include,
        },
        PipelineStepConfig::LineBuffer {
            encoding: "utf-8".into(),
        },
    ];
    let pipeline = Pipeline::from_config(&steps).unwrap();
    // Process a line with ERROR — should pass through all 4 transforms
    let out = pipeline.process(DataChunk::new("ERROR: crash\n")).await;
    assert!(!out.is_empty());
}

#[tokio::test]
async fn from_config_line_buffer_default_encoding() {
    let toml_str = r#"
    [[pipeline]]
    type = "line_buffer"
    "#;
    let config: TestPipelineConfig = toml::from_str(toml_str).unwrap();
    match &config.pipeline[0] {
        PipelineStepConfig::LineBuffer { encoding } => {
            assert_eq!(encoding, "utf-8");
        }
        _ => panic!("Expected LineBuffer"),
    }
}

#[derive(serde::Deserialize)]
struct TestPipelineConfig {
    pipeline: Vec<PipelineStepConfig>,
}

#[test]
fn from_config_invalid_regex_returns_error() {
    let steps = vec![PipelineStepConfig::RegexFilter {
        pattern: "[invalid".into(),
        mode: FilterModeConfig::Include,
    }];
    assert!(Pipeline::from_config(&steps).is_err());
}

// ---------------------------------------------------------------------------
// TOML deserialization
// ---------------------------------------------------------------------------

#[test]
fn toml_deserialize_all_variants() {
    let toml_str = r#"
    [[pipeline]]
    type = "timestamp"

    [[pipeline]]
    type = "log_level"
    format = "esp_idf"

    [[pipeline]]
    type = "regex_filter"
    pattern = "ERROR"
    mode = "include"

    [[pipeline]]
    type = "line_buffer"
    encoding = "ascii"
    "#;
    let config: TestPipelineConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.pipeline.len(), 4);
}

#[test]
fn toml_unknown_type_produces_error() {
    let toml_str = r#"
    [[pipeline]]
    type = "unknown_transform"
    "#;
    let result: Result<TestPipelineConfig, _> = toml::from_str(toml_str);
    assert!(result.is_err());
}

#[test]
fn toml_regex_filter_missing_pattern_produces_error() {
    let toml_str = r#"
    [[pipeline]]
    type = "regex_filter"
    mode = "include"
    "#;
    let result: Result<TestPipelineConfig, _> = toml::from_str(toml_str);
    assert!(result.is_err());
}

#[test]
fn toml_filter_mode_defaults_to_include() {
    let toml_str = r#"
    [[pipeline]]
    type = "regex_filter"
    pattern = "ERROR"
    "#;
    let config: TestPipelineConfig = toml::from_str(toml_str).unwrap();
    match &config.pipeline[0] {
        PipelineStepConfig::RegexFilter { mode, .. } => {
            assert!(matches!(mode, FilterModeConfig::Include));
        }
        _ => panic!("Expected RegexFilter"),
    }
}

// ---------------------------------------------------------------------------
// LogLevelTransform — syslog edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn syslog_all_severity_mappings() {
    use serialink::pipeline::transforms::log_level::{LogFormat, LogLevelTransform};

    let t = LogLevelTransform::new(LogFormat::Syslog);

    let cases = vec![
        ("<emerg> system down", "error"),
        ("<alert> disk full", "error"),
        ("<crit> kernel panic", "error"),
        ("<err> something broke", "error"),
        ("<warning> high load", "warn"),
        ("<notice> user login", "warn"),
        ("<info> system ready", "info"),
        ("<debug> verbose output", "debug"),
    ];

    for (input, expected_level) in cases {
        let out = t.process(DataChunk::new(input)).await;
        assert_eq!(
            out[0].metadata.get("log_level").unwrap(),
            expected_level,
            "Failed for input: {}",
            input
        );
    }
}

#[tokio::test]
async fn syslog_case_insensitive() {
    use serialink::pipeline::transforms::log_level::{LogFormat, LogLevelTransform};
    let t = LogLevelTransform::new(LogFormat::Syslog);
    let out = t.process(DataChunk::new("<ERR> uppercase")).await;
    assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");
}

#[tokio::test]
async fn syslog_unrecognized_level() {
    use serialink::pipeline::transforms::log_level::{LogFormat, LogLevelTransform};
    let t = LogLevelTransform::new(LogFormat::Syslog);
    let out = t.process(DataChunk::new("<foo> unknown")).await;
    assert!(out[0].metadata.get("log_level").is_none());
}

// ---------------------------------------------------------------------------
// Transform ordering sensitivity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ordering_log_level_then_regex_filter() {
    let steps = vec![
        PipelineStepConfig::LogLevel {
            format: LogFormatConfig::Generic,
        },
        PipelineStepConfig::RegexFilter {
            pattern: "ERROR".into(),
            mode: FilterModeConfig::Include,
        },
    ];
    let pipeline = Pipeline::from_config(&steps).unwrap();

    // "ERROR: crash" matches the regex on content -> passes, has log_level metadata
    let out = pipeline.process(DataChunk::new("ERROR: crash")).await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].metadata.get("log_level").unwrap(), "error");

    // "INFO: ok" does NOT match "ERROR" regex on content -> filtered out
    let out2 = pipeline.process(DataChunk::new("INFO: ok")).await;
    assert_eq!(out2.len(), 0);
}

#[tokio::test]
async fn ordering_regex_filter_then_log_level() {
    let steps = vec![
        PipelineStepConfig::RegexFilter {
            pattern: "INFO".into(),
            mode: FilterModeConfig::Include,
        },
        PipelineStepConfig::LogLevel {
            format: LogFormatConfig::Generic,
        },
    ];
    let pipeline = Pipeline::from_config(&steps).unwrap();

    // "ERROR: crash" does NOT match "INFO" -> filtered out before LogLevel runs
    let out = pipeline.process(DataChunk::new("ERROR: crash")).await;
    assert_eq!(out.len(), 0);

    // "INFO: ok" matches -> passes, gets log_level metadata
    let out2 = pipeline.process(DataChunk::new("INFO: ok")).await;
    assert_eq!(out2.len(), 1);
    assert_eq!(out2[0].metadata.get("log_level").unwrap(), "info");
}
