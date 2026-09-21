//! P3 solve-path matrix reuse: the SVD rank/condition diagnostic runs on the
//! thin R factor instead of cloning the `N x P` whitened design a second
//! time. Per-window `N x P` traffic drops from four matrices (SVD clone,
//! SVD U, QR clone, thin Q) to two (QR clone, thin Q); the estimator
//! outputs (beta, residual, covariances, rank) are bit-identical to the
//! pre-reuse path and the reported condition agrees to ~1e-15 relative.
//!
//! Base-vs-patched field comparison on the N=2003/P=25 reference geometry
//! (all four noise modes, estimate path): rank, residual bits, beta hash,
//! and covariance hashes identical on every window; condition differs by
//! at most a few ulps. That comparison is recorded in the P3 PR; the tests
//! below pin the property and the outputs permanently.

use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointScratch,
    JointSolverTolerances, NoiseMode, NoiseModel, PreparedNoisePlan, design_matrix, estimate_joint,
    estimate_joint_with_scratch, solve_direct_and_covariance, whiten,
};
use std::time::Instant;

const N: usize = 97;
const F_REF: f64 = 1.17e6;
const DT: f64 = 17.8e-9;
const PHASE: f64 = 0.3;

fn model() -> HarmonicSignalModel {
    HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    }
}

fn identity_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::Identity,
        reference_variance_v2: 0.01,
        variance_bins: None,
        correlation: None,
    }
}

fn diagonal_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::PhaseDiagonal,
        reference_variance_v2: 0.01,
        variance_bins: Some(vec![1.0, 1.1, 0.9, 1.2, 1.0, 1.05, 0.95, 1.15]),
        correlation: None,
    }
}

fn correlated_noise(phase_bins: bool) -> NoiseModel {
    NoiseModel {
        mode: if phase_bins {
            NoiseMode::PhaseCorrelated
        } else {
            NoiseMode::StationaryCorrelated
        },
        reference_variance_v2: 0.01,
        variance_bins: phase_bins.then(|| vec![1.0, 1.1, 0.9, 1.2, 1.0, 1.05, 0.95, 1.15]),
        correlation: Some(CorrelationKernel {
            lags: vec![1.0, 0.5],
            lag_step_s: DT,
        }),
    }
}

/// Deterministic tone + fixed-pattern pseudo-noise starting at `t_start`.
fn window_fixture(start: f64) -> (Vec<f64>, Vec<f64>) {
    let times: Vec<f64> = (0..N).map(|i| start + i as f64 * DT).collect();
    let mut state: u64 = 0x9E3779B97F4A7C15 ^ (start.to_bits().wrapping_mul(0x9E3779B1));
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as f64 / u64::MAX as f64) - 0.5
    };
    let signal: Vec<f64> = times
        .iter()
        .map(|&t| {
            (std::f64::consts::TAU * F_REF * t + 0.3).sin()
                + 0.2 * (std::f64::consts::TAU * 3.0 * F_REF * t).sin()
                + 0.05 * next()
        })
        .collect();
    (times, signal)
}

fn bit_xor(values: &[f64]) -> u64 {
    values
        .iter()
        .fold(0u64, |hash, value| hash ^ value.to_bits())
}

