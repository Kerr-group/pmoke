#[path = "keithley2010.rs"]
mod driver;

use crate::registry::{
    InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind, TransportKind,
};

pub use driver::Keithley2010;

pub const MODEL: &str = "Keithley2010";
pub const TRANSPORTS: &[TransportKind] = &[
    TransportKind::Gpib,
    TransportKind::PrologixTcp,
    TransportKind::PrologixSerial,
];
pub const PROTOCOLS: &[ProtocolKind] = &[ProtocolKind::Scpi];
pub const CAPABILITIES: &[InstrumentCapability] = &[
    InstrumentCapability::ScpiIdentify,
    InstrumentCapability::ScpiMeasureRead,
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::Multimeter,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    description: "Keithley 2010 multimeter",
};
