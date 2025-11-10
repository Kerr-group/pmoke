use crate::communications::validator::validate_fg;
use crate::config::{Config, Connection};
use anyhow::{Result, anyhow};
use instruments::nf::WF1946B;

pub enum FG {
    WF1946B(WF1946B),
}

pub struct FGHandler {
    inner: FG,
}

impl FGHandler {
    pub fn initialize(cfg: &Config) -> Result<Self> {
        validate_fg(cfg)?;

        let fg_cfg = cfg
            .instruments
            .as_ref()
            .unwrap()
            .function_generator
            .as_ref()
            .unwrap();

        let model = fg_cfg.model.as_str();
        let connection = &fg_cfg.connection;

        let fg = match (model, connection) {
            ("WF1946B", Connection::Gpib { board: _, address }) => {
                let wf = WF1946B::open(*address as i32)?;
                FG::WF1946B(wf)
            }
            (other, _) => return Err(anyhow!("Unknown oscilloscope model: {other}")),
        };

        Ok(Self { inner: fg })
    }
}

impl FGHandler {
    #[allow(dead_code)]
    pub fn identify(&mut self) -> Result<String> {
        match &mut self.inner {
            FG::WF1946B(dev) => Ok(dev.identify()?),
        }
    }

    #[allow(dead_code)]
    pub fn trigger(&mut self) -> Result<()> {
        match &mut self.inner {
            FG::WF1946B(dev) => Ok(dev.trigger()?),
            #[allow(unreachable_patterns)]
            _ => Err(anyhow!(
                "trigger is not supported on this function generator"
            )),
        }
    }
}