/// The thin-R diagnostic agrees with the design SVD: same rank verdict on
/// every window, singular values and condition within 1e-12 relative.
/// This is the property the P3 reuse relies on; if a future input breaks
/// it, this test fails closed instead of silently changing gates.
#[test]
fn thin_r_diagnostic_matches_design_diagnostic() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let stride = 100;
    let windows = 8;
    for (mode_name, noise) in [
        ("identity", identity_noise()),
        ("phase_diagonal", diagonal_noise()),
        ("stationary_correlated", correlated_noise(false)),
        ("phase_correlated", correlated_noise(true)),
    ] {
        let settings = JointHarmonicSettings {
            model: model.clone(),
            noise: noise.clone(),
            tolerances,
        };
        let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
        assert_eq!(plan.rows(), N);
        for window in 0..windows {
            let start = window as f64 * stride as f64 * DT;
            let (times, signal) = window_fixture(start);
            let design = design_matrix(&times, F_REF, PHASE, &model, sample_rate).unwrap();
            let system =
                whiten(&design, &signal, PHASE, F_REF, &times, &noise, tolerances).unwrap();
            // Pre-reuse reference: SVD straight on the whitened design.
            let sigma_design = system.design.clone().svd(true, false).singular_values;
            // Reused path: thin QR first, SVD on its R factor.
            let upper = system.design.clone().qr().r();
            assert_eq!(
                (upper.nrows(), upper.ncols()),
                (25, 25),
                "{mode_name} window {window}: thin R must be P x P"
            );
            let sigma_r = upper.clone().svd(true, false).singular_values;
            assert_eq!(
                sigma_design.len(),
                sigma_r.len(),
                "{mode_name} window {window}: singular count differs"
            );
            let sigma_max = sigma_design[0];
            let mut rank_design = 0;
            let mut rank_r = 0;
            let mut worst_rel: f64 = 0.0;
            for (a, b) in sigma_design.iter().zip(sigma_r.iter()) {
                let rel = (*a - *b).abs() / a.abs().max(1e-300);
                worst_rel = worst_rel.max(rel);
                if *a > tolerances.rank_tol * sigma_max {
                    rank_design += 1;
                }
                if *b > tolerances.rank_tol * sigma_r[0] {
                    rank_r += 1;
                }
            }
            assert!(
                worst_rel <= 1e-12,
                "{mode_name} window {window}: sigma(R) drifted {worst_rel:e} from sigma(A)"
            );
            assert_eq!(
                rank_design, rank_r,
                "{mode_name} window {window}: rank verdict differs ({rank_design} vs {rank_r})"
            );
            let condition_design = sigma_design[0] / sigma_design[sigma_design.len() - 1];
            let condition_r = sigma_r[0] / sigma_r[sigma_r.len() - 1];
            let condition_rel =
                (condition_design - condition_r).abs() / condition_design.abs().max(1e-300);
            assert!(
                condition_rel <= 1e-12,
                "{mode_name} window {window}: condition drifted {condition_rel:e}"
            );
            // The estimate entry point must agree with the design rank too.
            let estimate =
                estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
            assert_eq!(
                estimate.rank, rank_design,
                "{mode_name} window {window}: estimate rank differs"
            );
        }
    }
}

/// Golden outputs on two fixtures (identity + phase-correlated, first
/// window): estimator fields are pinned bit-for-bit, condition by 1e-12.
/// Base-vs-patched comparison showed these fields bit-identical (condition
/// within a few ulps); the goldens below freeze that behavior.
#[test]
fn solve_reuse_outputs_golden() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    // Frozen goldens (measured post-change; base-vs-patched field comparison
    // on the N=2003 reference geometry showed these estimator fields
    // bit-identical, condition within a few ulps). Fields: rank,
    // residual_bits, beta_xor, cov_beta_xor, cov_xy_xor, condition
    // (closeness).
    struct Golden {
        rank: usize,
        residual_bits: u64,
        beta_xor: u64,
        cov_beta_xor: u64,
        cov_xy_xor: u64,
        condition: f64,
    }
    let cases: Vec<(&str, NoiseModel, Golden)> = vec![
        (
            "identity",
            identity_noise(),
            Golden {
                rank: 25,
                residual_bits: 0x3fbfddcb9c69a01b,
                beta_xor: 0x3eeebd7a7d739461,
                cov_beta_xor: 0x3f1a84ac3f55d2ec,
                cov_xy_xor: 0x00004778d4085690,
                condition: 1.424_504_738_966_122_7,
            },
        ),
        (
            "phase_correlated",
            correlated_noise(true),
            Golden {
                rank: 25,
                residual_bits: 0x3fa9c86d6bb91882,
                beta_xor: 0x3f66b7708911e359,
                cov_beta_xor: 0x3f965ea966cbb00a,
                cov_xy_xor: 0x00033402550cee54,
                condition: 1.502_340_373_109_660_3,
            },
        ),
    ];
    for (mode_name, noise, golden) in cases {
        let settings = JointHarmonicSettings {
            model: model.clone(),
            noise: noise.clone(),
            tolerances,
        };
        let (times, signal) = window_fixture(0.0);
        let estimate =
            estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
        println!(
            "{mode_name}: rank={} residual_bits={:016x} beta_xor={:016x} \
             cov_beta_xor={:016x} cov_xy_xor={:016x} condition={:.17e}",
            estimate.rank,
            estimate.residual_rms.to_bits(),
            bit_xor(&estimate.beta),
            bit_xor(&estimate.covariance_beta.concat()),
            bit_xor(&estimate.covariance_xy.concat()),
            estimate.condition,
        );
        assert_eq!(estimate.rank, golden.rank, "{mode_name}: rank");
        assert_eq!(
            estimate.residual_rms.to_bits(),
            golden.residual_bits,
            "{mode_name}: residual_rms"
        );
        assert_eq!(
            bit_xor(&estimate.beta),
            golden.beta_xor,
            "{mode_name}: beta"
        );
        assert_eq!(
            bit_xor(&estimate.covariance_beta.concat()),
            golden.cov_beta_xor,
            "{mode_name}: covariance_beta"
        );
        assert_eq!(
            bit_xor(&estimate.covariance_xy.concat()),
            golden.cov_xy_xor,
            "{mode_name}: covariance_xy"
        );
        let rel = (estimate.condition - golden.condition).abs() / golden.condition.abs();
        assert!(rel <= 1e-12, "{mode_name}: condition drifted {rel:e}");
    }
}

