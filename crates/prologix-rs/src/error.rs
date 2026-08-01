use std::fmt;
use std::io;

use crate::config::{MAX_GPIB_ADDRESS, MAX_READ_TIMEOUT_MS, MIN_READ_TIMEOUT_MS};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    InvalidAddress(u8),
    InvalidReadTimeoutMs(u16),
    InvalidControllerQuery(&'static str),
    EmptyControllerResponse,
    ResponseNotUtf8(std::string::FromUtf8Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::InvalidAddress(address) => write!(
                f,
                "invalid Prologix GPIB address {address}; expected 0..={MAX_GPIB_ADDRESS}"
            ),
            Self::InvalidReadTimeoutMs(timeout_ms) => write!(
                f,
                "invalid Prologix read timeout {timeout_ms} ms; expected {MIN_READ_TIMEOUT_MS}..={MAX_READ_TIMEOUT_MS}"
            ),
            Self::InvalidControllerQuery(reason) => {
                write!(f, "invalid Prologix controller query: {reason}")
            }
            Self::EmptyControllerResponse => {
                write!(f, "Prologix controller returned an empty response")
            }
            Self::ResponseNotUtf8(err) => write!(f, "Prologix response is not UTF-8: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::ResponseNotUtf8(err) => Some(err),
            Self::InvalidAddress(_)
            | Self::InvalidReadTimeoutMs(_)
            | Self::InvalidControllerQuery(_)
            | Self::EmptyControllerResponse => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::ResponseNotUtf8(value)
    }
}
