use crate::communications::validator::validate_fg;
use crate::config::{Config, Connection};
use anyhow::{Context, Result, anyhow};
use instruments::nf::WF1946B;
use instruments::transport::{ScpiConnection, open_scpi_transport};

pub enum FG {
    WF1946B(WF1946B),
}

pub struct FGHandler {
    inner: FG,
}

impl FGHandler {
    pub fn initialize(cfg: &Config) -> Result<Self> {
        validate_fg(cfg)?;

        let fg_cfg = cfg
            .instruments
            .as_ref()
            .context("instrument configuration is missing")?
            .function_generator
            .as_ref()
            .context("function generator configuration is missing")?;

        let model = fg_cfg.model.as_str();
        let connection = &fg_cfg.connection;

        let fg = open_function_generator(model, connection)?;

        Ok(Self { inner: fg })
    }
}

fn open_function_generator(model: &str, connection: &Connection) -> Result<FG> {
    match model {
        "WF1946B" => {
            let transport = open_scpi_transport(&scpi_connection(connection)?)?;
            Ok(FG::WF1946B(WF1946B::new(transport)))
        }
        other => Err(anyhow!("unknown function generator model: {other}")),
    }
}

fn scpi_connection(connection: &Connection) -> Result<ScpiConnection> {
    match connection {
        Connection::Gpib { board, address } => Ok(ScpiConnection::Gpib {
            board: *board as i32,
            address: *address as i32,
            timeout_secs: 10,
            use_crlf: false,
        }),
        Connection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        } => Ok(ScpiConnection::PrologixTcp {
            host: host.clone(),
            port: *port,
            address: *address,
            read_timeout_ms: *read_timeout_ms,
        }),
        Connection::PrologixSerial {
            path,
            address,
            baud_rate,
            read_timeout_ms,
        } => Ok(ScpiConnection::PrologixSerial {
            path: path.clone(),
            address: *address,
            baud_rate: *baud_rate,
            read_timeout_ms: *read_timeout_ms,
        }),
        Connection::Tcpip { .. } | Connection::Usbtmc { .. } => Err(anyhow!(
            "function generator requires a SCPI GPIB or Prologix connection"
        )),
    }
}

impl FGHandler {
    #[allow(dead_code)]
    pub fn identify(&mut self) -> Result<String> {
        match &mut self.inner {
            FG::WF1946B(dev) => Ok(dev.identify()?),
        }
    }

    #[allow(dead_code)]
    pub fn trigger(&mut self) -> Result<()> {
        match &mut self.inner {
            FG::WF1946B(dev) => Ok(dev.trigger()?),
            #[allow(unreachable_patterns)]
            _ => Err(anyhow!(
                "trigger is not supported on this function generator"
            )),
        }
    }
}
