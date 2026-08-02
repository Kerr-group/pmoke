use pmoke_analysis_core::{
    BoxcarLegacySettings, SyntheticSignalSettings, analyze_boxcar_legacy, calculate_harmonics_kerr,
    generate_synthetic_signal, rotate_phase,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    parameters: Parameters,
    tolerances: Tolerances,
    expected: Expected,
}

#[derive(Deserialize)]
struct Parameters {
    samples: usize,
    sample_rate_hz: f64,
    reference_frequency_hz: f64,
    amplitude: f64,
    phase_rad: f64,
    noise_rms: f64,
    kerr_angle_rad: f64,
    seed: u64,
    half_window_cycles: f64,
    stride_samples: usize,
    rotation_rad: f64,
    kerr_factor: f64,
}

#[derive(Deserialize)]
struct Tolerances {
    lockin_phase_abs: f64,
    lockin_phase_rel: f64,
    kerr_abs: f64,
    kerr_rel: f64,
}

#[derive(Deserialize)]
struct Expected {
    output_samples: usize,
    output_rate_hz: f64,
    first_input_index: usize,
    last_input_index: usize,
    sample_index: usize,
    harmonics: Vec<ExpectedHarmonic>,
    modulation_depth: f64,
    kerr_rad: f64,
}

#[derive(Deserialize)]
struct ExpectedHarmonic {
    harmonic: usize,
    time_s: f64,
    x: f64,
    y: f64,
    in_phase: f64,
    out_of_phase: f64,
}

#[test]
fn shared_pipeline_recovers_the_reference_kerr_fixture() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/m4-synthetic-reference.json")).unwrap();
    let parameters = &fixture.parameters;
    let signal = generate_synthetic_signal(SyntheticSignalSettings {
        samples: parameters.samples,
        sample_rate_hz: parameters.sample_rate_hz,
        reference_frequency_hz: parameters.reference_frequency_hz,
        amplitude: parameters.amplitude,
        phase_rad: parameters.phase_rad,
        noise_rms: parameters.noise_rms,
        kerr_angle_rad: parameters.kerr_angle_rad,
        seed: parameters.seed,
    })
    .unwrap();

    let outputs = (1..=6)
        .map(|harmonic| {
            let output = analyze_boxcar_legacy(
                &signal,
                BoxcarLegacySettings {
                    start_time_s: 0.0,
                    sample_interval_s: 1.0 / parameters.sample_rate_hz,
                    reference_frequency_hz: parameters.reference_frequency_hz,
                    reference_phase_rad: 0.0,
                    half_window_cycles: parameters.half_window_cycles,
                    stride_samples: parameters.stride_samples,
                    harmonic,
                },
            )
            .unwrap();
            let rotated = rotate_phase(&output.x, &output.y, parameters.rotation_rad);
            (output, rotated)
        })
        .collect::<Vec<_>>();
    assert_eq!(fixture.expected.harmonics.len(), outputs.len());

    for (harmonic_index, (actual, expected)) in
        outputs.iter().zip(&fixture.expected.harmonics).enumerate()
    {
        let (output, (in_phase, out_of_phase)) = actual;
        assert_eq!(expected.harmonic, harmonic_index + 1);
        assert_eq!(
            output.metadata.output_samples,
            fixture.expected.output_samples
        );
        assert_eq!(
            output.metadata.first_input_index,
            fixture.expected.first_input_index
        );
        assert_eq!(
            output.metadata.last_input_index,
            fixture.expected.last_input_index
        );
        assert_close(
            output.metadata.output_rate_hz,
            fixture.expected.output_rate_hz,
            fixture.tolerances.lockin_phase_abs,
            fixture.tolerances.lockin_phase_rel,
        );
        let index = fixture.expected.sample_index;
        for (actual, expected) in [
            (output.time_s[index], expected.time_s),
            (output.x[index], expected.x),
            (output.y[index], expected.y),
            (in_phase[index], expected.in_phase),
            (out_of_phase[index], expected.out_of_phase),
        ] {
            assert_close(
                actual,
                expected,
                fixture.tolerances.lockin_phase_abs,
                fixture.tolerances.lockin_phase_rel,
            );
        }
    }

    let kerr = calculate_harmonics_kerr(
        &outputs[1].1.0,
        &outputs[2].1.0,
        &outputs[3].1.0,
        &outputs[5].1.0,
        parameters.kerr_factor,
    )
    .unwrap();
    assert_close(
        kerr.representative_modulation_depth,
        fixture.expected.modulation_depth,
        fixture.tolerances.kerr_abs,
        fixture.tolerances.kerr_rel,
    );
    assert_close(
        kerr.values_rad[fixture.expected.sample_index],
        fixture.expected.kerr_rad,
        fixture.tolerances.kerr_abs,
        fixture.tolerances.kerr_rel,
    );
    for value in kerr.values_rad {
        let tolerance = fixture.tolerances.kerr_abs
            + fixture.tolerances.kerr_rel * parameters.kerr_angle_rad.abs();
        assert!(
            (value - parameters.kerr_angle_rad).abs() <= tolerance,
            "expected {}, got {value}",
            parameters.kerr_angle_rad,
        );
    }
}

fn assert_close(actual: f64, expected: f64, absolute: f64, relative: f64) {
    let tolerance = absolute + relative * expected.abs();
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}, tolerance {tolerance}"
    );
}
