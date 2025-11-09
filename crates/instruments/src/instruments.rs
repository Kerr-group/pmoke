use gpib_rs::GpibError;
use std::{error::Error, fmt, io};

#[derive(Debug)]
pub enum InstrumentError {
    Gpib(GpibError),
    Io(io::Error),
}

pub type Result<T> = std::result::Result<T, InstrumentError>;

impl From<GpibError> for InstrumentError {
    fn from(e: GpibError) -> Self {
        InstrumentError::Gpib(e)
    }
}
impl From<io::Error> for InstrumentError {
    fn from(e: io::Error) -> Self {
        InstrumentError::Io(e)
    }
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstrumentError::Gpib(e) => write!(f, "GPIB error: {:?}", e),
            InstrumentError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl Error for InstrumentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            InstrumentError::Gpib(e) => Some(e),
            InstrumentError::Io(e) => Some(e),
        }
    }
}
