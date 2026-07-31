#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentRole {
    Oscilloscope,
    FunctionGenerator,
    Multimeter,
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

#[derive(Debug, Clone, Copy)]
pub struct InstrumentSpec {
    pub model: &'static str,
    pub role: InstrumentRole,
    pub transports: &'static [TransportKind],
    pub description: &'static str,
}

pub const KNOWN_INSTRUMENTS: &[InstrumentSpec] = &[
    crate::rigol::dho5108::SPEC,
    crate::nf::wf1946b::SPEC,
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
