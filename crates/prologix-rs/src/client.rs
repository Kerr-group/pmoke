use std::io::{Read, Write};

use crate::config::ControllerConfig;
use crate::error::Result;
use crate::wire::{read_response_bytes, write_line};

pub struct Prologix<T> {
    io: T,
    config: ControllerConfig,
}

impl<T> Prologix<T>
where
    T: Read + Write,
{
    pub fn new(io: T, address: u8) -> Result<Self> {
        Ok(Self {
            io,
            config: ControllerConfig::new(address)?,
        })
    }

    pub fn with_config(io: T, config: ControllerConfig) -> Self {
        Self { io, config }
    }

    pub fn address(&self) -> u8 {
        self.config.address()
    }

    pub fn read_timeout_ms(&self) -> u16 {
        self.config.read_timeout_ms()
    }

    pub fn into_inner(self) -> T {
        self.io
    }

    pub fn initialize(&mut self) -> Result<()> {
        self.controller_command("++mode 1")?;
        self.controller_command(&format!("++addr {}", self.config.address()))?;
        self.controller_command("++auto 0")?;
        self.controller_command("++eoi 1")?;
        self.controller_command("++eos 0")?;
        self.controller_command(&format!("++read_tmo_ms {}", self.config.read_timeout_ms()))?;
        Ok(())
    }

    pub fn controller_command(&mut self, command: &str) -> Result<()> {
        write_line(&mut self.io, command)
    }

    pub fn controller_version(&mut self) -> Result<String> {
        self.controller_command("++ver")?;
        let bytes = read_response_bytes(&mut self.io)?;
        Ok(String::from_utf8(bytes)?
            .trim_end_matches(['\r', '\n'])
            .to_string())
    }

    pub fn write(&mut self, command: &str) -> Result<()> {
        write_line(&mut self.io, command)
    }

    pub fn query(&mut self, command: &str) -> Result<String> {
        let bytes = self.query_bytes(command)?;
        Ok(String::from_utf8(bytes)?
            .trim_end_matches(['\r', '\n'])
            .to_string())
    }

    pub fn query_bytes(&mut self, command: &str) -> Result<Vec<u8>> {
        self.write(command)?;
        self.controller_command("++read eoi")?;
        read_response_bytes(&mut self.io)
    }
}
