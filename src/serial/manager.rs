use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use super::port::{PortConfig, SerialConnection};
use super::read_strategy::LineReadStrategy;
use crate::config::ProtocolConfig;
use crate::pipeline::engine::Pipeline;
use crate::protocol::frame_parser::FrameParser;
use crate::protocol::frame_strategy::FrameReadStrategy;
use crate::protocol::modbus::{ModbusAsciiDecoder, ModbusRtuDecoder};
use crate::protocol::types::SessionMode;

const MAX_SESSIONS: usize = 16;

/// Summary information about an active session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub port_path: String,
    pub baud_rate: u32,
    pub mode: SessionMode,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// Manages multiple serial port sessions.
#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, Arc<SerialConnection>>>>,
    pipeline: Option<Arc<Pipeline>>,
    default_protocol: Option<ProtocolConfig>,
}

impl SessionManager {
    /// Create a new, empty session manager.
    ///
    /// If `pipeline` is `Some`, all sessions created by this manager will
    /// apply the pipeline transforms to serial data before buffering.
    /// If `default_protocol` is `Some`, sessions created without an explicit
    /// protocol override will use it for binary/frame-oriented reading.
    pub fn new(pipeline: Option<Arc<Pipeline>>, default_protocol: Option<ProtocolConfig>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            pipeline,
            default_protocol,
        }
    }

    /// Open a new serial connection and return the session ID.
    ///
    /// If `protocol_override` is `Some`, the session will use frame-oriented
    /// binary reading with the given protocol config. If `None`, the manager's
    /// `default_protocol` is used when available, otherwise plain text/line mode.
    ///
    /// Holds the lock across the entire operation to prevent races where two
    /// concurrent callers both pass the limit/duplicate check before either inserts.
    pub async fn create_session(
        &self,
        port_path: String,
        mut config: PortConfig,
        protocol_override: Option<ProtocolConfig>,
    ) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();

        let mut sessions = self.sessions.lock().await;
        if sessions.len() >= MAX_SESSIONS {
            return Err(anyhow!("Maximum session limit ({}) reached", MAX_SESSIONS));
        }
        if sessions.values().any(|c| c.port_path == port_path) {
            return Err(anyhow!(
                "Port {} is already open in another session",
                port_path
            ));
        }

        // Determine effective protocol config: per-session override > manager default > None.
        let effective_protocol = protocol_override.or_else(|| self.default_protocol.clone());

        let strategy: Box<dyn crate::serial::read_strategy::ReadStrategy> =
            if let Some(ref proto) = effective_protocol {
                config.mode = SessionMode::Binary;
                let parser = FrameParser::new(proto.frame.clone());
                let decoder: Option<Box<dyn crate::protocol::types::ProtocolDecoder>> =
                    match proto.decoder.as_deref() {
                        Some("modbus_rtu") => Some(Box::new(ModbusRtuDecoder)),
                        Some("modbus_ascii") => Some(Box::new(ModbusAsciiDecoder)),
                        _ => None,
                    };
                let protocol_name = proto.frame.name.clone();
                Box::new(FrameReadStrategy::new(parser, decoder, protocol_name))
            } else {
                Box::new(LineReadStrategy::new())
            };

        let conn = SerialConnection::open(
            session_id.clone(),
            port_path.clone(),
            config,
            self.pipeline.clone(),
            strategy,
        )
        .await?;
        let conn = Arc::new(conn);
        sessions.insert(session_id.clone(), conn);

        info!(session_id = %session_id, port = %port_path, "Session created");
        Ok(session_id)
    }

    /// Look up an existing session by ID.
    pub async fn get_session(&self, id: &str) -> Option<Arc<SerialConnection>> {
        let sessions = self.sessions.lock().await;
        sessions.get(id).cloned()
    }

    /// Close a session and remove it from the manager.
    pub async fn close_session(&self, id: &str) -> Result<()> {
        let conn = {
            let mut sessions = self.sessions.lock().await;
            sessions
                .remove(id)
                .ok_or_else(|| anyhow!("Session not found: {}", id))?
        };

        conn.close().await?;
        info!(session_id = %id, "Session closed and removed");
        Ok(())
    }

    /// List all active sessions.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let connections: Vec<Arc<SerialConnection>> = {
            let sessions = self.sessions.lock().await;
            sessions.values().cloned().collect()
        };
        let mut infos = Vec::with_capacity(connections.len());
        for conn in &connections {
            let status = conn.status();
            infos.push(SessionInfo {
                session_id: conn.session_id.clone(),
                port_path: conn.port_path.clone(),
                baud_rate: conn.config.baud_rate,
                mode: conn.config.mode.clone(),
                status: status.to_string(),
                created_at: conn.created_at,
            });
        }
        infos
    }

    /// Close all sessions. Intended for use during shutdown.
    pub async fn close_all(&self) {
        let session_ids: Vec<String> = {
            let sessions = self.sessions.lock().await;
            sessions.keys().cloned().collect()
        };
        for id in session_ids {
            let _ = self.close_session(&id).await;
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(None, None)
    }
}
