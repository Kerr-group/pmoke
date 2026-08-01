use std::{error::Error, fmt, io};

#[derive(Debug)]
pub enum InstrumentError {
    #[cfg(feature = "gpib")]
    Gpib(GpibError),
    Io(io::Error),
    #[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
    Prologix(prologix_rs::Error),
}

pub type Result<T> = std::result::Result<T, InstrumentError>;

impl InstrumentError {
    pub fn is_timeout(&self) -> bool {
        match self {
            #[cfg(feature = "gpib")]
            Self::Gpib(error) => error.iberr == 6,
            Self::Io(error) => is_io_timeout(error),
            #[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
            Self::Prologix(prologix_rs::Error::Io(error)) => is_io_timeout(error),
            #[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
            Self::Prologix(_) => false,
        }
    }
}

fn is_io_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

#[cfg(feature = "gpib")]
use gpib_rs::GpibError;

#[cfg(feature = "gpib")]
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

#[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
impl From<prologix_rs::Error> for InstrumentError {
    fn from(e: prologix_rs::Error) -> Self {
        InstrumentError::Prologix(e)
    }
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "gpib")]
            InstrumentError::Gpib(e) => write!(f, "GPIB error: {:?}", e),
            InstrumentError::Io(e) => write!(f, "I/O error: {}", e),
            #[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
            InstrumentError::Prologix(e) => write!(f, "Prologix error: {}", e),
        }
    }
}

impl Error for InstrumentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "gpib")]
            InstrumentError::Gpib(e) => Some(e),
            InstrumentError::Io(e) => Some(e),
            #[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
            InstrumentError::Prologix(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_host_io_timeouts() {
        assert!(InstrumentError::Io(io::Error::new(io::ErrorKind::TimedOut, "late")).is_timeout());
        assert!(
            InstrumentError::Io(io::Error::new(io::ErrorKind::WouldBlock, "late")).is_timeout()
        );
        assert!(!InstrumentError::Io(io::Error::other("broken")).is_timeout());
    }
}
