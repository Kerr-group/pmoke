use crate::config::{Config, Connection};
use anyhow::{Result, anyhow, bail};

pub fn validate_connection(conn: &Connection) -> anyhow::Result<Connection> {
    match conn {
        Connection::Gpib { board, address } => {
            if *address > 30 {
                anyhow::bail!("GPIB address {} is out of range (0-30).", address);
            }
            Ok(Connection::Gpib {
                board: *board,
                address: *address,
            })
        }
        Connection::Tcpip { ip, port } => Ok(Connection::Tcpip {
            ip: ip.clone(),
            port: *port,
        }),
        Connection::Usbtmc { resource } => {
            if resource.trim().is_empty() {
                bail!("USB-TMC VISA resource must not be empty.");
            }
            Ok(Connection::Usbtmc {
                resource: resource.clone(),
            })
        }
        Connection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        } => {
            validate_prologix_address(*address)?;
            validate_prologix_read_timeout_ms(*read_timeout_ms)?;
            if host.trim().is_empty() {
                bail!("Prologix TCP host must not be empty.");
            }
            if *port == 0 {
                bail!("Prologix TCP port must be in 1..=65535.");
            }
            Ok(Connection::PrologixTcp {
                host: host.clone(),
                port: *port,
                address: *address,
                read_timeout_ms: *read_timeout_ms,
            })
        }
        Connection::PrologixSerial {
            path,
            address,
            baud_rate,
            read_timeout_ms,
        } => {
            validate_prologix_address(*address)?;
            validate_prologix_read_timeout_ms(*read_timeout_ms)?;
            if path.trim().is_empty() {
                bail!("Prologix serial path must not be empty.");
            }
            if *baud_rate == 0 {
                bail!("Prologix serial baud_rate must be positive.");
            }
            Ok(Connection::PrologixSerial {
                path: path.clone(),
                address: *address,
                baud_rate: *baud_rate,
                read_timeout_ms: *read_timeout_ms,
            })
        }
    }
}

pub fn validate_oscilloscope(cfg: &Config) -> Result<()> {
    let instruments = cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("No instruments defined in configuration."))?;

    let osc_cfg = &instruments.oscilloscope;

    let endpoint = validate_connection(&osc_cfg.connection)?;

    match osc_cfg.model.as_str() {
        "DHO5108" => match endpoint {
            Connection::Tcpip { .. } | Connection::Usbtmc { .. } => {}
            _ => {
                bail!("DHO5108 must be connected over TCP/IP or USB-TMC.");
            }
        },
        other => {
            bail!("Unknown oscilloscope model: {other}");
        }
    };
    Ok(())
}

pub fn validate_fg(cfg: &Config) -> Result<()> {
    let instruments = cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("No instruments defined in configuration."))?;

    let fg_cfg = instruments
        .function_generator
        .as_ref()
        .ok_or_else(|| anyhow!("Function generator configuration is missing."))?;

    let endpoint = validate_connection(&fg_cfg.connection)?;

    match fg_cfg.model.as_str() {
        "WF1946B" => match endpoint {
            Connection::Gpib { .. }
            | Connection::PrologixTcp { .. }
            | Connection::PrologixSerial { .. } => {}
            _ => {
                bail!("WF1946B must be connected over GPIB or Prologix.");
            }
        },
        other => {
            bail!("Unknown function generator model: {other}");
        }
    };
    Ok(())
}

fn validate_prologix_address(address: u8) -> Result<()> {
    if address > 30 {
        bail!("Prologix GPIB address {address} is out of range (0-30).");
    }
    Ok(())
}

fn validate_prologix_read_timeout_ms(read_timeout_ms: u16) -> Result<()> {
    if !(1..=3000).contains(&read_timeout_ms) {
        bail!("Prologix read_timeout_ms {read_timeout_ms} is out of range (1-3000).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_usbtmc_resource() {
        let error = validate_connection(&Connection::Usbtmc {
            resource: "  ".to_string(),
        })
        .unwrap_err();

        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn accepts_prologix_generator_connections() {
        assert!(
            validate_connection(&Connection::PrologixTcp {
                host: "192.168.1.10".to_string(),
                port: 1234,
                address: 11,
                read_timeout_ms: 3000,
            })
            .is_ok()
        );

        assert!(
            validate_connection(&Connection::PrologixSerial {
                path: "/dev/cu.usbserial-1".to_string(),
                address: 11,
                baud_rate: 115_200,
                read_timeout_ms: 3000,
            })
            .is_ok()
        );
    }
}
