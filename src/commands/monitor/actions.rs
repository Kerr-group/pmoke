use crate::config::{self, ConfigLoad, ValidationTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ActionGroup {
    Acquisition,
    Analysis,
    Pipeline,
    Utilities,
}

impl ActionGroup {
    pub(super) const ALL: [Self; 4] = [
        Self::Utilities,
        Self::Acquisition,
        Self::Analysis,
        Self::Pipeline,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Acquisition => "ACQUISITION",
            Self::Analysis => "ANALYSIS",
            Self::Pipeline => "PIPELINE",
            Self::Utilities => "UTILITIES",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkflowEntry {
    Group(ActionGroup),
    Action(MonitorAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MonitorAction {
    Show,
    Reference,
    Sensor,
    Li,
    Phase,
    Kerr,
    Analyze,
    Doctor,
    RawVerify,
    #[cfg(feature = "hw")]
    Single,
    #[cfg(feature = "hw")]
    Trigger,
    #[cfg(feature = "hw")]
    Autoshot,
    #[cfg(feature = "hw")]
    Fetch,
    #[cfg(feature = "hw")]
    Screenshot,
    #[cfg(feature = "hw")]
    Automeasure,
    #[cfg(feature = "hw")]
    Process,
    #[cfg(feature = "hw")]
    Auto,
}

impl MonitorAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Show => "Config",
            Self::Reference => "Reference",
            Self::Sensor => "Sensor",
            Self::Li => "Lock-in",
            Self::Phase => "Phase",
            Self::Kerr => "Kerr",
            Self::Analyze => "Analyze",
            Self::Doctor => "Doctor",
            Self::RawVerify => "RAW verify",
            #[cfg(feature = "hw")]
            Self::Single => "Single",
            #[cfg(feature = "hw")]
            Self::Trigger => "Trigger",
            #[cfg(feature = "hw")]
            Self::Autoshot => "Shot",
            #[cfg(feature = "hw")]
            Self::Fetch => "Fetch",
            #[cfg(feature = "hw")]
            Self::Screenshot => "Screenshot",
            #[cfg(feature = "hw")]
            Self::Automeasure => "Measure",
            #[cfg(feature = "hw")]
            Self::Process => "Process",
            #[cfg(feature = "hw")]
            Self::Auto => "Auto",
        }
    }

    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Show => "Print normalized config and diagnostics.",
            Self::Reference => {
                "Fit reference frequency and phase from the configured waveform input."
            }
            Self::Sensor => "Integrate sensor pulse channels.",
            Self::Li => "Run numerical lock-in and write lockin_results.",
            Self::Phase => "Rotate lock-in phase and write lockin_rotated.",
            Self::Kerr => "Calculate Kerr angle from rotated lock-in data.",
            Self::Analyze => "Run reference, sensor, lock-in, phase, and Kerr.",
            Self::Doctor => "Check config, storage, Python, and available instruments.",
            Self::RawVerify => "Verify the RAW manifest, sizes, and channel checksums.",
            #[cfg(feature = "hw")]
            Self::Single => "Set oscilloscope to single acquisition mode.",
            #[cfg(feature = "hw")]
            Self::Trigger => "Send trigger from the function generator.",
            #[cfg(feature = "hw")]
            Self::Autoshot => "Set single mode and send trigger.",
            #[cfg(feature = "hw")]
            Self::Fetch => "Fetch oscilloscope data using the configured output format.",
            #[cfg(feature = "hw")]
            Self::Screenshot => "Save an oscilloscope screenshot.",
            #[cfg(feature = "hw")]
            Self::Automeasure => "Single, trigger, then fetch waveform data.",
            #[cfg(feature = "hw")]
            Self::Process => "Fetch waveform and run the analysis chain.",
            #[cfg(feature = "hw")]
            Self::Auto => "Run the full automatic measurement and analysis.",
        }
    }

    pub(super) fn target(self) -> Option<ValidationTarget> {
        match self {
            Self::Show => None,
            Self::Reference => Some(ValidationTarget::Reference),
            Self::Sensor => Some(ValidationTarget::Sensor),
            Self::Li => Some(ValidationTarget::Li),
            Self::Phase => Some(ValidationTarget::Phase),
            Self::Kerr => Some(ValidationTarget::Kerr),
            Self::Analyze => Some(ValidationTarget::Analyze),
            Self::Doctor | Self::RawVerify => None,
            #[cfg(feature = "hw")]
            Self::Single => Some(ValidationTarget::Single),
            #[cfg(feature = "hw")]
            Self::Trigger => Some(ValidationTarget::Trigger),
            #[cfg(feature = "hw")]
            Self::Autoshot => Some(ValidationTarget::Autoshot),
            #[cfg(feature = "hw")]
            Self::Fetch => Some(ValidationTarget::Fetch),
            #[cfg(feature = "hw")]
            Self::Screenshot => Some(ValidationTarget::Screenshot),
            #[cfg(feature = "hw")]
            Self::Automeasure => Some(ValidationTarget::Automeasure),
            #[cfg(feature = "hw")]
            Self::Process => Some(ValidationTarget::Process),
            #[cfg(feature = "hw")]
            Self::Auto => Some(ValidationTarget::Auto),
        }
    }

    pub(super) fn command_name(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Reference => "reference",
            Self::Sensor => "sensor",
            Self::Li => "li",
            Self::Phase => "phase",
            Self::Kerr => "kerr",
            Self::Analyze => "analyze",
            Self::Doctor => "doctor",
            Self::RawVerify => "raw verify",
            #[cfg(feature = "hw")]
            Self::Single => "single",
            #[cfg(feature = "hw")]
            Self::Trigger => "trigger",
            #[cfg(feature = "hw")]
            Self::Autoshot => "autoshot",
            #[cfg(feature = "hw")]
            Self::Fetch => "fetch",
            #[cfg(feature = "hw")]
            Self::Screenshot => "screenshot",
            #[cfg(feature = "hw")]
            Self::Automeasure => "automeasure",
            #[cfg(feature = "hw")]
            Self::Process => "process",
            #[cfg(feature = "hw")]
            Self::Auto => "auto",
        }
    }

    pub(super) fn command_args(self) -> &'static [&'static str] {
        match self {
            Self::Show => &["show"],
            Self::Reference => &["reference"],
            Self::Sensor => &["sensor"],
            Self::Li => &["li"],
            Self::Phase => &["phase"],
            Self::Kerr => &["kerr"],
            Self::Analyze => &["analyze"],
            Self::Doctor => &["doctor"],
            Self::RawVerify => &["raw", "verify"],
            #[cfg(feature = "hw")]
            Self::Single => &["single"],
            #[cfg(feature = "hw")]
            Self::Trigger => &["trigger"],
            #[cfg(feature = "hw")]
            Self::Autoshot => &["autoshot"],
            #[cfg(feature = "hw")]
            Self::Fetch => &["fetch"],
            #[cfg(feature = "hw")]
            Self::Screenshot => &["screenshot"],
            #[cfg(feature = "hw")]
            Self::Automeasure => &["automeasure"],
            #[cfg(feature = "hw")]
            Self::Process => &["process"],
            #[cfg(feature = "hw")]
            Self::Auto => &["auto"],
        }
    }

    pub(super) fn group(self) -> ActionGroup {
        match self {
            #[cfg(feature = "hw")]
            Self::Single
            | Self::Trigger
            | Self::Autoshot
            | Self::Fetch
            | Self::Screenshot
            | Self::Automeasure => ActionGroup::Acquisition,
            Self::Reference
            | Self::Sensor
            | Self::Li
            | Self::Phase
            | Self::Kerr
            | Self::Analyze => ActionGroup::Analysis,
            #[cfg(feature = "hw")]
            Self::Process | Self::Auto => ActionGroup::Pipeline,
            Self::Show | Self::Doctor | Self::RawVerify => ActionGroup::Utilities,
        }
    }
}

