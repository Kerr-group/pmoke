#[path = "wf1946b.rs"]
mod driver;

use crate::registry::{
    ConnectionExample, InstrumentCapability, InstrumentRole, InstrumentSpec, ProtocolKind,
    TransportKind,
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
pub const EXAMPLES: &[ConnectionExample] = &[
    ConnectionExample {
        transport: TransportKind::Gpib,
        connection: "gpib://0/11",
        required_feature: "hw-gpib",
    },
    ConnectionExample {
        transport: TransportKind::PrologixTcp,
        connection: "prologix-tcp://192.168.1.50:1234?addr=11",
        required_feature: "hw-prologix-tcp",
    },
    ConnectionExample {
        transport: TransportKind::PrologixSerial,
        connection: "prologix-serial:///dev/cu.usbserial-XXXX?addr=11",
        required_feature: "hw-prologix-serial",
    },
];
pub const SPEC: InstrumentSpec = InstrumentSpec {
    model: MODEL,
    role: InstrumentRole::FunctionGenerator,
    transports: TRANSPORTS,
    protocols: PROTOCOLS,
    capabilities: CAPABILITIES,
    examples: EXAMPLES,
    description: "NF WF1946B function generator",
};
