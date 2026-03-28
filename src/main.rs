use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod config;
mod exit_codes;
mod interface;
mod pipeline;
mod protocol;
mod serial;

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Json,
    Text,
}

#[derive(Parser)]
#[command(name = "serialink")]
#[command(about = "Structured serial port tool for automation, CI/CD, and AI agents")]
#[command(version)]
struct Cli {
    /// Path to a TOML configuration file (pipeline transforms, etc.)
    #[arg(long, global = true)]
    config: Option<String>,

    /// Output format (default: json for agent-native output)
    #[arg(long, global = true, default_value = "json")]
    format: OutputFormat,

    /// Shorthand for --format text (human-readable output)
    #[arg(long, global = true)]
    human: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List available serial ports
    List {},

    /// Monitor a serial port (stream output)
    Monitor {
        /// Serial port path (e.g., /dev/ttyUSB0)
        port: String,

        /// Baud rate
        #[arg(short, long, default_value = "115200")]
        baud_rate: u32,

        /// Regex filter — only show matching lines
        #[arg(short, long)]
        filter: Option<String>,

        /// Stop after N seconds
        #[arg(long)]
        duration: Option<u64>,
    },

    /// Send data to a serial port
    Send {
        /// Serial port path (e.g., /dev/ttyUSB0)
        port: String,

        /// Data to send (use \\r\\n for carriage return + newline)
        data: String,

        /// Baud rate
        #[arg(short, long, default_value = "115200")]
        baud_rate: u32,

        /// Interpret data as hex bytes (e.g., "01 03 00 01 00 01")
        #[arg(long)]
        hex: bool,

        /// Regex pattern to expect in response
        #[arg(short, long)]
        expect: Option<String>,

        /// Timeout in seconds for expect pattern
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },

    /// Start as a server (MCP, SSE, or HTTP)
    Serve {
        /// Run as MCP server (stdio transport)
        #[arg(long)]
        mcp: bool,

        /// Run as MCP SSE server (HTTP transport, for remote AI agents)
        #[arg(long)]
        sse: bool,

        /// Run as HTTP/WebSocket REST API server
        #[arg(long)]
        http: bool,

        /// Server bind address
        #[arg(long, default_value = "127.0.0.1:8600")]
        bind: String,

        /// API key for HTTP authentication (or set SERIALINK_API_KEY env var)
        #[arg(long, env = "SERIALINK_API_KEY")]
        api_key: Option<String>,
    },
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // Clap parse error — attempt to detect if --format json was intended.
            // If any arg contains "json" or no --human flag, assume JSON mode for error output.
            let args: Vec<String> = std::env::args().collect();
            let likely_json = !args.iter().any(|a| a == "--human" || a == "--format=text");
            if likely_json {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "error": "invalid_input",
                        "message": e.to_string().lines().next().unwrap_or("invalid arguments"),
                        "exit_code": exit_codes::INVALID_INPUT,
                    })
                );
            } else {
                e.print().ok();
            }
            std::process::exit(exit_codes::INVALID_INPUT);
        }
    };

    // Resolve effective format: --human overrides --format
    let format = if cli.human {
        OutputFormat::Text
    } else {
        cli.format.clone()
    };

    let is_json = matches!(format, OutputFormat::Json);

    // Initialize logging to stderr. In JSON mode, suppress info/warn to avoid polluting output.
    let filter = if is_json {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error"))
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(!is_json)
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let code = rt.block_on(run_inner(cli, format.clone()));

    match code {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(e) => {
            let error_str = e.to_string();
            let exit_code = classify_error(&error_str);
            if is_json {
                let error_key = match exit_code {
                    exit_codes::CONNECTION_ERROR => "connection_error",
                    exit_codes::INVALID_INPUT => "invalid_input",
                    exit_codes::TIMEOUT => "timeout",
                    _ => "internal_error",
                };
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "error": error_key,
                        "message": error_str,
                        "exit_code": exit_code,
                    })
                );
            } else {
                eprintln!("Error: {}", error_str);
            }
            std::process::exit(exit_code);
        }
    }
}

