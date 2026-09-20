pub mod calibration;
pub mod error;
pub mod joint;
mod lockin;
mod moke;
pub mod moke_uncertainty;
mod phase;
mod synthetic;

pub use calibration::{
    AcquisitionMeta, AdequacyGroup, AdequacyPolicy, AdequacyReport, ApplicabilityError,
    ApplicabilityReport, ApplicabilityRequest, ArtifactRequest, ArtifactWithHash, BlockPlan,
    BlockPlanRequest, BuilderInfo, CALIBRATION_ALGORITHM_VERSION,
    CALIBRATION_ARTIFACT_SCHEMA_VERSION, CALIBRATION_PHASE_CONVENTION, CORRELATION_TAPER_ID,
    CalSample, CalibrationArtifact, CalibrationRole, CapabilityRecord, CorrelationOutput,
    CorrelationRecipe, CorrelationTable, DEFAULT_BLOCK_LEN, DEFAULT_CORRELATION_ETA,
    DEFAULT_DT_REL_TOL, DEFAULT_FLOOR_RATIO, DEFAULT_FREQ_REL_TOL, DEFAULT_MAX_LAG,
    DEFAULT_MIN_CYCLES_PER_BIN, DEFAULT_MIN_CYCLES_PER_BLOCK, DEFAULT_MIN_SAMPLES_PER_BIN,
    DEFAULT_MIN_TRAINING_BLOCKS, DEFAULT_MIN_TRAINING_INTERVALS, DEFAULT_SHRINKAGE_ALPHA,
    FREQUENCY_TOL_COVERAGE_K, FREQUENCY_TOL_FLOOR, HeldoutReport, MAX_CALIBRATION_ARTIFACT_BYTES,
    ModelBinding, NUISANCE_HARMONICS, NUISANCE_PARAMETERS, NuisanceFit, PhaseTable,
    PhaseVarianceOutput, PhaseVarianceRecipe, PlannedBlock, PlannedExclusion, RegularizationRecord,
    RoleInterval, SearchSpace, TrainingRecord, TuningMode, VARIANCE_INTERP_ID, ValidationRecord,
    assemble_samples, build_artifact, effective_frequency_rel_tol, ensure_fixed_tuning,
    estimate_correlation, estimate_phase_variance, fit_nuisance, frequency_rel_tol_bound,
    inspect_applicability, nuisance_design_matrix, plan_blocks, resolve_build_frequency_tol,
    scs_adequacy,
};
pub use error::{AnalysisError, Result};
pub use joint::{
    CORRELATION_DT_REL_TOL, CorrelationKernel, DEFAULT_MAX_CONDITION, DEFAULT_MAX_JITTER,
    DEFAULT_MAX_NOISE_CONDITION, DEFAULT_RANK_TOL, HarmonicSignalModel, JointEstimate,
    JointHarmonicSettings, JointSolverTolerances, LAG_ZERO_TOL, MAX_MODEL_PARAMETERS,
    MAX_WINDOW_SAMPLES, NoiseMode, NoiseModel, PreparedNoisePlan, TIMEBASE_RELATIVE_TOLERANCE,
    WhitenedSystem, cholesky_factor, covariance_from_qr, design_matrix, estimate_joint,
    estimate_joint_with_plan, forward_substitute, interpolate_variance, map_covariance_to_xy,
    map_to_xy, pack_upper_triangle, rotate_xy_covariance, solve_direct, toeplitz_from_lags,
    validate_correlation_kernel, validate_noise_model, validate_signal_model, validate_timebase,
    whiten,
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
