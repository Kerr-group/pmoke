#[path = "keithley2010.rs"]
mod driver;

use crate::registry::{InstrumentRole, InstrumentSpec, TransportKind};

pub use driver::Keithley2010;

pub const MODEL: &str = "Keithley2010";
pub const TRANSPORTS: &[TransportKind] = &[
    TransportKind::Gpib,
    TransportKind::PrologixTcp,
    TransportKind::PrologixSerial,
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::Multimeter,
    transports: TRANSPORTS,
    description: "Keithley 2010 multimeter",
};
