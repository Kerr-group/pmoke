use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::client::Prologix;
use crate::config::{ControllerConfig, DEFAULT_PORT, DEFAULT_TIMEOUT_MS};
use crate::error::{Error, Result};

impl Prologix<TcpStream> {
    pub fn tcp(host: impl Into<String>, port: u16) -> TcpBuilder {
        TcpBuilder::new(host, port)
    }

    pub fn tcp_default_port(host: impl Into<String>) -> TcpBuilder {
        TcpBuilder::new(host, DEFAULT_PORT)
    }
}

#[derive(Debug, Clone)]
pub struct TcpBuilder {
    host: String,
    port: u16,
    address: u8,
    timeout: Duration,
    read_timeout_ms: u16,
}

impl TcpBuilder {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            address: 0,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS as u64),
            read_timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    pub fn address(mut self, address: u8) -> Self {
        self.address = address;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn read_timeout_ms(mut self, read_timeout_ms: u16) -> Self {
        self.read_timeout_ms = read_timeout_ms;
        self
    }

    pub fn open(self) -> Result<Prologix<TcpStream>> {
        let config = ControllerConfig::with_read_timeout_ms(self.address, self.read_timeout_ms)?;
        let addresses: Vec<_> = (self.host.as_str(), self.port).to_socket_addrs()?.collect();
        if addresses.is_empty() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no socket address resolved for {}:{}", self.host, self.port),
            )));
        }

        let mut last_error = None;
        let mut stream = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, self.timeout) {
                Ok(candidate) => {
                    stream = Some(candidate);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let stream = stream.ok_or_else(|| {
            Error::Io(last_error.unwrap_or_else(|| {
                io::Error::other(format!("failed to connect to {}:{}", self.host, self.port))
            }))
        })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let mut controller = Prologix::with_config(stream, config);
        controller.initialize()?;
        Ok(controller)
    }
}
