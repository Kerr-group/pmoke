mod error;
pub mod joint;
mod lockin;
mod moke;
mod phase;
mod synthetic;

pub use error::{AnalysisError, Result};
pub use joint::{
    DEFAULT_MAX_CONDITION, DEFAULT_RANK_TOL, HarmonicSignalModel, JointEstimate,
    JointHarmonicSettings, JointSolverTolerances, MAX_MODEL_PARAMETERS, NoiseMode, NoiseModel,
    covariance_from_qr, design_matrix, estimate_joint, interpolate_variance, map_covariance_to_xy,
    map_to_xy, pack_upper_triangle, rotate_xy_covariance, solve_direct, validate_noise_model,
    validate_signal_model, whiten,
};
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