pub(super) fn monitor_actions() -> Vec<MonitorAction> {
    vec![
        MonitorAction::Show,
        #[cfg(feature = "hw")]
        MonitorAction::Single,
        #[cfg(feature = "hw")]
        MonitorAction::Trigger,
        #[cfg(feature = "hw")]
        MonitorAction::Autoshot,
        #[cfg(feature = "hw")]
        MonitorAction::Fetch,
        #[cfg(feature = "hw")]
        MonitorAction::Screenshot,
        #[cfg(feature = "hw")]
        MonitorAction::Automeasure,
        MonitorAction::Reference,
        MonitorAction::Sensor,
        MonitorAction::Li,
        MonitorAction::Phase,
        MonitorAction::Kerr,
        MonitorAction::Analyze,
        MonitorAction::Doctor,
        MonitorAction::RawVerify,
        #[cfg(feature = "hw")]
        MonitorAction::Process,
        #[cfg(feature = "hw")]
        MonitorAction::Auto,
    ]
}

pub(super) fn action_readiness(action: MonitorAction, load: &ConfigLoad) -> Result<(), String> {
    if matches!(action, MonitorAction::Show | MonitorAction::Doctor) {
        return Ok(());
    }

    let ConfigLoad::Ready { config, .. } = load else {
        return Err("configuration has errors; open Diagnostics".to_string());
    };

    if matches!(action, MonitorAction::RawVerify) {
        let raw = config.paths().acquisition_dir();
        return raw
            .is_dir()
            .then_some(())
            .ok_or_else(|| format!("RAW directory not found: {}", raw.display()));
    }

    let target = action
        .target()
        .ok_or_else(|| "action has no validation target".to_string())?;
    config::validate_for_target(config, target).map_err(|error| format!("{error:#}"))
}

pub(super) fn action_runnable(action: MonitorAction, load: &ConfigLoad) -> bool {
    action_readiness(action, load).is_ok()
}
