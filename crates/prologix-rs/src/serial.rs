use std::io;
use std::time::Duration;

use crate::client::Prologix;
use crate::config::{ControllerConfig, DEFAULT_TIMEOUT_MS};
use crate::error::Result;

impl Prologix<Box<dyn serialport::SerialPort>> {
    pub fn serial(path: impl Into<String>) -> SerialBuilder {
        SerialBuilder::new(path)
    }
}

#[derive(Debug, Clone)]
pub struct SerialBuilder {
    path: String,
    address: u8,
    baud_rate: u32,
    timeout: Duration,
    read_timeout_ms: u16,
}

impl SerialBuilder {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            address: 0,
            baud_rate: 115_200,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS as u64),
            read_timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    pub fn address(mut self, address: u8) -> Self {
        self.address = address;
        self
    }

    pub fn baud_rate(mut self, baud_rate: u32) -> Self {
        self.baud_rate = baud_rate;
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

    pub fn open(self) -> Result<Prologix<Box<dyn serialport::SerialPort>>> {
        let config = ControllerConfig::with_read_timeout_ms(self.address, self.read_timeout_ms)?;
        let port = serialport::new(self.path, self.baud_rate)
            .timeout(self.timeout)
            .open()
            .map_err(|err| crate::error::Error::Io(io::Error::other(err)))?;
        let mut controller = Prologix::with_config(port, config);
        controller.initialize()?;
        Ok(controller)
    }
}
