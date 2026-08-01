#[path = "dummy_inst.rs"]
mod driver;

use crate::registry::{
    ConnectionExample, InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind,
    TransportKind,
};

pub use driver::{DummyInstrument, DummyResult};

pub const MODEL: &str = "DummyInstrument";
pub const TRANSPORTS: &[TransportKind] = &[TransportKind::Dummy];
pub const PROTOCOLS: &[ProtocolKind] = &[ProtocolKind::VendorSpecific];
pub const CAPABILITIES: &[InstrumentCapability] = &[InstrumentCapability::VendorIdentify];
pub const EXAMPLES: &[ConnectionExample] = &[ConnectionExample {
    transport: TransportKind::Dummy,
    connection: "dummy://default",
    required_feature: "none",
}];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::FunctionGenerator,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    examples: EXAMPLES,
    description: "In-memory dummy instrument",
};
