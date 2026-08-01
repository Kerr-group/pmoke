#[path = "keithley2000.rs"]
mod driver;

use crate::registry::{
    ConnectionExample, InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind,
    TransportKind,
};

pub use driver::Keithley2000;

pub const MODEL: &str = "Keithley2000";
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
pub const EXAMPLES: &[ConnectionExample] = &[
    ConnectionExample {
        transport: TransportKind::Gpib,
        connection: "gpib://0/17",
        required_feature: "hw-gpib",
    },
    ConnectionExample {
        transport: TransportKind::PrologixTcp,
        connection: "prologix-tcp://10.249.11.17:1234?addr=17",
        required_feature: "hw-prologix-tcp",
    },
    ConnectionExample {
        transport: TransportKind::PrologixSerial,
        connection: "prologix-serial:///dev/cu.usbserial-XXXX?addr=17",
        required_feature: "hw-prologix-serial",
    },
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::Multimeter,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    examples: EXAMPLES,
    description: "Keithley 2000 multimeter",
};
