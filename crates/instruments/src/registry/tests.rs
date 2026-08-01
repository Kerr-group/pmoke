use super::*;

#[test]
fn dho5108_is_not_registered_for_prologix() {
    assert!(supports_transport("DHO5108", TransportKind::Tcpip));
    assert!(supports_transport("DHO5108", TransportKind::Usbtmc));
    assert!(!supports_transport("DHO5108", TransportKind::PrologixTcp));
    assert!(!supports_transport(
        "DHO5108",
        TransportKind::PrologixSerial
    ));
}

#[test]
fn known_instruments_are_registered_from_device_specs() {
    assert_eq!(
        find_instrument(crate::rigol::dho5108::MODEL).map(|spec| spec.model),
        Some(crate::rigol::dho5108::MODEL)
    );
    assert_eq!(
        find_instrument(crate::nf::wf1946b::MODEL).map(|spec| spec.model),
        Some(crate::nf::wf1946b::MODEL)
    );
    assert_eq!(
        find_instrument(crate::keithley::keithley2010::MODEL).map(|spec| spec.model),
        Some(crate::keithley::keithley2010::MODEL)
    );
    assert_eq!(
        find_instrument(crate::keithley::keithley2000::MODEL).map(|spec| spec.model),
        Some(crate::keithley::keithley2000::MODEL)
    );
    assert_eq!(
        find_instrument(crate::rigol::dho5108::MODEL).map(|spec| spec.role),
        Some(InstrumentRole::Oscilloscope)
    );
    assert_eq!(
        find_instrument(crate::nf::wf1946b::MODEL).map(|spec| spec.role),
        Some(InstrumentRole::FunctionGenerator)
    );
    assert_eq!(
        find_instrument(crate::keithley::keithley2010::MODEL).map(|spec| spec.role),
        Some(InstrumentRole::Multimeter)
    );
    assert_eq!(
        find_instrument(crate::keithley::keithley2000::MODEL).map(|spec| spec.role),
        Some(InstrumentRole::Multimeter)
    );
}

#[test]
fn known_instruments_expose_protocols_capabilities_and_examples() {
    for spec in KNOWN_INSTRUMENTS {
        assert!(
            !spec.transports.is_empty(),
            "{} must declare transports",
            spec.model
        );
        assert!(
            !spec.protocols.is_empty(),
            "{} must declare protocols",
            spec.model
        );
        assert!(
            !spec.capabilities.is_empty(),
            "{} must declare capabilities",
            spec.model
        );
        assert!(
            !spec.examples.is_empty(),
            "{} must provide at least one connection example",
            spec.model
        );
    }
}
