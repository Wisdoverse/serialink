use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::pipeline::engine::Pipeline;
use crate::pipeline::transform::DataChunk;
use crate::protocol::types::{SessionMode, META_MODE};
use crate::serial::read_strategy::ReadStrategy;

/// A timestamped line received from a serial port.
#[derive(Debug, Clone, Serialize)]
pub struct TimestampedLine {
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub raw: Vec<u8>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Connection status of a serial port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Reconnecting,
    Closed,
}

impl std::fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionStatus::Connected => write!(f, "Connected"),
            ConnectionStatus::Disconnected => write!(f, "Disconnected"),
            ConnectionStatus::Reconnecting => write!(f, "Reconnecting"),
            ConnectionStatus::Closed => write!(f, "Closed"),
        }
    }
}

/// Configuration for opening a serial port.
#[derive(Debug, Clone, Serialize)]
pub struct PortConfig {
    pub baud_rate: u32,
    #[serde(skip)]
    pub data_bits: DataBits,
    #[serde(skip)]
    pub stop_bits: StopBits,
    #[serde(skip)]
    pub parity: Parity,
    #[serde(skip)]
    pub flow_control: FlowControl,
    pub auto_reconnect: bool,
    pub reconnect_interval_ms: u64,
    #[serde(default)]
    pub mode: SessionMode,
}

impl Default for PortConfig {
    fn default() -> Self {
        Self {
            baud_rate: 115200,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            auto_reconnect: true,
            reconnect_interval_ms: 2000,
            mode: SessionMode::default(),
        }
    }
}

const BUFFER_CAPACITY: usize = 10_000;

/// Shared mutable state for a serial connection.
struct SharedState {
    buffer: VecDeque<TimestampedLine>,
    status: ConnectionStatus,
    /// Handle to the underlying serial port for writing.
    port: Option<Box<dyn serialport::SerialPort>>,
}

/// A managed serial port connection with background reading, buffering,
/// and broadcast-based real-time streaming.
#[derive(Clone)]
pub struct SerialConnection {
    pub session_id: String,
    pub port_path: String,
    pub config: PortConfig,
    pub created_at: DateTime<Utc>,
    state: Arc<Mutex<SharedState>>,
    tx: broadcast::Sender<TimestampedLine>,
    /// Signal the background reader to stop.
    cancel: tokio_util::sync::CancellationToken,
    /// Handle to the background reader task.
    reader_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Optional pipeline for transforming serial data before buffering/broadcast.
    #[allow(dead_code)]
    pipeline: Option<Arc<Pipeline>>,
}

impl SerialConnection {
    /// Open a serial port and start the background reader task.
    pub async fn open(
        session_id: String,
        port_path: String,
        config: PortConfig,
        pipeline: Option<Arc<Pipeline>>,
        strategy: Box<dyn ReadStrategy>,
    ) -> Result<Self> {
        let (tx, _) = broadcast::channel(256);

        let port =
            Self::open_raw_port(&port_path, &config).context("Failed to open serial port")?;

        // Clone the port for the reader before storing the original in shared state.
        let reader_port = port
            .try_clone()
            .context("Failed to clone serial port for reader")?;

        let state = Arc::new(Mutex::new(SharedState {
            buffer: VecDeque::with_capacity(BUFFER_CAPACITY),
            status: ConnectionStatus::Connected,
            port: Some(port),
        }));

        let cancel = tokio_util::sync::CancellationToken::new();

        // Spawn the background reader.
        let reader_port_path = port_path.clone();
        let reader_config = config.clone();
        let reader_state = state.clone();
        let reader_tx = tx.clone();
        let reader_cancel = cancel.clone();
        let reader_pipeline = pipeline.clone();

        let handle = tokio::spawn(async move {
            Self::reader_loop(
                reader_port_path,
                reader_config,
                reader_state,
                reader_tx,
                reader_cancel,
                reader_port,
                reader_pipeline,
                strategy,
            )
            .await;
        });

        let conn = Self {
            session_id,
            port_path,
            config,
            created_at: Utc::now(),
            state,
            tx,
            cancel,
            reader_handle: Arc::new(tokio::sync::Mutex::new(Some(handle))),
            pipeline,
        };

        Ok(conn)
    }

