use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::time::timeout;
use tracing::info;

use crate::config::ProtocolConfig;
use crate::exit_codes;
use crate::harness::executor;
use crate::harness::schema::HarnessConfig;
use crate::protocol::format;
use crate::serial::discovery;
use crate::serial::manager::SessionManager;
use crate::serial::port::PortConfig;

pub async fn cmd_list(json: bool) -> Result<()> {
    let ports = discovery::list_ports()?;

    if json {
        println!("{}", serde_json::to_string(&ports)?);
    } else if ports.is_empty() {
        println!("No serial ports found.");
    } else {
        println!(
            "{:<20} {:<12} {:<10} {:<10} DESCRIPTION",
            "PORT", "TYPE", "VID", "PID"
        );
        println!("{}", "-".repeat(70));
        for p in &ports {
            println!(
                "{:<20} {:<12} {:<10} {:<10} {}",
                p.name,
                p.port_type,
                p.vid.map(|v| format!("{:04X}", v)).unwrap_or_default(),
                p.pid.map(|v| format!("{:04X}", v)).unwrap_or_default(),
                p.manufacturer.as_deref().unwrap_or(""),
            );
        }
        println!("\nFound {} port(s).", ports.len());
    }

    Ok(())
}

pub async fn cmd_monitor(
    port_path: String,
    baud_rate: u32,
    json: bool,
    filter: Option<String>,
    duration: Option<u64>,
    pipeline: Option<std::sync::Arc<crate::pipeline::engine::Pipeline>>,
    protocol_config: Option<ProtocolConfig>,
) -> Result<()> {
    let filter_regex = filter
        .as_deref()
        .map(|p| {
            if p.len() > 1024 {
                return Err(anyhow!("Filter regex too long (max 1024 chars)"));
            }
            regex::RegexBuilder::new(p)
                .size_limit(1 << 20)
                .dfa_size_limit(1 << 20)
                .build()
                .map_err(|e| anyhow!("Invalid filter regex: {}", e))
        })
        .transpose()?;

    let port_config = PortConfig {
        baud_rate,
        ..PortConfig::default()
    };

    let manager = SessionManager::new(pipeline, protocol_config);
    let session_id = manager
        .create_session(port_path.clone(), port_config, None)
        .await?;
    let conn = manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| anyhow!("Failed to get session"))?;

    info!(port = %port_path, baud = baud_rate, "Monitoring started");

    if !json {
        eprintln!(
            "Monitoring {} at {} baud. Press Ctrl+C to stop.",
            port_path, baud_rate
        );
    }

    let mut rx = conn.subscribe();

    let monitor_fut = async {
        loop {
            match rx.recv().await {
                Ok(line) => {
                    if let Some(ref re) = filter_regex {
                        if !re.is_match(format::matchable_content(&line)) {
                            continue;
                        }
                    }

                    if json {
                        let json_line = if format::is_binary_line(&line) {
                            format::format_binary_line(&line)
                        } else {
                            let mut obj = serde_json::json!({
                                "timestamp": line.timestamp.to_rfc3339(),
                                "content": line.content,
                            });
                            if !line.metadata.is_empty() {
                                obj["metadata"] = serde_json::json!(line.metadata);
                            }
                            obj
                        };
                        println!("{}", json_line);
                    } else if format::is_binary_line(&line) {
                        println!(
                            "[{}] [binary] {}",
                            line.timestamp.format("%H:%M:%S%.3f"),
                            format::format_hex_bytes(&line.raw)
                        );
                    } else {
                        println!(
                            "[{}] {}",
                            line.timestamp.format("%H:%M:%S%.3f"),
                            line.content
                        );
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[warn] Dropped {} messages (buffer overflow)", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    eprintln!("[info] Connection closed.");
                    break;
                }
            }
        }
    };

    if let Some(dur_secs) = duration {
        let _ = timeout(Duration::from_secs(dur_secs), monitor_fut).await;
    } else {
        tokio::select! {
            _ = monitor_fut => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nStopping monitor...");
            }
        }
    }

    manager.close_session(&session_id).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_send(
    port_path: String,
    baud_rate: u32,
    data: String,
    hex: bool,
    expect: Option<String>,
    pipeline: Option<std::sync::Arc<crate::pipeline::engine::Pipeline>>,
    timeout_secs: u64,
    protocol_config: Option<ProtocolConfig>,
    json: bool,
) -> Result<i32> {
    let port_config = PortConfig {
        baud_rate,
        auto_reconnect: false,
        ..PortConfig::default()
    };

    let manager = SessionManager::new(pipeline, protocol_config);
    let session_id = manager
        .create_session(port_path.clone(), port_config, None)
        .await?;
    let conn = manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| anyhow!("Failed to get session"))?;

    let data_bytes = if hex {
        if data.len() > 6144 {
            return Err(anyhow!(
                "Hex input too long (max 6144 chars, got {})",
                data.len()
            ));
        }
        format::parse_hex(&data).map_err(|e| anyhow!("Invalid hex input: {}", e))?
    } else {
        // Unescape common sequences
        data.replace("\\r", "\r")
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .into_bytes()
    };

    let exit_code = if let Some(expect_pattern) = &expect {
        if expect_pattern.len() > 1024 {
            return Err(anyhow!("Expect regex too long (max 1024 chars)"));
        }
        let re = regex::RegexBuilder::new(expect_pattern)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20)
            .build()
            .map_err(|e| anyhow!("Invalid expect pattern: {}", e))?;

        // Subscribe BEFORE writing so fast responses are not missed.
        let mut rx = conn.subscribe();
        conn.write_data(&data_bytes).await?;
        let result = timeout(Duration::from_secs(timeout_secs), async {
            loop {
                match rx.recv().await {
                    Ok(line) => {
                        if re.is_match(format::matchable_content(&line)) {
                            return Some(line);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        })
        .await;

        match result {
            Ok(Some(line)) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "matched",
                            "pattern": expect_pattern,
                            "line": line.content,
                        })
                    );
                } else {
                    println!("{}", line.content);
                }
                exit_codes::SUCCESS
            }
            Ok(None) | Err(_) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "timeout",
                            "pattern": expect_pattern,
                            "timeout_seconds": timeout_secs,
                        })
                    );
                } else {
                    eprintln!(
                        "Timeout: pattern '{}' not matched within {}s",
                        expect_pattern, timeout_secs
                    );
                }
                exit_codes::TIMEOUT
            }
        }
    } else {
        conn.write_data(&data_bytes).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let lines = conn.get_recent_lines(10).await;
        if json {
            let line_strings: Vec<&str> = lines.iter().map(|l| l.content.as_str()).collect();
            println!(
                "{}",
                serde_json::json!({
                    "status": "sent",
                    "lines": line_strings,
                })
            );
        } else {
            for line in &lines {
                println!("{}", line.content);
            }
        }
        exit_codes::SUCCESS
    };

    manager.close_session(&session_id).await?;

    Ok(exit_code)
}