/// Thread-local allocation counting: only the measuring thread's
/// allocations inside the flagged window are counted, so concurrently
/// running tests cannot pollute the bound.
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
    }

    pub struct ThreadLocalCounting;

    pub fn set_counting(active: bool) {
        COUNTING.with(|flag| flag.set(active));
    }

    unsafe impl GlobalAlloc for ThreadLocalCounting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let active = COUNTING.try_with(|flag| flag.get()).unwrap_or(false);
            if active {
                ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
            }
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }
}

#[global_allocator]
static GLOBAL_COUNTING: counting::ThreadLocalCounting = counting::ThreadLocalCounting;

/// Allocated-bytes regression gate on the N=2003/P=25 reference geometry
/// (identity, scratch pipeline path): one window must allocate well under
/// the pre-reuse budget. Pre-reuse traffic is ~2.09 MB/window (SVD clone +
/// SVD U + QR clone + thin Q, four `N x P` units plus design/whiten/output
/// traffic); post-reuse it is ~1.29 MB (two `N x P` units). The 1.7 MB
/// bound sits ~30% above the post-reuse level and ~20% below pre-reuse, so
/// reintroducing the duplicate clone fails while platform allocator noise
/// (counted as requested bytes, layout-exact) cannot trip it.
#[test]
fn solve_reuse_temp_bytes_bound() {
    const BIG_N: usize = 2003;
    const BIG_F_REF: f64 = 10e3;
    const BIG_DT: f64 = 1e-7;
    const BIG_PHASE: f64 = 0.3;
    const BIG_STRIDE: usize = 100;
    let sample_rate = 1.0 / BIG_DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let noise = identity_noise();
    let plan = PreparedNoisePlan::prepare(&noise, BIG_N, tolerances).unwrap();
    let windows = 20usize;
    let fixtures: Vec<(Vec<f64>, Vec<f64>)> = (0..windows)
        .map(|window| {
            let start = window as f64 * BIG_STRIDE as f64 * BIG_DT;
            let times: Vec<f64> = (0..BIG_N).map(|i| start + i as f64 * BIG_DT).collect();
            let mut state: u64 = 0x9E3779B97F4A7C15 ^ (start.to_bits().wrapping_mul(0x9E3779B1));
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state as f64 / u64::MAX as f64) - 0.5
            };
            let signal: Vec<f64> = times
                .iter()
                .map(|&t| {
                    (std::f64::consts::TAU * BIG_F_REF * t + 0.3).sin()
                        + 0.2 * (std::f64::consts::TAU * 3.0 * BIG_F_REF * t).sin()
                        + 0.05 * next()
                })
                .collect();
            (times, signal)
        })
        .collect();
    let mut scratch = JointScratch::new();
    // Warm up outside the counted window (buffer growth, caches).
    for (times, signal) in fixtures.iter().take(2) {
        let _ = estimate_joint_with_scratch(
            times,
            signal,
            BIG_F_REF,
            BIG_PHASE,
            sample_rate,
            &model,
            tolerances,
            &plan,
            &mut scratch,
        )
        .unwrap();
    }
    counting::ALLOCATED.store(0, std::sync::atomic::Ordering::Relaxed);
    counting::set_counting(true);
    for (times, signal) in &fixtures {
        let _ = estimate_joint_with_scratch(
            times,
            signal,
            BIG_F_REF,
            BIG_PHASE,
            sample_rate,
            &model,
            tolerances,
            &plan,
            &mut scratch,
        )
        .unwrap();
    }
    counting::set_counting(false);
    let bytes_per_window =
        counting::ALLOCATED.load(std::sync::atomic::Ordering::Relaxed) as f64 / windows as f64;
    println!("solve-reuse temp bytes/window: {bytes_per_window:.0} (bound 1_700_000)");
    assert!(
        bytes_per_window <= 1_700_000.0,
        "solve temp regression: {bytes_per_window:.0} bytes/window exceeds 1_700_000"
    );
}

