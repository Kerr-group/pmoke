#[path = "wf1946b.rs"]
mod driver;

use crate::registry::{
    InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind, TransportKind,
};

pub use driver::WF1946B;

pub const MODEL: &str = "WF1946B";
pub const TRANSPORTS: &[TransportKind] = &[
    TransportKind::Gpib,
    TransportKind::PrologixTcp,
    TransportKind::PrologixSerial,
];
pub const PROTOCOLS: &[ProtocolKind] = &[ProtocolKind::Scpi];
pub const CAPABILITIES: &[InstrumentCapability] = &[
    InstrumentCapability::ScpiIdentify,
    InstrumentCapability::ScpiTrigger,
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::FunctionGenerator,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    description: "NF WF1946B function generator",
};
