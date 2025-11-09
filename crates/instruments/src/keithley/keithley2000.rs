use crate::instruments::Result;
use gpib_rs::Instrument;

// #[derive(Debug, Clone, Copy)]
// pub enum Function {
//     VoltDC,
// }
//
// #[allow(dead_code)]
// impl Function {
//     fn scpi_token(self) -> &'static str {
//         match self {
//             Function::VoltDC => "VOLT:DC",
//         }
//     }
// }

pub struct Keithley2000 {
    inst: Instrument,
    use_crlf: bool,
}

impl Keithley2000 {
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

    pub fn set_crlf(&mut self, on: bool) {
        self.use_crlf = on;
    }

    pub fn set_timeout_secs(&mut self, secs: u64) -> Result<()> {
        self.inst.set_timeout_secs(secs)?;
        Ok(())
    }

    pub fn identify(&self) -> Result<String> {
        let res = if self.use_crlf {
            self.inst.query_crlf("*IDN?")?
        } else {
            self.inst.query_line("*IDN?")?
        };
        Ok(res)
    }
}