/// Heuristic to classify an anyhow error into a semantic exit code.
fn classify_error(msg: &str) -> i32 {
    let lower = msg.to_lowercase();
    // Config/input errors first (before connection errors, since "no such file" could be either)
    if lower.contains("invalid")
        || lower.contains("too long")
        || lower.contains("too many")
        || lower.contains("regex")
        || lower.contains("toml")
        || lower.contains("parse")
        || lower.contains("config")
    {
        exit_codes::INVALID_INPUT
    } else if lower.contains("permission denied")
        || lower.contains("failed to open")
        || lower.contains("connection")
        || lower.contains("device or resource busy")
    {
        exit_codes::CONNECTION_ERROR
    } else if lower.contains("no such file") {
        // Could be config file or device — classify as INVALID_INPUT since
        // actual device errors typically say "failed to open" not "no such file"
        exit_codes::INVALID_INPUT
    } else if lower.contains("timeout") || lower.contains("timed out") {
        exit_codes::TIMEOUT
    } else {
        exit_codes::INTERNAL_ERROR
    }
}

async fn run_inner(cli: Cli, format: OutputFormat) -> anyhow::Result<i32> {
    // Build pipeline and protocol config from config file if provided.
    let (pipeline, protocol_config) = if let Some(config_path) = &cli.config {
        let cfg = config::load_config(config_path)?;
        let pl = if cfg.pipeline.is_empty() {
            None
        } else {
            let pl = pipeline::engine::Pipeline::from_config(&cfg.pipeline)?;
            Some(std::sync::Arc::new(pl))
        };
        (pl, cfg.protocol)
    } else {
        (None, None)
    };

    let is_json = matches!(format, OutputFormat::Json);

    match cli.command {
        Commands::List {} => {
            interface::cli::cmd_list(is_json).await?;
            Ok(exit_codes::SUCCESS)
        }
        Commands::Monitor {
            port,
            baud_rate,
            filter,
            duration,
        } => {
            interface::cli::cmd_monitor(
                port,
                baud_rate,
                is_json,
                filter,
                duration,
                pipeline,
                protocol_config,
            )
            .await?;
            Ok(exit_codes::SUCCESS)
        }
        Commands::Send {
            port,
            data,
            baud_rate,
            hex,
            expect,
            timeout,
        } => {
            let result = interface::cli::cmd_send(
                port,
                baud_rate,
                data,
                hex,
                expect.clone(),
                pipeline,
                timeout,
                protocol_config,
                is_json,
            )
            .await?;
            Ok(result)
        }
        Commands::Serve {
            mcp,
            sse,
            http,
            bind,
            api_key,
        } => {
            // Validate: exactly one mode must be specified.
            let mode_count = [mcp, sse, http].iter().filter(|&&x| x).count();
            if mode_count == 0 {
                anyhow::bail!(
                    "Specify exactly one of --mcp, --sse, or --http. Run `serialink serve --help` for details."
                );
            }
            if mode_count > 1 {
                anyhow::bail!("Only one of --mcp, --sse, or --http can be specified at a time.");
            }

            let manager = std::sync::Arc::new(serial::manager::SessionManager::new(
                pipeline,
                protocol_config,
            ));

            if mcp {
                interface::mcp::run_mcp_server(manager).await?;
            } else if sse {
                let addr: std::net::SocketAddr = bind.parse()?;
                if !addr.ip().is_loopback() {
                    anyhow::bail!(
                        "SSE transport does not support authentication. \
                         Refusing to bind to {} — use --bind 127.0.0.1:PORT \
                         or switch to --http with --api-key for remote access.",
                        addr
                    );
                }
                interface::mcp::run_mcp_sse_server(manager, addr).await?;
            } else if http {
                let addr: std::net::SocketAddr = bind.parse()?;
                if !addr.ip().is_loopback() && api_key.is_none() {
                    anyhow::bail!(
                        "Refusing to bind HTTP to {} without --api-key. \
                         Set --api-key or use --bind 127.0.0.1:PORT for local-only access.",
                        addr
                    );
                }
                interface::http::run_http_server(manager, addr, api_key).await?;
            }

            Ok(exit_codes::SUCCESS)
        }
    }
}
