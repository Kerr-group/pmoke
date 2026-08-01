#[path = "dho5108.rs"]
mod driver;

use crate::registry::{
    ConnectionExample, InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind,
    TransportKind,
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
pub const EXAMPLES: &[ConnectionExample] = &[
    ConnectionExample {
        transport: TransportKind::Tcpip,
        connection: "tcp://10.249.11.19:55255",
        required_feature: "hw-core",
    },
    ConnectionExample {
        transport: TransportKind::Usbtmc,
        connection: "visa:USB0::0x1AB1::0x0450::DHO5A27090041::INSTR",
        required_feature: "hw-gpib on Windows",
    },
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::Oscilloscope,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    examples: EXAMPLES,
    description: "Rigol DHO5108 oscilloscope",
};
