use crate::Result;
use crate::transport::BoxedScpiTransport;

pub struct Keithley2000 {
    transport: BoxedScpiTransport,
}

impl Keithley2000 {
    pub fn new(transport: BoxedScpiTransport) -> Self {
        Self { transport }
    }

    pub fn set_timeout_secs(&mut self, secs: u64) -> Result<()> {
        self.transport.set_timeout_secs(secs)
    }

    pub fn identify(&mut self) -> Result<String> {
        self.transport.query_line("*IDN?")
    }
}
