#[path = "dho5108.rs"]
mod driver;

use crate::registry::{InstrumentRole, InstrumentSpec, TransportKind};

pub use driver::{
    DHO5108, DhoHorizontalSettings, DhoRawWaveform, DhoRawWaveformWritten, DhoTriggerStatus,
    DhoWaveformPreamble,
};

pub const MODEL: &str = "DHO5108";
pub const TRANSPORTS: &[TransportKind] = &[TransportKind::Tcpip, TransportKind::Usbtmc];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::Oscilloscope,
    transports: TRANSPORTS,
    description: "Rigol DHO5108 oscilloscope",
};