pub async fn cmd_test(cfg: crate::config::SerialinkConfig, json: bool) -> i32 {
    let harness_meta = match cfg.harness {
        Some(h) => h,
        None => {
            let msg = "Config file has no [harness] section";
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"error": "invalid_input", "message": msg})
                );
            } else {
                eprintln!("Error: {}", msg);
            }
            return crate::exit_codes::INVALID_INPUT;
        }
    };

    if cfg.devices.is_empty() {
        let msg = "Config file has no [[device]] entries";
        if json {
            eprintln!(
                "{}",
                serde_json::json!({"error": "invalid_input", "message": msg})
            );
        } else {
            eprintln!("Error: {}", msg);
        }
        return crate::exit_codes::INVALID_INPUT;
    }

    if cfg.steps.is_empty() {
        let msg = "Config file has no [[step]] entries";
        if json {
            eprintln!(
                "{}",
                serde_json::json!({"error": "invalid_input", "message": msg})
            );
        } else {
            eprintln!("Error: {}", msg);
        }
        return crate::exit_codes::INVALID_INPUT;
    }

    let harness_config = HarnessConfig {
        harness: harness_meta,
        devices: cfg.devices,
        steps: cfg.steps,
    };

    let report = executor::run_harness(&harness_config).await;

    if json {
        println!("{}", serde_json::to_string(&report).unwrap_or_default());
    } else {
        println!("Harness: {}", report.harness);
        println!("Result:  {:?}", report.result);
        println!("Duration: {}ms", report.duration_ms);
        println!();
        println!(
            "{:<20} {:<20} {:<10} {:<10} ERROR",
            "STEP", "ACTION", "RESULT", "TIME(ms)"
        );
        println!("{}", "-".repeat(80));
        for s in &report.steps {
            println!(
                "{:<20} {:<20} {:<10} {:<10} {}",
                s.id,
                s.action,
                format!("{:?}", s.result),
                s.duration_ms,
                s.error.as_deref().unwrap_or(""),
            );
        }
    }

    match report.result {
        crate::harness::schema::HarnessResult::Pass => crate::exit_codes::SUCCESS,
        crate::harness::schema::HarnessResult::Fail => crate::exit_codes::PATTERN_NOT_MATCHED,
        crate::harness::schema::HarnessResult::Timeout => crate::exit_codes::TIMEOUT,
        crate::harness::schema::HarnessResult::Aborted => crate::exit_codes::PATTERN_NOT_MATCHED,
    }
}
