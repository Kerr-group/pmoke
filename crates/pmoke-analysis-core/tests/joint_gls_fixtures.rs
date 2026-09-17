//! WP-1 fixture evidence: public joint-GLS fixtures are intact and
//! self-consistent (AT-001..006/010 inputs). Hash verification of the frozen
//! files lives in scripts/test_generate_joint_gls_fixtures.py; these tests
//! consume the fixtures through the Rust numerical stack.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::f64::consts::TAU;

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    license: String,
    files: BTreeMap<String, ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestFile {
    sha256: String,
    bytes: u64,
}

#[derive(Deserialize)]
struct Conventions {
    schema_version: u32,
    cases: Vec<ConventionCase>,
}

#[derive(Deserialize)]
struct ConventionCase {
    name: String,
    f_ref: f64,
    dt: f64,
    t_start: f64,
    samples: usize,
    phase_rad: f64,
    dc: f64,
    fit_harmonics: Vec<usize>,
    output_harmonics: Vec<usize>,
    coeffs: BTreeMap<String, [f64; 2]>,
    signal: Vec<f64>,
    expected_beta: Vec<f64>,
    expected_xy: BTreeMap<String, Xy>,
}

#[derive(Deserialize)]
struct Xy {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct Noise {
    schema_version: u32,
    families: BTreeMap<String, NoiseFamily>,
}

#[derive(Deserialize)]
struct NoiseFamily {
    samples: usize,
    values: Vec<f64>,
    sample_mean: f64,
    sample_var: f64,
    phases: Option<Vec<f64>>,
    variances: Option<Vec<f64>>,
    variance_mean: Option<f64>,
}

fn close(a: f64, b: f64, tolerance: f64, context: &str) {
    assert!(
        (a - b).abs() <= tolerance * (1.0 + b.abs()),
        "{context}: {a} vs {b}"
    );
}

#[test]
fn manifest_lists_expected_files() {
    let manifest: Manifest =
        serde_json::from_str(include_str!("fixtures/joint-gls/manifest.json")).unwrap();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.license, "CC0-1.0");
    for name in [
        "geometry.json",
        "conventions.json",
        "noise.json",
        "legacy-golden.json",
    ] {
        let entry = manifest
            .files
            .get(name)
            .unwrap_or_else(|| panic!("{name} listed"));
        assert_eq!(entry.sha256.len(), 64, "{name}");
        assert!(entry.bytes > 0, "{name}");
    }
}

#[test]
fn convention_signals_match_closed_form_mixture() {
    let payload: Conventions =
        serde_json::from_str(include_str!("fixtures/joint-gls/conventions.json")).unwrap();
    assert_eq!(payload.schema_version, 1);
    assert!(!payload.cases.is_empty());
    for case in &payload.cases {
        assert_eq!(case.signal.len(), case.samples, "{}", case.name);
        assert_eq!(
            case.expected_beta.len(),
            1 + 2 * case.fit_harmonics.len(),
            "{}",
            case.name
        );
        for (index, stored) in case.signal.iter().enumerate() {
            let time = case.t_start + index as f64 * case.dt;
            let phi = TAU * case.f_ref * time - case.phase_rad;
            let mut expected = case.dc;
            for (key, pair) in &case.coeffs {
                let harmonic: usize = key.parse().unwrap();
                expected += pair[0] * (harmonic as f64 * phi).cos()
                    + pair[1] * (harmonic as f64 * phi).sin();
            }
            close(
                *stored,
                expected,
                1.0e-9,
                &format!("{}[{index}]", case.name),
            );
        }
        // Beta order [DC, a1, b1, ...] and peak-amplitude XY mapping.
        assert_eq!(case.expected_beta[0], case.dc, "{}", case.name);
        for key in case.output_harmonics.iter().map(usize::to_string) {
            let position = case
                .fit_harmonics
                .iter()
                .position(|h| h.to_string() == key)
                .unwrap();
            let (a, b) = (
                case.expected_beta[1 + 2 * position],
                case.expected_beta[2 + 2 * position],
            );
            let xy = case.expected_xy.get(&key).unwrap();
            close(xy.x, b, 1.0e-15, &format!("{} X{key}", case.name));
            close(xy.y, a, 1.0e-15, &format!("{} Y{key}", case.name));
        }
    }
}

#[test]
fn noise_families_match_recorded_moments() {
    let payload: Noise =
        serde_json::from_str(include_str!("fixtures/joint-gls/noise.json")).unwrap();
    assert_eq!(payload.schema_version, 1);
    assert!(!payload.families.is_empty());
    for (name, family) in &payload.families {
        assert_eq!(family.values.len(), family.samples, "{name}");
        assert!(family.values.iter().all(|v| v.is_finite()), "{name}");
        let mean = family.values.iter().sum::<f64>() / family.values.len() as f64;
        let var = family
            .values
            .iter()
            .map(|v| (v - mean).powi(2))
            .sum::<f64>()
            / family.values.len() as f64;
        close(mean, family.sample_mean, 1.0e-12, name);
        close(var, family.sample_var, 1.0e-12, name);
        // Phase-diagonal families additionally carry per-sample truth used
        // to build the exact known covariance without RNG replay.
        if let (Some(phases), Some(variances), Some(variance_mean)) =
            (&family.phases, &family.variances, family.variance_mean)
        {
            use std::f64::consts::TAU;
            assert_eq!(phases.len(), family.samples, "{name}");
            assert_eq!(variances.len(), family.samples, "{name}");
            assert!(
                phases.iter().all(|p| *p >= 0.0 && *p < TAU)
                    && variances.iter().all(|v| v.is_finite() && *v > 0.0),
                "{name}"
            );
            let mean_var = variances.iter().sum::<f64>() / variances.len() as f64;
            close(mean_var, variance_mean, 1.0e-12, name);
        }
    }
}
