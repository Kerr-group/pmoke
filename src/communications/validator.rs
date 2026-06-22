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
            Connection::Gpib { .. } => {}
            _ => {
                bail!("WF1946B must be connected over GPIB.");
            }
        },
        other => {
            bail!("Unknown function generator model: {other}");
        }
    };
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
}
