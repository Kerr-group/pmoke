use crate::instruments::Result;
use gpib_rs::Instrument;

pub struct WF1946B {
    inst: Instrument,
    use_crlf: bool,
}

impl WF1946B {
    pub fn open(pad: i32) -> Result<Self> {
        Ok(Self {
            inst: Instrument::open(pad)?,
            use_crlf: false,
        })
    }

    pub fn open_with(pad: i32, timeout_secs: u64, use_crlf: bool) -> Result<Self> {
        let inst = Instrument::open_with(0, pad, timeout_secs)?;
        Ok(Self { inst, use_crlf })
    }

    pub fn identify(&self) -> Result<String> {
        let res = if self.use_crlf {
            self.inst.query_crlf("*IDN?")?
        } else {
            self.inst.query_line("*IDN?")?
        };
        Ok(res)
    }

    pub fn trigger(&self) -> Result<()> {
        self.inst.write_line("*TRG")?;
        Ok(())
    }
}
