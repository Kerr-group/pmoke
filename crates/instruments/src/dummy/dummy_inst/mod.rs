#[path = "dummy_inst.rs"]
mod driver;

use crate::registry::{
    InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind, TransportKind,
};

pub use driver::{DummyInstrument, DummyResult};

pub const MODEL: &str = "DummyInstrument";
pub const TRANSPORTS: &[TransportKind] = &[TransportKind::Dummy];
pub const PROTOCOLS: &[ProtocolKind] = &[ProtocolKind::VendorSpecific];
pub const CAPABILITIES: &[InstrumentCapability] = &[InstrumentCapability::VendorIdentify];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::FunctionGenerator,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    description: "In-memory dummy instrument",
};
