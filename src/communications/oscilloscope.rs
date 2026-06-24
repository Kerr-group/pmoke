use crate::communications::validator::validate_oscilloscope;
use crate::config::{Config, Connection};
use anyhow::{Result, anyhow};
use instruments::rigol::{DHO5108, DhoHorizontalSettings, DhoRawWaveform, DhoRawWaveformWritten};
use std::io::Write;

pub enum Oscilloscope {
    DHO5108(DHO5108),
}

pub struct OscilloscopeHandler {
    inner: Oscilloscope,
}

impl OscilloscopeHandler {
    pub fn initialize(cfg: &Config) -> Result<Self> {
        validate_oscilloscope(cfg)?;

        let osc_cfg = &cfg.instruments.as_ref().unwrap().oscilloscope;

        let model = osc_cfg.model.as_str();
        let connection = &osc_cfg.connection;

        let osc = match (model, connection) {
            ("DHO5108", Connection::Tcpip { ip, port }) => {
                let dho = DHO5108::open(ip, *port, None)?;
                Oscilloscope::DHO5108(dho)
            }
            ("DHO5108", Connection::Usbtmc { resource }) => {
                let dho = DHO5108::open_usbtmc(resource, None)?;
                Oscilloscope::DHO5108(dho)
            }
            (other, _) => return Err(anyhow!("Unknown oscilloscope model: {other}")),
        };

        Ok(Self { inner: osc })
    }
}

impl OscilloscopeHandler {
    #[allow(dead_code)]
    pub fn identify(&mut self) -> Result<String> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.identify()?),
        }
    }

    #[allow(dead_code)]
    pub fn set_single(&mut self) -> Result<()> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.set_single()?),
            #[allow(unreachable_patterns)]
            _ => Err(anyhow!("set_single is not supported on this oscilloscope")),
        }
    }

    #[allow(dead_code)]
    pub fn fetch(&mut self, ch: u8, depth: usize) -> Result<Vec<f64>> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.fetch(ch, depth)?),
            #[allow(unreachable_patterns)]
            _ => Err(anyhow!("fetch is not supported on this oscilloscope")),
        }
    }

    #[allow(dead_code)]
    pub fn fetch_raw_word(&mut self, ch: u8, depth: usize) -> Result<DhoRawWaveform> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.fetch_raw_word(ch, depth)?),
            #[allow(unreachable_patterns)]
            _ => Err(anyhow!(
                "raw WORD fetch is not supported on this oscilloscope"
            )),
        }
    }

    #[allow(dead_code)]
    pub fn fetch_raw_word_into<W: Write>(
        &mut self,
        ch: u8,
        depth: usize,
        writer: &mut W,
    ) -> Result<DhoRawWaveformWritten> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.fetch_raw_word_into(ch, depth, writer)?),
            #[allow(unreachable_patterns)]
            _ => Err(anyhow!(
                "raw WORD fetch is not supported on this oscilloscope"
            )),
        }
    }

    pub fn query_horizontal_settings(&mut self) -> Result<DhoHorizontalSettings> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.query_horizontal_settings()?),
        }
    }

    pub fn query_memory_depth(&mut self) -> Result<usize> {
        match &mut self.inner {
            Oscilloscope::DHO5108(dev) => Ok(dev.query_memory_depth()?),
        }
    }
}
