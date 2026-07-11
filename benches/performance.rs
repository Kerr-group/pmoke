use pmoke::config::{Lockin, LockinLpfKind};
use pmoke::lockin::lockin_core::LockinProcessor;
use pmoke::lockin::sensor::pulse_calculator::PulseIntegralCalculator;
use pmoke::utils::waveform::convert_raw_word_to_voltages;
use serde::Serialize;
use std::env;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    commit: String,
    rustc: String,
    os: &'static str,
    arch: &'static str,
    samples: usize,
    iterations: usize,
    results: Vec<Measurement>,
}

#[derive(Serialize)]
struct Measurement {
    name: &'static str,
    median_seconds: f64,
    samples_per_second: f64,
    output_values: usize,
}

struct Options {
    samples: usize,
    iterations: usize,
    output: Option<PathBuf>,
}

fn main() {
    let options = parse_options();
    let words = synthetic_words(options.samples);
    let times = synthetic_times(options.samples);
    let signal = synthetic_signal(&times);

    // One untimed run catches invalid inputs and reduces first-use noise.
    black_box(convert_raw_word_to_voltages(&words, 2.5e-4, 0.0, 32_768.0).unwrap());
    black_box(PulseIntegralCalculator::new(1.0e-7).integrate(&signal, 0.125, -2.0));
    black_box(run_lockin(&times, &signal));

    let results = vec![
        measure(
            "raw_word_decode",
            options.samples,
            options.iterations,
            || {
                convert_raw_word_to_voltages(black_box(&words), 2.5e-4, 0.0, 32_768.0)
                    .unwrap()
                    .len()
            },
        ),
        measure(
            "sensor_integral",
            options.samples,
            options.iterations,
            || {
                PulseIntegralCalculator::new(1.0e-7)
                    .integrate(black_box(&signal), 0.125, -2.0)
                    .len()
            },
        ),
        measure("boxcar_legacy", options.samples, options.iterations, || {
            run_lockin(black_box(&times), black_box(&signal))
        }),
    ];

    let report = Report {
        schema_version: 1,
        commit: command_output("git", &["rev-parse", "HEAD"]),
        rustc: command_output("rustc", &["--version"]),
        os: env::consts::OS,
        arch: env::consts::ARCH,
        samples: options.samples,
        iterations: options.iterations,
        results,
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize benchmark report");
    println!("{json}");
    if let Some(path) = options.output {
        fs::write(&path, format!("{json}\n"))
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    }
}

fn parse_options() -> Options {
    // `cargo test --all-targets` executes custom bench harnesses too. Keep that
    // implicit invocation tiny; `cargo bench` supplies `--bench` and receives
    // the representative defaults below.
    let mut samples = 100_000;
    let mut iterations = 1;
    let mut cargo_bench = false;
    let mut samples_explicit = false;
    let mut iterations_explicit = false;
    let mut output = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // `cargo bench` passes this rustc-compatible marker to custom harnesses.
            "--bench" => cargo_bench = true,
            "--smoke" => {
                samples = 100_000;
                iterations = 1;
                samples_explicit = true;
                iterations_explicit = true;
            }
            "--samples" => {
                samples_explicit = true;
                samples = args
                    .next()
                    .expect("--samples requires a value")
                    .parse()
                    .expect("--samples must be a positive integer");
            }
            "--iterations" => {
                iterations_explicit = true;
                iterations = args
                    .next()
                    .expect("--iterations requires a value")
                    .parse()
                    .expect("--iterations must be a positive integer");
            }
            "--output" => output = Some(args.next().expect("--output requires a path").into()),
            other => panic!("unknown benchmark option: {other}"),
        }
    }
    if cargo_bench {
        if !samples_explicit {
            samples = 1_000_000;
        }
        if !iterations_explicit {
            iterations = 5;
        }
    }
    assert!(samples >= 1_000, "samples must be at least 1000");
    assert!(iterations > 0, "iterations must be positive");
    Options {
        samples,
        iterations,
        output,
    }
}

fn measure(
    name: &'static str,
    samples: usize,
    iterations: usize,
    mut operation: impl FnMut() -> usize,
) -> Measurement {
    let mut durations = Vec::with_capacity(iterations);
    let mut output_values = 0;
    for _ in 0..iterations {
        let start = Instant::now();
        output_values = black_box(operation());
        durations.push(start.elapsed());
    }
    durations.sort_unstable();
    let median = median(&durations).as_secs_f64();
    Measurement {
        name,
        median_seconds: median,
        samples_per_second: samples as f64 / median,
        output_values,
    }
}

fn median(values: &[Duration]) -> Duration {
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2
    }
}

fn synthetic_words(samples: usize) -> Vec<u8> {
    (0..samples)
        .flat_map(|index| {
            let word = ((index.wrapping_mul(4051).wrapping_add(7919)) & 0xffff) as u16;
            word.to_le_bytes()
        })
        .collect()
}

fn synthetic_times(samples: usize) -> Vec<f64> {
    let dt = 1.0e-7;
    (0..samples).map(|index| index as f64 * dt).collect()
}

fn synthetic_signal(times: &[f64]) -> Vec<f64> {
    let omega = 2.0 * std::f64::consts::PI * 10_000.0;
    times
        .iter()
        .enumerate()
        .map(|(index, &time)| {
            let noise = ((index.wrapping_mul(17) % 101) as f64 - 50.0) * 1.0e-4;
            0.125 + 0.8 * (omega * time + 0.2).sin() + noise
        })
        .collect()
}

fn run_lockin(times: &[f64], signal: &[f64]) -> usize {
    let lockin = Lockin {
        workers: 1,
        stride_samples: 100,
        lpf_kind: LockinLpfKind::BoxcarLegacy,
        lpf_half_window_cycles: 1.0,
        lpf_cutoff_hz: None,
        lpf_cutoff_ref_ratio: None,
        lpf_stopband_atten_db: 60.0,
        lpf_sync_average_cycles: 1.0,
        lpf_iir_order: 2,
        lpf_debug_output: false,
        lpf_debug_label: None,
        lpf_debug_overwrite: false,
        snr_background_window: None,
        snr_signal_window: None,
    };
    let processor = LockinProcessor::new(times, signal, 10_000.0, 0.2, &lockin)
        .expect("valid benchmark lock-in configuration");
    let result = processor.compute_harmonic_detailed(1, false);
    black_box(result.li_x.len() + result.li_y.len())
}

fn command_output(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}
