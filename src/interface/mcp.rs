use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{Error as McpError, RoleServer, ServiceExt};
use serde_json::json;

use crate::protocol::format;
use crate::protocol::presets;
use crate::serial::discovery;
use crate::serial::manager::SessionManager;
use crate::serial::port::PortConfig;
use crate::serial::validate_port_path;

/// MCP server that exposes serial port tools to AI agents.
#[derive(Clone)]
pub struct SerialinkMcpServer {
    manager: Arc<SessionManager>,
}

/// Helper to format a line as JSON, using binary format for binary-mode lines.
fn format_line_json(l: &crate::serial::port::TimestampedLine) -> serde_json::Value {
    if format::is_binary_line(l) {
        format::format_binary_line(l)
    } else {
        let mut obj = json!({
            "timestamp": l.timestamp.to_rfc3339(),
            "content": l.content,
        });
        if !l.metadata.is_empty() {
            obj["metadata"] = json!(l.metadata);
        }
        obj
    }
}

impl SerialinkMcpServer {
    pub fn new(manager: Arc<SessionManager>) -> Self {
        Self { manager }
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool::new(
                "list_ports",
                "List all available serial ports on the system",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                })),
            ),
            Tool::new(
                "open_port",
                "Open a serial port connection and return a session ID",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "port_path": {
                            "type": "string",
                            "description": "Path to the serial port (e.g. /dev/ttyUSB0)"
                        },
                        "baud_rate": {
                            "type": "integer",
                            "description": "Baud rate (default: 115200)"
                        },
                        "mode": {
                            "type": "string",
                            "description": "Session mode: 'text' (default) or 'binary'"
                        },
                        "protocol": {
                            "type": "string",
                            "description": "Protocol preset name (e.g. 'modbus_rtu', 'modbus_ascii'). Auto-sets mode to binary."
                        }
                    },
                    "required": ["port_path"]
                })),
            ),
            Tool::new(
                "close_port",
                "Close an active serial port session",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session ID returned by open_port"
                        }
                    },
                    "required": ["session_id"]
                })),
            ),
            Tool::new(
                "read_lines",
                "Read the most recent N lines from the serial port buffer",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session ID"
                        },
                        "count": {
                            "type": "integer",
                            "description": "Number of recent lines to read (default: 50)"
                        }
                    },
                    "required": ["session_id"]
                })),
            ),
            Tool::new(
                "write_data",
                "Write data to a serial port",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session ID"
                        },
                        "data": {
                            "type": "string",
                            "description": "The data string to write to the serial port"
                        }
                    },
                    "required": ["session_id", "data"]
                })),
            ),
            Tool::new(
                "send_data",
                "Send binary data (hex-encoded) to a serial port",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session ID"
                        },
                        "hex": {
                            "type": "string",
                            "description": "Hex-encoded bytes to send (e.g. '01 03 00 01 00 01 D5 CA')"
                        }
                    },
                    "required": ["session_id", "hex"]
                })),
            ),
            Tool::new(
                "send_and_expect",
                "Write data to the serial port and wait for a regex pattern match in output",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session ID"
                        },
                        "data": {
                            "type": "string",
                            "description": "The data string to write"
                        },
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to wait for in output"
                        },
                        "timeout_ms": {
                            "type": "integer",
                            "description": "Timeout in milliseconds (default: 5000)"
                        }
                    },
                    "required": ["session_id", "data", "pattern"]
                })),
            ),
            Tool::new(
                "snapshot",
                "Get buffered lines from a serial port session as a snapshot",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "The session ID"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of lines to return (default: 500, max: 5000)"
                        }
                    },
                    "required": ["session_id"]
                })),
            ),
            Tool::new(
                "list_sessions",
                "List all active serial port sessions",
                rmcp::model::object(json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                })),
            ),
        ]
    }

    async fn handle_tool(
        &self,
        name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        match name {
            "list_ports" => self.handle_list_ports().await,
            "open_port" => self.handle_open_port(args).await,
            "close_port" => self.handle_close_port(args).await,
            "read_lines" => self.handle_read_lines(args).await,
            "write_data" => self.handle_write_data(args).await,
            "send_data" => self.handle_send_data(args).await,
            "send_and_expect" => self.handle_send_and_expect(args).await,
            "snapshot" => self.handle_snapshot(args).await,
            "list_sessions" => self.handle_list_sessions().await,
            _ => Err(McpError::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >()),
        }
    }

    async fn handle_list_ports(&self) -> Result<CallToolResult, McpError> {
        let ports = discovery::list_ports().map_err(|e| {
            McpError::internal_error(
                "Failed to list serial ports",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        let json_str = serde_json::to_string(&ports).map_err(|e| {
            McpError::internal_error(
                "Failed to serialize ports",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    async fn handle_open_port(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let port_path = args
            .get("port_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("missing required parameter: port_path", None))?
            .to_string();

        validate_port_path(&port_path).map_err(|e| McpError::invalid_params(e, None))?;

        let mut config = PortConfig::default();
        if let Some(baud) = args.get("baud_rate").and_then(|v| v.as_u64()) {
            if baud == 0 || baud > 3_000_000 {
                return Err(McpError::invalid_params(
                    "baud_rate must be between 1 and 3000000",
                    None,
                ));
            }
            config.baud_rate = baud as u32;
        }

        // Resolve protocol preset and mode.
        let mode_str = args.get("mode").and_then(|v| v.as_str()).unwrap_or("text");
        let protocol_name = args.get("protocol").and_then(|v| v.as_str());

        let protocol_override = if let Some(name) = protocol_name {
            if name.is_empty() || name.len() > 128 {
                return Err(McpError::invalid_params(
                    "protocol name must be 1-128 characters",
                    None,
                ));
            }
            let preset = presets::resolve_preset(name).ok_or_else(|| {
                McpError::invalid_params(
                    "Unknown protocol preset. Available: modbus_rtu, modbus_ascii",
                    None,
                )
            })?;
            Some(preset)
        } else {
            None
        };

        // Validate mode.
        let effective_mode = if protocol_override.is_some() {
            "binary"
        } else {
            mode_str
        };
        match effective_mode {
            "text" | "binary" => {}
            _ => {
                return Err(McpError::invalid_params(
                    "mode must be 'text' or 'binary'",
                    None,
                ));
            }
        }

        // If mode=binary without a protocol and no TOML config, error.
        if effective_mode == "binary" && protocol_override.is_none() {
            // Check if manager has a default protocol (from TOML config).
            // We can try to create and see if manager's default kicks in.
            // For now, we just allow it — the manager will use its default_protocol if available.
        }

        let session_id = self
            .manager
            .create_session(port_path.clone(), config, protocol_override)
            .await
            .map_err(|e| {
                McpError::internal_error(
                    "Failed to open serial port",
                    Some(json!({ "reason": e.to_string() })),
                )
            })?;

        Ok(CallToolResult::success(vec![Content::text(
            json!({
                "session_id": session_id,
                "port_path": port_path,
                "mode": effective_mode,
                "status": "connected"
            })
            .to_string(),
        )]))
    }

    async fn handle_close_port(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("missing required parameter: session_id", None)
            })?;

        self.manager.close_session(session_id).await.map_err(|e| {
            McpError::internal_error(
                "Failed to close session",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(
            json!({ "status": "closed", "session_id": session_id }).to_string(),
        )]))
    }

    async fn handle_read_lines(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("missing required parameter: session_id", None)
            })?;

        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let count = count.min(1000); // Cap at 1000 lines

        let conn = self.manager.get_session(session_id).await.ok_or_else(|| {
            McpError::invalid_params(
                "Session not found",
                Some(json!({ "session_id": session_id })),
            )
        })?;

        let lines = conn.get_recent_lines(count).await;

        let output: Vec<serde_json::Value> = lines.iter().map(format_line_json).collect();

        let json_str = serde_json::to_string(&output).map_err(|e| {
            McpError::internal_error(
                "Failed to serialize lines",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    async fn handle_write_data(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("missing required parameter: session_id", None)
            })?;

        let data = args
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("missing required parameter: data", None))?;

        let conn = self.manager.get_session(session_id).await.ok_or_else(|| {
            McpError::invalid_params(
                "Session not found",
                Some(json!({ "session_id": session_id })),
            )
        })?;

        conn.write_data(data.as_bytes()).await.map_err(|e| {
            McpError::internal_error(
                "Failed to write data",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(
            json!({ "status": "written", "bytes": data.len() }).to_string(),
        )]))
    }

    async fn handle_send_data(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("missing required parameter: session_id", None)
            })?;

        let hex_str = args
            .get("hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("missing required parameter: hex", None))?;

        if hex_str.len() > 6144 {
            return Err(McpError::invalid_params(
                "Hex string too long (max 6144 chars)",
                None,
            ));
        }

        let bytes = format::parse_hex(hex_str).map_err(|e| {
            McpError::invalid_params("Invalid hex string", Some(json!({ "reason": e })))
        })?;

        let conn = self.manager.get_session(session_id).await.ok_or_else(|| {
            McpError::invalid_params(
                "Session not found",
                Some(json!({ "session_id": session_id })),
            )
        })?;

        conn.write_data(&bytes).await.map_err(|e| {
            McpError::internal_error(
                "Failed to write data",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(
            json!({ "status": "written", "bytes": bytes.len() }).to_string(),
        )]))
    }

    async fn handle_send_and_expect(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("missing required parameter: session_id", None)
            })?;

        let data = args
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("missing required parameter: data", None))?;

        let pattern_str = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::invalid_params("missing required parameter: pattern", None))?;

        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);
        let timeout_ms = timeout_ms.min(30_000); // Cap at 30 seconds

        if pattern_str.len() > 1024 {
            return Err(McpError::invalid_params(
                "Regex pattern too long (max 1024 chars)",
                None,
            ));
        }
        let re = regex::RegexBuilder::new(pattern_str)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20)
            .build()
            .map_err(|e| {
                McpError::invalid_params(
                    "Invalid regex pattern",
                    Some(json!({ "reason": e.to_string() })),
                )
            })?;

        let conn = self.manager.get_session(session_id).await.ok_or_else(|| {
            McpError::invalid_params(
                "Session not found",
                Some(json!({ "session_id": session_id })),
            )
        })?;

        // Subscribe before writing so we don't miss any output.
        let mut rx = conn.subscribe();

        // Write the data.
        conn.write_data(data.as_bytes()).await.map_err(|e| {
            McpError::internal_error(
                "Failed to write data",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        // Wait for the pattern match with timeout.
        let mut collected_lines: Vec<String> = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(CallToolResult::error(vec![Content::text(
                    json!({
                        "error": "timeout",
                        "message": format!("Pattern '{}' not matched within {}ms", pattern_str, timeout_ms),
                        "collected_lines": collected_lines,
                    })
                    .to_string(),
                )]));
            }

            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(line)) => {
                    let matchable = format::matchable_content(&line);
                    let matched = re.is_match(matchable);
                    if collected_lines.len() < 200 {
                        collected_lines.push(line.content.clone());
                    }
                    if matched {
                        return Ok(CallToolResult::success(vec![Content::text(
                            json!({
                                "matched": true,
                                "matched_line": line.content,
                                "collected_lines": collected_lines,
                            })
                            .to_string(),
                        )]));
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                    if collected_lines.len() < 200 {
                        collected_lines.push(format!("[dropped {} messages due to lag]", n));
                    }
                    continue; // Don't treat lag as fatal
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                    // Return error only for closed channel
                    return Ok(CallToolResult::error(vec![Content::text(
                        json!({
                            "error": "channel_closed",
                            "collected_lines": collected_lines,
                        })
                        .to_string(),
                    )]));
                }
                Err(_) => {
                    // Timeout.
                    return Ok(CallToolResult::error(vec![Content::text(
                        json!({
                            "error": "timeout",
                            "message": format!("Pattern '{}' not matched within {}ms", pattern_str, timeout_ms),
                            "collected_lines": collected_lines,
                        })
                        .to_string(),
                    )]));
                }
            }
        }
    }

    async fn handle_snapshot(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("missing required parameter: session_id", None)
            })?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
        let limit = limit.min(5000);

        let conn = self.manager.get_session(session_id).await.ok_or_else(|| {
            McpError::invalid_params(
                "Session not found",
                Some(json!({ "session_id": session_id })),
            )
        })?;

        let lines = conn.get_recent_lines(limit).await;

        let output: Vec<serde_json::Value> = lines.iter().map(format_line_json).collect();

        let json_str = serde_json::to_string(&json!({
            "session_id": session_id,
            "line_count": output.len(),
            "lines": output,
        }))
        .map_err(|e| {
            McpError::internal_error(
                "Failed to serialize snapshot",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }

    async fn handle_list_sessions(&self) -> Result<CallToolResult, McpError> {
        let sessions = self.manager.list_sessions().await;

        let json_str = serde_json::to_string(&sessions).map_err(|e| {
            McpError::internal_error(
                "Failed to serialize sessions",
                Some(json!({ "reason": e.to_string() })),
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(json_str)]))
    }
}

impl ServerHandler for SerialinkMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "serialink".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "Serialink MCP server provides tools for interacting with serial ports. \
                 Use list_ports to discover available ports, open_port to connect, \
                 then read_lines/write_data/send_and_expect to communicate."
                    .to_string(),
            ),
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn list_tools(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async {
            Ok(ListToolsResult {
                tools: Self::tool_definitions(),
                next_cursor: None,
            })
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            let args = request.arguments.unwrap_or_default();
            self.handle_tool(&request.name, &args).await
        }
    }
}

/// Start the MCP server on stdio transport.
pub async fn run_mcp_server(manager: Arc<SessionManager>) -> Result<()> {
    let server = SerialinkMcpServer::new(manager);
    let transport = rmcp::transport::stdio();
    let service = server
        .serve(transport)
        .await
        .map_err(|e: std::io::Error| anyhow::anyhow!("Failed to start MCP server: {}", e))?;
    service.waiting().await?;
    Ok(())
}

/// Start the MCP server on SSE transport (HTTP, for remote access).
pub async fn run_mcp_sse_server(
    manager: Arc<SessionManager>,
    bind: std::net::SocketAddr,
) -> Result<()> {
    use rmcp::transport::sse_server::SseServer;

    tracing::info!("Starting MCP SSE server on {}", bind);
    eprintln!("MCP SSE server listening on http://{}", bind);
    eprintln!("  SSE endpoint:     http://{}/sse", bind);
    eprintln!("  Message endpoint: http://{}/message", bind);

    let sse_server = SseServer::serve(bind)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start SSE server: {}", e))?;

    let ct = sse_server.with_service(move || SerialinkMcpServer::new(manager.clone()));

    // Wait for ctrl+c
    tokio::signal::ctrl_c().await?;
    ct.cancel();

    Ok(())
}
