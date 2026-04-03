pub mod discovery;
pub mod manager;
pub mod port;
pub mod read_strategy;

/// Validate that a port path is a permitted serial device path.
///
/// Shared by MCP and HTTP interfaces.
pub fn validate_port_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("port path cannot be empty");
    }
    if !path.starts_with('/') && !path.starts_with("COM") {
        return Err("port path must be absolute (Unix) or COMx (Windows)");
    }
    if path.contains("..") {
        return Err("port path cannot contain '..'");
    }
    let allowed_prefixes = ["/dev/tty", "/dev/serial/", "/dev/cu.", "/dev/pts/", "COM"];
    if !allowed_prefixes.iter().any(|p| path.starts_with(p)) {
        return Err(
            "port path must be a serial device (/dev/tty*, /dev/serial/*, /dev/cu.*, /dev/pts/*, COMx)",
        );
    }
    Ok(())
}
