mod error;
mod lockin;
mod moke;
mod phase;
mod synthetic;

pub use error::{AnalysisError, Result};
pub use lockin::{
    BoxcarLegacyOutput, BoxcarLegacyPairOutput, BoxcarLegacySettings, BoxcarMeanOutput,
    BoxcarMeanSettings, FiniteSignal, LockinMetadata, analyze_boxcar_legacy,
    analyze_boxcar_legacy_pair, analyze_boxcar_legacy_pair_finite, boxcar_mean,
    boxcar_response_abs,
};
pub use moke::{HarmonicsMokeOutput, calculate_harmonics_moke, calculate_harmonics_vm};
pub use phase::rotate_phase;
pub use synthetic::{SyntheticSignalSettings, generate_synthetic_signal};

pub const DEFAULT_MAX_DEMO_SAMPLES: usize = 100_000;
pub const MAX_UPLOAD_SAMPLES: usize = 1_000_000;
pub const MAX_UPLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_HARMONIC_POINTS: usize = 1_500_000;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
