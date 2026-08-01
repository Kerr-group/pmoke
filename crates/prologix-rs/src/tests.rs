use super::*;
use std::collections::VecDeque;
use std::io::{self, Read, Write};

#[derive(Debug, Default)]
struct MockIo {
    written: Vec<u8>,
    reads: VecDeque<io::Result<Vec<u8>>>,
}

impl MockIo {
    fn with_read(data: &'static [u8]) -> Self {
        Self {
            written: Vec::new(),
            reads: VecDeque::from([Ok(data.to_vec())]),
        }
    }

    fn written_text(&self) -> String {
        String::from_utf8(self.written.clone()).unwrap()
    }
}

impl Read for MockIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(next) = self.reads.pop_front() else {
            return Ok(0);
        };
        let data = next?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        if n < data.len() {
            self.reads.push_front(Ok(data[n..].to_vec()));
        }
        Ok(n)
    }
}

impl Write for MockIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn initialize_writes_safe_controller_defaults() {
    let io = MockIo::default();
    let mut controller = Prologix::with_config(
        io,
        ControllerConfig::with_read_timeout_ms(11, 2500).unwrap(),
    );

    controller.initialize().unwrap();

    assert_eq!(
        controller.into_inner().written_text(),
        "++mode 1\n++addr 11\n++auto 0\n++eoi 1\n++eos 0\n++read_tmo_ms 2500\n"
    );
}

#[test]
fn query_uses_explicit_eoi_read_and_trims_text_response() {
    let io = MockIo::with_read(b"NF,WF1946B,0,1\r\n");
    let mut controller = Prologix::new(io, 11).unwrap();

    let response = controller.query("*IDN?").unwrap();

    assert_eq!(response, "NF,WF1946B,0,1");
    assert_eq!(
        controller.into_inner().written_text(),
        "*IDN?\n++read eoi\n"
    );
}

#[test]
fn controller_version_queries_the_adapter_without_gpib_read() {
    let io = MockIo::with_read(b"Prologix GPIB-ETHERNET Controller version 6.101\r\n");
    let mut controller = Prologix::new(io, 17).unwrap();

    let response = controller.controller_version().unwrap();

    assert_eq!(response, "Prologix GPIB-ETHERNET Controller version 6.101");
    assert_eq!(controller.into_inner().written_text(), "++ver\n");
}

#[test]
fn validates_address_and_timeout() {
    assert!(matches!(
        ControllerConfig::new(31),
        Err(Error::InvalidAddress(31))
    ));
    assert!(matches!(
        ControllerConfig::with_read_timeout_ms(1, 0),
        Err(Error::InvalidReadTimeoutMs(0))
    ));
    assert!(matches!(
        ControllerConfig::with_read_timeout_ms(1, 3001),
        Err(Error::InvalidReadTimeoutMs(3001))
    ));
}