/// Generous-bound wall gate on the same reference shape: 60 scratch windows
/// must complete far inside 60 s. The sharp temp gate above trips on a
/// reintroduced clone; this one only trips on a pathological regression.
#[test]
fn solve_reuse_identity_wall_bound() {
    const BIG_N: usize = 2003;
    const BIG_F_REF: f64 = 10e3;
    const BIG_DT: f64 = 1e-7;
    const BIG_PHASE: f64 = 0.3;
    const BIG_STRIDE: usize = 100;
    let sample_rate = 1.0 / BIG_DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let noise = identity_noise();
    let plan = PreparedNoisePlan::prepare(&noise, BIG_N, tolerances).unwrap();
    let windows = 60usize;
    let fixtures: Vec<(Vec<f64>, Vec<f64>)> = (0..windows)
        .map(|window| {
            let start = window as f64 * BIG_STRIDE as f64 * BIG_DT;
            let times: Vec<f64> = (0..BIG_N).map(|i| start + i as f64 * BIG_DT).collect();
            let mut state: u64 = 0x9E3779B97F4A7C15 ^ (start.to_bits().wrapping_mul(0x9E3779B1));
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state as f64 / u64::MAX as f64) - 0.5
            };
            let signal: Vec<f64> = times
                .iter()
                .map(|&t| {
                    (std::f64::consts::TAU * BIG_F_REF * t + 0.3).sin()
                        + 0.2 * (std::f64::consts::TAU * 3.0 * BIG_F_REF * t).sin()
                        + 0.05 * next()
                })
                .collect();
            (times, signal)
        })
        .collect();
    let mut scratch = JointScratch::new();
    let t0 = Instant::now();
    for (times, signal) in &fixtures {
        let _ = estimate_joint_with_scratch(
            times,
            signal,
            BIG_F_REF,
            BIG_PHASE,
            sample_rate,
            &model,
            tolerances,
            &plan,
            &mut scratch,
        )
        .unwrap();
    }
    let elapsed = t0.elapsed();
    let per_window_us = elapsed.as_secs_f64() * 1e6 / windows as f64;
    println!(
        "solve-reuse identity: {windows} windows in {elapsed:.2?} ({per_window_us:.1}us/window)"
    );
    assert!(
        elapsed.as_secs() < 60,
        "identity wall regression: {windows} windows took {elapsed:?} (bound 60s)"
    );
}

/// The shared-R combined entry point still reproduces the sequential pair
/// bit-for-bit on the reused factor (same factor object, same order).
#[test]
fn solve_reuse_combined_matches_sequential() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    for (mode_name, noise) in [
        ("identity", identity_noise()),
        ("phase_diagonal", diagonal_noise()),
        ("stationary_correlated", correlated_noise(false)),
        ("phase_correlated", correlated_noise(true)),
    ] {
        let (times, signal) = window_fixture(0.0);
        let design = design_matrix(&times, F_REF, PHASE, &model, sample_rate).unwrap();
        let system = whiten(&design, &signal, PHASE, F_REF, &times, &noise, tolerances).unwrap();
        let (beta, rms, rank, condition) =
            pmoke_analysis_core::solve_direct(&system.design, &system.response, tolerances)
                .unwrap();
        let covariance =
            pmoke_analysis_core::covariance_from_qr(&system.design, system.variance_scale).unwrap();
        let (beta2, rms2, rank2, condition2, covariance2) = solve_direct_and_covariance(
            &system.design,
            &system.response,
            tolerances,
            system.variance_scale,
        )
        .unwrap();
        assert_eq!(beta.as_slice(), beta2.as_slice(), "{mode_name}: beta");
        assert!(rms == rms2, "{mode_name}: rms");
        assert_eq!(rank, rank2, "{mode_name}: rank");
        assert!(condition == condition2, "{mode_name}: condition");
        assert_eq!(covariance, covariance2, "{mode_name}: covariance");
    }
}
