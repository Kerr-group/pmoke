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
    }
}

pub fn validate_oscilloscope(cfg: &Config) -> Result<()> {
    let instruments = cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("No instruments defined in configuration."))?;

    let osc_cfg = instruments
        .oscilloscope
        .as_ref()
        .ok_or_else(|| anyhow!("Oscilloscope configuration is missing."))?;

    let endpoint = validate_connection(&osc_cfg.connection)?;

    match osc_cfg.model.as_str() {
        "DHO5108" => match endpoint {
            Connection::Tcpip { .. } => {}
            _ => {
                bail!("DHO5108 must be connected over TCP/IP.");
            }
        },
        other => {
            bail!("Unknown oscilloscope model: {other}");
        }
    };
    Ok(())
}
