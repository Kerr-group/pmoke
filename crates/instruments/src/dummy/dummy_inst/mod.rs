#[path = "dummy_inst.rs"]
mod driver;

use crate::registry::{InstrumentRole, InstrumentSpec, TransportKind};

pub use driver::{DummyInstrument, DummyResult};

pub const MODEL: &str = "DummyInstrument";
pub const TRANSPORTS: &[TransportKind] = &[TransportKind::Dummy];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::FunctionGenerator,
    transports: TRANSPORTS,
    description: "In-memory dummy instrument",
};
