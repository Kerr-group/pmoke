use crate::Result;
use crate::transport::BoxedScpiTransport;

pub struct WF1946B {
    transport: BoxedScpiTransport,
}

impl WF1946B {
    pub fn new(transport: BoxedScpiTransport) -> Self {
        Self { transport }
    }

    pub fn identify(&mut self) -> Result<String> {
        self.transport.query_line("*IDN?")
    }

    pub fn trigger(&mut self) -> Result<()> {
        self.transport.write_line("*TRG")?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
