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
//! below pin the property permanently and freeze the outputs against the
//! committed fixture to bounded tolerance (machine-local bit-identity of
//! the reuse itself is pinned exactly by the combined test; see below).

use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointScratch,
    JointSolverTolerances, NoiseMode, NoiseModel, PreparedNoisePlan, design_matrix, estimate_joint,
    estimate_joint_with_scratch, solve_direct_and_covariance, whiten,
};
use serde::Deserialize;
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
/// window): rank is pinned exactly; residual, beta, covariances, and
/// condition are pinned by bounded relative tolerance (1e-12) against the
/// committed fixture (`fixtures/joint-gls/solve-temp-reuse-golden.json`).
///
/// Why tolerance instead of bit-exact goldens: the estimator pipeline
/// (fixture/design trig through a least-squares solve) reproduces
/// bit-exactly on one machine but drifts a few ulps across machines.
/// Three hosts — the PR author's M4 Pro, a second M4, and ubuntu x86-64
/// CI — produced three `residual_rms` values spread over 5 ulps with
/// matching drift in the beta/covariance hashes, while every
/// same-machine bit-identity check passed on all three. The drift sources
/// (system libm trig, codegen/FMA contraction) are outside this crate's
/// control, and every other golden in this suite already uses bounded
/// tolerance for the same reason (`joint_gls_legacy.rs`: "covers platform
/// float drift"). 1e-12 keeps ~1000x headroom over the observed ~1e-15
/// spread while still freezing behavior: any algorithmic change shifting
/// outputs beyond 1e-12 fails on every platform. Machine-local
/// bit-identity of the reuse itself stays pinned exactly by
/// `solve_reuse_combined_matches_sequential` below.
///
/// Committed fixture provenance: per-element `estimate_joint` outputs
/// recorded locally (see the fixture's `generator` field); serde_json
/// round-trips f64 exactly, so the reference values are bit-exact and
/// only the comparison is bounded.
#[derive(Deserialize)]
struct SolveReuseGolden {
    schema_version: u32,
    cases: Vec<SolveReuseGoldenCase>,
}

#[derive(Deserialize)]
struct SolveReuseGoldenCase {
    name: String,
    rank: usize,
    residual_rms: f64,
    beta: Vec<f64>,
    covariance_beta: Vec<Vec<f64>>,
    covariance_xy: Vec<Vec<f64>>,
    condition: f64,
}

/// Bounded-closeness assert following the suite convention
/// (`joint_gls_fixtures.rs::close`): relative tolerance with a unit floor
/// so near-zero entries still carry a finite absolute bound.
fn assert_close(got: f64, want: f64, tol: f64, context: &str) {
    assert!(
        (got - want).abs() <= tol * (1.0 + want.abs()),
        "{context}: {got} vs {want}"
    );
}

fn assert_matrix_close(got: &[Vec<f64>], want: &[Vec<f64>], tol: f64, label: &str) {
    assert_eq!(got.len(), want.len(), "{label}: row count");
    for (row, (got_row, want_row)) in got.iter().zip(want.iter()).enumerate() {
        assert_eq!(
            got_row.len(),
            want_row.len(),
            "{label}[{row}]: column count"
        );
        for (col, (got, want)) in got_row.iter().zip(want_row.iter()).enumerate() {
            assert_close(*got, *want, tol, &format!("{label}[{row}][{col}]"));
        }
    }
}

#[test]
fn solve_reuse_outputs_golden() {
    const TOL: f64 = 1e-12;
    let golden: SolveReuseGolden = serde_json::from_str(include_str!(
        "fixtures/joint-gls/solve-temp-reuse-golden.json"
    ))
    .unwrap();
    assert_eq!(golden.schema_version, 1);
    assert_eq!(golden.cases.len(), 2);
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    for expect in &golden.cases {
        let noise = match expect.name.as_str() {
            "identity" => identity_noise(),
            "phase_correlated" => correlated_noise(true),
            other => panic!("unknown solve-temp-reuse golden case: {other}"),
        };
        let settings = JointHarmonicSettings {
            model: model.clone(),
            noise,
            tolerances,
        };
        let (times, signal) = window_fixture(0.0);
        let estimate =
            estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
        println!(
            "{}: rank={} residual_bits={:016x} beta_xor={:016x} \
             cov_beta_xor={:016x} cov_xy_xor={:016x} condition={:.17e}",
            expect.name,
            estimate.rank,
            estimate.residual_rms.to_bits(),
            bit_xor(&estimate.beta),
            bit_xor(&estimate.covariance_beta.concat()),
            bit_xor(&estimate.covariance_xy.concat()),
            estimate.condition,
        );
        assert_eq!(estimate.rank, expect.rank, "{}: rank", expect.name);
        assert_close(
            estimate.residual_rms,
            expect.residual_rms,
            TOL,
            &format!("{}: residual_rms", expect.name),
        );
        assert_eq!(
            estimate.beta.len(),
            expect.beta.len(),
            "{}: beta length",
            expect.name
        );
        for (index, (got, want)) in estimate.beta.iter().zip(expect.beta.iter()).enumerate() {
            assert_close(*got, *want, TOL, &format!("{}: beta[{index}]", expect.name));
        }
        assert_matrix_close(
            &estimate.covariance_beta,
            &expect.covariance_beta,
            TOL,
            &format!("{}: covariance_beta", expect.name),
        );
        assert_matrix_close(
            &estimate.covariance_xy,
            &expect.covariance_xy,
            TOL,
            &format!("{}: covariance_xy", expect.name),
        );
        let rel = (estimate.condition - expect.condition).abs() / expect.condition.abs();
        assert!(rel <= TOL, "{}: condition drifted {rel:e}", expect.name);
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
