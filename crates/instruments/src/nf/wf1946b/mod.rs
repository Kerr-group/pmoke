#[path = "wf1946b.rs"]
mod driver;

use crate::registry::{InstrumentRole, InstrumentSpec, TransportKind};

pub use driver::WF1946B;

pub const MODEL: &str = "WF1946B";
pub const TRANSPORTS: &[TransportKind] = &[
    TransportKind::Gpib,
    TransportKind::PrologixTcp,
    TransportKind::PrologixSerial,
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::FunctionGenerator,
    transports: TRANSPORTS,
    description: "NF WF1946B function generator",
};
