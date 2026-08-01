#[path = "dho5108.rs"]
mod driver;

use crate::registry::{
    InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind, TransportKind,
};

pub use driver::{
    DHO5108, DhoHorizontalSettings, DhoRawWaveform, DhoRawWaveformWritten, DhoTriggerStatus,
    DhoWaveformPreamble,
};

pub const MODEL: &str = "DHO5108";
pub const TRANSPORTS: &[TransportKind] = &[TransportKind::Tcpip, TransportKind::Usbtmc];
pub const PROTOCOLS: &[ProtocolKind] = &[ProtocolKind::Scpi];
pub const CAPABILITIES: &[InstrumentCapability] = &[
    InstrumentCapability::ScpiIdentify,
    InstrumentCapability::Screenshot,
    InstrumentCapability::WaveformFetch,
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::Oscilloscope,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    description: "Rigol DHO5108 oscilloscope",
};
