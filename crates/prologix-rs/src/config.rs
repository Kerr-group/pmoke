use crate::error::{Error, Result};

pub const DEFAULT_PORT: u16 = 1234;
pub const DEFAULT_TIMEOUT_MS: u16 = 3000;
pub const MAX_GPIB_ADDRESS: u8 = 30;
pub const MIN_READ_TIMEOUT_MS: u16 = 1;
pub const MAX_READ_TIMEOUT_MS: u16 = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerConfig {
    address: u8,
    read_timeout_ms: u16,
}

impl ControllerConfig {
    pub fn new(address: u8) -> Result<Self> {
        Self::with_read_timeout_ms(address, DEFAULT_TIMEOUT_MS)
    }

    pub fn with_read_timeout_ms(address: u8, read_timeout_ms: u16) -> Result<Self> {
        validate_address(address)?;
        validate_read_timeout_ms(read_timeout_ms)?;
        Ok(Self {
            address,
            read_timeout_ms,
        })
    }

    pub fn address(self) -> u8 {
        self.address
    }

    pub fn read_timeout_ms(self) -> u16 {
        self.read_timeout_ms
    }
}

pub(crate) fn validate_address(address: u8) -> Result<()> {
    if address <= MAX_GPIB_ADDRESS {
        Ok(())
    } else {
        Err(Error::InvalidAddress(address))
    }
}

pub(crate) fn validate_read_timeout_ms(read_timeout_ms: u16) -> Result<()> {
    if (MIN_READ_TIMEOUT_MS..=MAX_READ_TIMEOUT_MS).contains(&read_timeout_ms) {
        Ok(())
    } else {
        Err(Error::InvalidReadTimeoutMs(read_timeout_ms))
    }
}