    /// Write bytes to the serial port with a 5-second timeout.
    pub async fn write_data(&self, data: &[u8]) -> Result<()> {
        let data = data.to_vec();
        let state = self.state.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let mut guard = state.lock().map_err(|_| anyhow::anyhow!("Lock poisoned"))?;
                let port = guard
                    .port
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Serial port is not open"))?;
                port.write_all(&data)
                    .context("Failed to write to serial port")?;
                Ok(())
            }),
        )
        .await;
        match result {
            Ok(inner) => inner?,
            Err(_) => anyhow::bail!("Write timed out after 5 seconds"),
        }
    }

    /// Close the serial connection and stop the background reader.
    pub async fn close(&self) -> Result<()> {
        self.cancel.cancel();

        // Await the background reader task.
        let handle = {
            let mut rh = self.reader_handle.lock().await;
            rh.take()
        };
        if let Some(h) = handle {
            let _ = h.await;
        }

        let mut guard = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Lock poisoned"))?;
        guard.status = ConnectionStatus::Closed;
        guard.port = None;

        info!(session_id = %self.session_id, "Serial connection closed");
        Ok(())
    }

    /// Get the last `count` lines from the ring buffer.
    pub async fn get_recent_lines(&self, count: usize) -> Vec<TimestampedLine> {
        let guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let len = guard.buffer.len();
        let skip = len.saturating_sub(count);
        guard.buffer.iter().skip(skip).cloned().collect()
    }

    /// Subscribe to real-time line streaming.
    pub fn subscribe(&self) -> broadcast::Receiver<TimestampedLine> {
        self.tx.subscribe()
    }

    /// Get the current connection status.
    pub fn status(&self) -> ConnectionStatus {
        match self.state.lock() {
            Ok(guard) => guard.status.clone(),
            Err(_) => ConnectionStatus::Closed,
        }
    }

    // ── internal helpers ──────────────────────────────────────────────

    fn open_raw_port(
        port_path: &str,
        config: &PortConfig,
    ) -> Result<Box<dyn serialport::SerialPort>> {
        let port = serialport::new(port_path, config.baud_rate)
            .data_bits(config.data_bits)
            .stop_bits(config.stop_bits)
            .parity(config.parity)
            .flow_control(config.flow_control)
            .timeout(Duration::from_millis(100))
            .open()
            .with_context(|| format!("Failed to open serial port {}", port_path))?;

        Ok(port)
    }

    #[allow(clippy::too_many_arguments)]
    async fn reader_loop(
        port_path: String,
        config: PortConfig,
        state: Arc<Mutex<SharedState>>,
        tx: broadcast::Sender<TimestampedLine>,
        cancel: tokio_util::sync::CancellationToken,
        initial_read_port: Box<dyn serialport::SerialPort>,
        pipeline: Option<Arc<Pipeline>>,
        mut strategy: Box<dyn ReadStrategy>,
    ) {
        let mut read_port_opt: Option<Box<dyn serialport::SerialPort>> = Some(initial_read_port);

        loop {
            if cancel.is_cancelled() {
                break;
            }

            // Take the read port out for the blocking read.
            let mut read_port = match read_port_opt.take() {
                Some(p) => p,
                None => break,
            };

            let cancel_clone = cancel.clone();
            // Move strategy into the blocking task and get it back.
            let read_result = tokio::task::spawn_blocking(move || {
                let lines = strategy.read_frames(&mut *read_port, &cancel_clone);
                (read_port, strategy, lines)
            })
            .await;

            match read_result {
                Ok((port_back, strategy_back, Ok(lines))) => {
                    // Put the read port and strategy back for next iteration.
                    read_port_opt = Some(port_back);
                    strategy = strategy_back;

                    if lines.is_empty() {
                        // Timeout with no data — just loop again.
                        continue;
                    }

                    // Run pipeline transforms BEFORE acquiring the state lock.
                    // MutexGuard is not Send, so we cannot hold it across .await.
                    // Binary-mode lines skip the pipeline.
                    let transformed = if let Some(ref pl) = pipeline {
                        let mut text_lines = Vec::new();
                        let mut binary_lines = Vec::new();
                        for line in lines {
                            if line.metadata.get(META_MODE).map(|v| v.as_str()) == Some("binary") {
                                binary_lines.push(line);
                            } else {
                                text_lines.push(line);
                            }
                        }

                        let mut result = binary_lines;
                        for line in text_lines {
                            let chunk: DataChunk = line.into();
                            let outputs = pl.process(chunk).await;
                            result.extend(outputs.into_iter().map(TimestampedLine::from));
                        }
                        result
                    } else {
                        lines
                    };

                    if transformed.is_empty() {
                        // Pipeline filtered everything — skip lock acquisition.
                        continue;
                    }

                    let mut guard = match state.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            error!("Session state lock poisoned in reader loop");
                            break;
                        }
                    };
                    for line in transformed {
                        // Send to broadcast (ignore errors if no receivers).
                        let _ = tx.send(line.clone());

                        // Push to ring buffer.
                        if guard.buffer.len() >= BUFFER_CAPACITY {
                            guard.buffer.pop_front();
                        }
                        guard.buffer.push_back(line);
                    }
                }
                Ok((_, strategy_back, Err(_))) => {
                    // Port error — handle disconnect / reconnect.
                    strategy = strategy_back;

                    if cancel.is_cancelled() {
                        break;
                    }

                    {
                        let mut guard = match state.lock() {
                            Ok(g) => g,
                            Err(_) => {
                                error!("Session state lock poisoned during disconnect");
                                break;
                            }
                        };
                        guard.port = None;
                        guard.status = ConnectionStatus::Disconnected;
                    }

                    warn!(port = %port_path, "Serial port disconnected");

                    if !config.auto_reconnect {
                        break;
                    }

                    // Reconnect loop.
                    {
                        let mut guard = match state.lock() {
                            Ok(g) => g,
                            Err(_) => {
                                error!("Session state lock poisoned before reconnect");
                                break;
                            }
                        };
                        guard.status = ConnectionStatus::Reconnecting;
                    }

                    loop {
                        if cancel.is_cancelled() {
                            return;
                        }

                        tokio::time::sleep(Duration::from_millis(config.reconnect_interval_ms))
                            .await;

                        if cancel.is_cancelled() {
                            return;
                        }

                        match Self::open_raw_port(&port_path, &config) {
                            Ok(port) => {
                                // Clone for reader before storing original.
                                match port.try_clone() {
                                    Ok(cloned) => {
                                        let mut guard = match state.lock() {
                                            Ok(g) => g,
                                            Err(_) => {
                                                error!(
                                                    "Session state lock poisoned during reconnect"
                                                );
                                                return;
                                            }
                                        };
                                        guard.port = Some(port);
                                        guard.status = ConnectionStatus::Connected;
                                        read_port_opt = Some(cloned);
                                        strategy.reset();
                                        info!(port = %port_path, "Reconnected to serial port");
                                        break;
                                    }
                                    Err(e) => {
                                        error!("Failed to clone reconnected port: {}", e);
                                        // Drop port and retry
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(port = %port_path, error = %e, "Reconnect attempt failed");
                            }
                        }
                    }
                }
                Err(_) => {
                    // spawn_blocking panicked
                    if cancel.is_cancelled() {
                        break;
                    }

                    {
                        let mut guard = match state.lock() {
                            Ok(g) => g,
                            Err(_) => {
                                error!("Session state lock poisoned during disconnect");
                                break;
                            }
                        };
                        guard.port = None;
                        guard.status = ConnectionStatus::Disconnected;
                    }

                    warn!(port = %port_path, "Serial port reader task panicked");
                    break;
                }
            }
        }
    }
}
