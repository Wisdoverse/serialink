use anyhow::{Context, Result};
use serde::Serialize;
use serialport::SerialPortType;
use std::fmt;

/// Information about an available serial port.
#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub name: String,
    pub port_type: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
}

impl fmt::Display for PortInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]", self.name, self.port_type)?;
        if let (Some(vid), Some(pid)) = (self.vid, self.pid) {
            write!(f, " VID:{:04X} PID:{:04X}", vid, pid)?;
        }
        if let Some(ref mfr) = self.manufacturer {
            write!(f, " ({})", mfr)?;
        }
        if let Some(ref sn) = self.serial_number {
            write!(f, " SN:{}", sn)?;
        }
        Ok(())
    }
}

/// List all available serial ports on the system.
pub fn list_ports() -> Result<Vec<PortInfo>> {
    let ports = serialport::available_ports().context("Failed to enumerate serial ports")?;

    let infos = ports
        .into_iter()
        .map(|p| {
            let (port_type, vid, pid, serial_number, manufacturer) = match p.port_type {
                SerialPortType::UsbPort(info) => (
                    "USB".to_string(),
                    Some(info.vid),
                    Some(info.pid),
                    info.serial_number,
                    info.manufacturer,
                ),
                SerialPortType::PciPort => ("PCI".to_string(), None, None, None, None),
                SerialPortType::BluetoothPort => ("Bluetooth".to_string(), None, None, None, None),
                SerialPortType::Unknown => ("Unknown".to_string(), None, None, None, None),
            };

            PortInfo {
                name: p.port_name,
                port_type,
                vid,
                pid,
                serial_number,
                manufacturer,
            }
        })
        .collect();

    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_ports_returns_ok() {
        // Should not panic, even if no ports available
        let result = list_ports();
        assert!(result.is_ok());
    }

    #[test]
    fn test_port_info_display() {
        let info = PortInfo {
            name: "/dev/ttyUSB0".to_string(),
            port_type: "USB".to_string(),
            vid: Some(0x1A86),
            pid: Some(0x7523),
            serial_number: Some("12345".to_string()),
            manufacturer: Some("Silicon Labs".to_string()),
        };
        let display = format!("{}", info);
        assert!(display.contains("/dev/ttyUSB0"));
        assert!(display.contains("USB"));
        assert!(display.contains("1A86"));
        assert!(display.contains("Silicon Labs"));
    }
}
