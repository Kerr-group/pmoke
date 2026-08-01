#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentRole {
    Oscilloscope,
    FunctionGenerator,
    Multimeter,
}

impl InstrumentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oscilloscope => "oscilloscope",
            Self::FunctionGenerator => "function_generator",
            Self::Multimeter => "multimeter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Dummy,
    Gpib,
    Tcpip,
    Usbtmc,
    PrologixTcp,
    PrologixSerial,
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dummy => "dummy",
            Self::Gpib => "gpib",
            Self::Tcpip => "tcpip",
            Self::Usbtmc => "usbtmc",
            Self::PrologixTcp => "prologix_tcp",
            Self::PrologixSerial => "prologix_serial",
        }
    }

    pub fn required_feature(self) -> Option<&'static str> {
        match self {
            Self::Dummy => None,
            Self::Gpib => Some("hw-gpib"),
            Self::Tcpip => Some("hw-core"),
            Self::Usbtmc => Some("hw-gpib"),
            Self::PrologixTcp => Some("hw-prologix-tcp"),
            Self::PrologixSerial => Some("hw-prologix-serial"),
        }
    }

    pub fn feature_note(self) -> Option<&'static str> {
        match self {
            Self::Usbtmc => Some("Windows + NI-VISA"),
            _ => None,
        }
    }

    pub fn connection_template(self) -> &'static str {
        match self {
            Self::Dummy => "dummy://default",
            Self::Gpib => "gpib://0/<addr>",
            Self::Tcpip => "tcp://<host>:<port>",
            Self::Usbtmc => "visa:USB0::...::INSTR",
            Self::PrologixTcp => "prologix-tcp://<host>:1234?addr=<addr>",
            Self::PrologixSerial => "prologix-serial:///dev/cu.usbserial-XXXX?addr=<addr>",
        }
    }

    pub fn diagnostic_capabilities(self) -> &'static [TransportDiagnosticCapability] {
        const NONE: &[TransportDiagnosticCapability] = &[];
        const PROLOGIX: &[TransportDiagnosticCapability] =
            &[TransportDiagnosticCapability::PrologixControllerVersion];

        match self {
            Self::PrologixTcp | Self::PrologixSerial => PROLOGIX,
            Self::Dummy | Self::Gpib | Self::Tcpip | Self::Usbtmc => NONE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolKind {
    Scpi,
    VendorSpecific,
    Raw,
}

impl ProtocolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scpi => "scpi",
            Self::VendorSpecific => "vendor_specific",
            Self::Raw => "raw",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentCapability {
    ScpiIdentify,
    ScpiTrigger,
    ScpiMeasureRead,
    Screenshot,
    WaveformFetch,
    VendorIdentify,
}

impl InstrumentCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScpiIdentify => "scpi_identify",
            Self::ScpiTrigger => "scpi_trigger",
            Self::ScpiMeasureRead => "scpi_measure_read",
            Self::Screenshot => "screenshot",
            Self::WaveformFetch => "waveform_fetch",
            Self::VendorIdentify => "vendor_identify",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportDiagnosticCapability {
    PrologixControllerVersion,
}

impl TransportDiagnosticCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrologixControllerVersion => "prologix_controller_version",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstrumentSpec {
    pub model: &'static str,
    pub role: InstrumentRole,
    pub transports: &'static [TransportKind],
    pub protocols: &'static [ProtocolKind],
    pub capabilities: &'static [InstrumentCapability],
    pub description: &'static str,
}

pub const KNOWN_INSTRUMENTS: &[InstrumentSpec] = &[
    crate::rigol::dho5108::SPEC,
    crate::nf::wf1946b::SPEC,
    crate::keithley::keithley2010::SPEC,
    crate::keithley::keithley2000::SPEC,
    crate::dummy::dummy_inst::SPEC,
];

pub fn find_instrument(model: &str) -> Option<&'static InstrumentSpec> {
    KNOWN_INSTRUMENTS.iter().find(|spec| spec.model == model)
}

pub fn supports_transport(model: &str, transport: TransportKind) -> bool {
    find_instrument(model)
        .map(|spec| spec.transports.contains(&transport))
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "registry/tests.rs"]
mod tests;
