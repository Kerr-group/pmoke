use std::time::{Duration, Instant};

use crate::instruments::Result;
use gpib_rs::GpibError;

#[derive(Clone)]
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    #[inline]
    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 11) as f64) / ((u64::MAX >> 11) as f64) - 0.5
    }
}

pub struct DummyInstrument {
    pad: i32,
    use_crlf: bool,
    timeout: Duration,
    last_io: Instant,
    prng: Lcg,
}

impl DummyInstrument {
    pub fn open(pad: i32) -> Result<Self> {
        Self::open_with(pad, 10, false)
    }

    pub fn open_with(pad: i32, timeout_secs: u64, use_crlf: bool) -> Result<Self> {
        const SEED64: u64 = 0xC0FF_EEAD_F00D_BEEF_u64;

        Ok(Self {
            pad,
            use_crlf,
            timeout: std::time::Duration::from_secs(timeout_secs.max(1)),
            last_io: std::time::Instant::now(),
            prng: Lcg::new((pad as u64) ^ SEED64),
        })
    }

    pub fn set_timeout_secs(&mut self, secs: u64) -> Result<()> {
        self.timeout = Duration::from_secs(secs.max(1));
        Ok(())
    }

    pub fn clear(&mut self) -> Result<()> {
        self.touch();
        Ok(())
    }

    pub fn write_line(&mut self, s: &str) -> Result<()> {
        self.touch();
        self.handle_write(s.trim_end_matches(&['\r', '\n'][..]));
        Ok(())
    }

    pub fn write_crlf(&mut self, s: &str) -> Result<()> {
        self.use_crlf = true;
        self.write_line(s)
    }

    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.touch();
        let s = String::from_utf8_lossy(bytes);
        self.handle_write(s.trim_end_matches(&['\r', '\n'][..]));
        Ok(())
    }

    pub fn read_string(&mut self) -> Result<String> {
        self.touch();
        Ok(String::from("0"))
    }

    pub fn read_all(&mut self) -> Result<Vec<u8>> {
        self.touch();
        Ok(self.read_string()?.into_bytes())
    }

    pub fn query_line(&mut self, cmd: &str) -> Result<String> {
        self.touch();
        self.simulate_query(cmd)
    }

    pub fn query_crlf(&mut self, cmd: &str) -> Result<String> {
        self.use_crlf = true;
        self.query_line(cmd)
    }

    pub fn query_all_line(&mut self, cmd: &str) -> Result<Vec<u8>> {
        Ok(self.query_line(cmd)?.into_bytes())
    }

    fn touch(&mut self) {
        self.last_io = Instant::now();
    }

    fn simulate_query(&mut self, raw: &str) -> Result<String> {
        let cmd = raw.trim();
        match cmd.to_ascii_uppercase().as_str() {
            "*IDN?" => Ok("DUMMY INSTRUMENTS".to_string()),
            ":MEAS:VOLT:DC?" => Ok(format!("{}", self.value_for("VDC"))),
            ":MEAS:VOLT:AC?" => Ok(format!("{}", self.value_for("VAC"))),
            ":MEAS:CURR:DC?" => Ok(format!("{}", self.value_for("IDC"))),
            ":MEAS:CURR:AC?" => Ok(format!("{}", self.value_for("IAC"))),
            ":MEAS:RES?" => Ok(format!("{}", self.value_for("R2W"))),
            ":MEAS:FRES?" => Ok(format!("{}", self.value_for("R4W"))),
            s if s.starts_with(":CONF:") || s.starts_with(":SENS:") || s.starts_with(":SYST:") => {
                Ok(String::from("0"))
            }
            _ => Ok(format!("OK ({cmd})")),
        }
    }

    fn handle_write(&mut self, raw: &str) {
        let _ = raw;
    }

    fn value_for(&mut self, kind: &str) -> f64 {
        let base = match kind {
            "VDC" => 1.2345,
            "VAC" => 0.9876,
            "IDC" => 0.0123,
            "IAC" => 0.0456,
            "R2W" => 1000.0,
            "R4W" => 999.99,
            _ => 0.0,
        };
        let span = match kind {
            "VDC" | "VAC" => 10.0,
            "IDC" | "IAC" => 0.1,
            "R2W" | "R4W" => 10000.0,
            _ => 1.0,
        };
        base + self.prng.next_f64() * span * 1e-3
    }
}

impl Default for DummyInstrument {
    fn default() -> Self {
        Self::open_with(17, 10, false).unwrap()
    }
}

impl std::fmt::Debug for DummyInstrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DummyInstrument")
            .field("pad", &self.pad)
            .field("use_crlf", &self.use_crlf)
            .field("timeout_s", &self.timeout.as_secs())
            .finish()
    }
}

pub type DummyResult<T> = Result<T>;

#[allow(dead_code)]
fn dummy_err(ctx: &'static str) -> GpibError {
    GpibError {
        ctx,
        ibsta: 0,
        iberr: 0,
        note: None,
    }
}
