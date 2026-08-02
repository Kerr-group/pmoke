mod error;
mod kerr;
mod lockin;
mod phase;
mod synthetic;

pub use error::{AnalysisError, Result};
pub use kerr::{HarmonicsKerrOutput, calculate_harmonics_kerr};
pub use lockin::{
    BoxcarLegacyOutput, BoxcarLegacyPairOutput, BoxcarLegacySettings, FiniteSignal, LockinMetadata,
    analyze_boxcar_legacy, analyze_boxcar_legacy_pair, analyze_boxcar_legacy_pair_finite,
    boxcar_response_abs,
};
pub use phase::rotate_phase;
pub use synthetic::{SyntheticSignalSettings, generate_synthetic_signal};

pub const DEFAULT_MAX_DEMO_SAMPLES: usize = 100_000;
pub const MAX_UPLOAD_SAMPLES: usize = 1_000_000;
pub const MAX_UPLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_HARMONIC_POINTS: usize = 1_500_000;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
