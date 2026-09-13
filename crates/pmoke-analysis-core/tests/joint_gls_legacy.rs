//! FR-052 legacy golden: the frozen boxcar outputs were produced by an
//! independent Python reimplementation
//! (scripts/generate_joint_gls_fixtures.py), not by this crate. Any change
//! in legacy grid, columns, or numerical behavior fails here.

use pmoke_analysis_core::{BoxcarLegacySettings, analyze_boxcar_legacy};
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    schema_version: u32,
    cases: Vec<GoldenCase>,
}

#[derive(Deserialize)]
struct GoldenCase {
    name: String,
    params: GoldenParams,
    signal: Vec<f64>,
    times: Vec<f64>,
    x: Vec<f64>,
    y: Vec<f64>,
    meta: GoldenMeta,
}

#[derive(Deserialize)]
struct GoldenParams {
    f_ref: f64,
    dt: f64,
    t0: f64,
    phase: f64,
    cycles: f64,
    stride: usize,
    harmonic: usize,
    samples: usize,
}

#[derive(Deserialize)]
struct GoldenMeta {
    input_samples: usize,
    output_samples: usize,
    first_input_index: usize,
    last_input_index: usize,
}

#[test]
fn legacy_boxcar_matches_independent_golden() {
    let golden: Golden =
        serde_json::from_str(include_str!("fixtures/joint-gls/legacy-golden.json")).unwrap();
    assert_eq!(golden.schema_version, 1);
    assert!(!golden.cases.is_empty());
    for case in &golden.cases {
        let params = &case.params;
        assert_eq!(case.signal.len(), params.samples, "{}", case.name);
        let settings = BoxcarLegacySettings {
            start_time_s: params.t0,
            sample_interval_s: params.dt,
            reference_frequency_hz: params.f_ref,
            reference_phase_rad: params.phase,
            half_window_cycles: params.cycles,
            stride_samples: params.stride,
            harmonic: params.harmonic,
        };
        let output = analyze_boxcar_legacy(&case.signal, settings).unwrap();
        assert_eq!(output.metadata.input_samples, case.meta.input_samples);
        assert_eq!(output.metadata.output_samples, case.meta.output_samples);
        assert_eq!(
            output.metadata.first_input_index,
            case.meta.first_input_index
        );
        assert_eq!(output.metadata.last_input_index, case.meta.last_input_index);
        assert_eq!(output.x.len(), case.x.len(), "{}", case.name);
        // Python reimplementation uses different summation order and libm
        // paths; 1e-9 absolute plus relative covers platform float drift
        // while remaining far below any scientific tolerance.
        for (label, got, want) in [
            ("time", &output.time_s, &case.times),
            ("x", &output.x, &case.x),
            ("y", &output.y, &case.y),
        ] {
            assert_eq!(got.len(), want.len(), "{} {label}", case.name);
            for (index, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                assert!(
                    (g - w).abs() <= 1.0e-9 * (1.0 + w.abs()),
                    "{} {label}[{index}]: {g} vs {w}",
                    case.name
                );
            }
        }
    }
}
