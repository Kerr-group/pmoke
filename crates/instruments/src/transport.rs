use crate::Result;
use std::fmt;
#[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
use std::time::Duration;

pub trait ScpiTransport {
    fn write_line(&mut self, command: &str) -> Result<()>;
    fn query_line(&mut self, command: &str) -> Result<String>;

    fn set_timeout_secs(&mut self, _secs: u64) -> Result<()> {
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        Ok(())
    }
}

pub type BoxedScpiTransport = Box<dyn ScpiTransport + Send>;

#[derive(Debug, Clone)]
pub enum ScpiConnection {
    Gpib {
        board: i32,
        address: i32,
        timeout_secs: u64,
        use_crlf: bool,
    },
    PrologixTcp {
        host: String,
        port: u16,
        address: u8,
        read_timeout_ms: u16,
    },
    PrologixSerial {
        path: String,
        address: u8,
        baud_rate: u32,
        read_timeout_ms: u16,
    },
}

impl fmt::Display for ScpiConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gpib { board, address, .. } => write!(f, "GPIB board {board}, address {address}"),
            Self::PrologixTcp {
                host,
                port,
                address,
                ..
            } => write!(f, "Prologix TCP {host}:{port}, address {address}"),
            Self::PrologixSerial { path, address, .. } => {
                write!(f, "Prologix serial {path}, address {address}")
            }
        }
    }
}

pub fn open_scpi_transport(connection: &ScpiConnection) -> Result<BoxedScpiTransport> {
    match connection {
        ScpiConnection::Gpib {
            board,
            address,
            timeout_secs,
            use_crlf,
        } => open_gpib_transport(*board, *address, *timeout_secs, *use_crlf),
        ScpiConnection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        } => open_prologix_tcp_transport(host, *port, *address, *read_timeout_ms),
        ScpiConnection::PrologixSerial {
            path,
            address,
            baud_rate,
            read_timeout_ms,
        } => open_prologix_serial_transport(path, *address, *baud_rate, *read_timeout_ms),
    }
}

#[cfg(feature = "gpib")]
fn open_gpib_transport(
    board: i32,
    address: i32,
    timeout_secs: u64,
    use_crlf: bool,
) -> Result<BoxedScpiTransport> {
    Ok(Box::new(GpibTransport::open_with(
        board,
        address,
        timeout_secs,
        use_crlf,
    )?))
}

#[cfg(not(feature = "gpib"))]
fn open_gpib_transport(
    _board: i32,
    _address: i32,
    _timeout_secs: u64,
    _use_crlf: bool,
) -> Result<BoxedScpiTransport> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this instruments build does not include GPIB support",
    )
    .into())
}

#[cfg(feature = "prologix-tcp")]
fn open_prologix_tcp_transport(
    host: &str,
    port: u16,
    address: u8,
    read_timeout_ms: u16,
) -> Result<BoxedScpiTransport> {
    let controller = prologix_rs::Prologix::tcp(host, port)
        .address(address)
        .timeout(prologix_io_timeout(read_timeout_ms))
        .read_timeout_ms(read_timeout_ms)
        .open()?;
    Ok(Box::new(controller))
}

#[cfg(not(feature = "prologix-tcp"))]
fn open_prologix_tcp_transport(
    _host: &str,
    _port: u16,
    _address: u8,
    _read_timeout_ms: u16,
) -> Result<BoxedScpiTransport> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this instruments build does not include Prologix TCP support",
    )
    .into())
}

#[cfg(feature = "prologix-serial")]
fn open_prologix_serial_transport(
    path: &str,
    address: u8,
    baud_rate: u32,
    read_timeout_ms: u16,
) -> Result<BoxedScpiTransport> {
    let controller = prologix_rs::Prologix::serial(path)
        .address(address)
        .baud_rate(baud_rate)
        .timeout(prologix_io_timeout(read_timeout_ms))
        .read_timeout_ms(read_timeout_ms)
        .open()?;
    Ok(Box::new(controller))
}

#[cfg(not(feature = "prologix-serial"))]
fn open_prologix_serial_transport(
    _path: &str,
    _address: u8,
    _baud_rate: u32,
    _read_timeout_ms: u16,
) -> Result<BoxedScpiTransport> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this instruments build does not include Prologix serial support",
    )
    .into())
}

#[cfg(feature = "gpib")]
pub struct GpibTransport {
    instrument: gpib_rs::Instrument,
    use_crlf: bool,
}

#[cfg(feature = "gpib")]
impl GpibTransport {
    pub fn open(address: i32) -> Result<Self> {
        Self::open_with(0, address, 10, false)
    }

    pub fn open_with(board: i32, address: i32, timeout_secs: u64, use_crlf: bool) -> Result<Self> {
        let instrument = gpib_rs::Instrument::open_with(board, address, timeout_secs)?;
        Ok(Self {
            instrument,
            use_crlf,
        })
    }

    pub fn from_instrument(instrument: gpib_rs::Instrument) -> Self {
        Self {
            instrument,
            use_crlf: false,
        }
    }

    pub fn use_crlf(mut self, use_crlf: bool) -> Self {
        self.use_crlf = use_crlf;
        self
    }

    pub fn into_inner(self) -> gpib_rs::Instrument {
        self.instrument
    }
}

#[cfg(feature = "gpib")]
impl ScpiTransport for GpibTransport {
    fn write_line(&mut self, command: &str) -> Result<()> {
        if self.use_crlf {
            self.instrument.write_crlf(command)?;
        } else {
            self.instrument.write_line(command)?;
        }
        Ok(())
    }

    fn query_line(&mut self, command: &str) -> Result<String> {
        if self.use_crlf {
            Ok(self.instrument.query_crlf(command)?)
        } else {
            Ok(self.instrument.query_line(command)?)
        }
    }

    fn set_timeout_secs(&mut self, secs: u64) -> Result<()> {
        self.instrument.set_timeout_secs(secs)?;
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.instrument.clear()?;
        Ok(())
    }
}

#[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
impl<T> ScpiTransport for prologix_rs::Prologix<T>
where
    T: std::io::Read + std::io::Write + Send,
{
    fn write_line(&mut self, command: &str) -> Result<()> {
        self.write(command)?;
        Ok(())
    }

    fn query_line(&mut self, command: &str) -> Result<String> {
        Ok(self.query(command)?)
    }
}

#[cfg(any(feature = "prologix-tcp", feature = "prologix-serial"))]
fn prologix_io_timeout(read_timeout_ms: u16) -> Duration {
    Duration::from_millis(u64::from(read_timeout_ms))
}

#[cfg(all(test, feature = "prologix-tcp"))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    #[test]
    fn prologix_read_timeout_also_limits_host_io() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            for _ in 0..8 {
                let mut command = String::new();
                reader.read_line(&mut command).unwrap();
            }
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });

        let connection = ScpiConnection::PrologixTcp {
            host: "127.0.0.1".to_string(),
            port,
            address: 17,
            read_timeout_ms: 50,
        };
        let mut transport = open_scpi_transport(&connection).unwrap();
        let started = Instant::now();
        let _error = transport.query_line("*IDN?").unwrap_err();
        let elapsed = started.elapsed();

        release_tx.send(()).unwrap();
        drop(transport);
        server.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(1),
            "query exceeded configured host timeout: {elapsed:?}"
        );
    }
}
