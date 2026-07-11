use numpy::PyUntypedArrayMethods;
use pmoke::config::{Lockin, LockinLpfKind};
use pmoke::lockin::lockin_core::LockinProcessor;
use pmoke::lockin::sensor::pulse_calculator::PulseIntegralCalculator;
use pmoke::python;
use pmoke::utils::raw_csv::{RawCsvChannel, write_raw_csv};
use pmoke::utils::raw_data::RawTimeAxis;
use pmoke::utils::time_axis::WaveformTime;
use pmoke::utils::waveform::WaveformData;
use pmoke::utils::waveform::convert_raw_word_to_voltages;
use pyo3::Python;
use rayon::prelude::*;
use serde::Serialize;
use std::env;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    case: &'static str,
    commit: String,
    rustc: String,
    os: &'static str,
    arch: &'static str,
    samples: usize,
    raw_csv_channels: usize,
    iterations: usize,
    results: Vec<Measurement>,
    python_transfer: python::PythonTransferStats,
}

#[derive(Serialize)]
struct Measurement {
    name: &'static str,
    median_seconds: f64,
    samples_per_second: f64,
    input_bytes: usize,
    output_values: usize,
}

struct Options {
    case: BenchmarkCase,
    samples: usize,
    channels: usize,
    iterations: usize,
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BenchmarkCase {
    All,
    RawWordDecode,
    SensorIntegral,
    LockinWorker1,
    LockinWorker2,
    PythonCopy,
    RawToCsv,
    AnalysisPipeline,
}

impl BenchmarkCase {
    fn parse(value: &str) -> Self {
        match value {
            "all" => Self::All,
            "raw_word_decode" => Self::RawWordDecode,
            "sensor_integral" => Self::SensorIntegral,
            "lockin_w1" => Self::LockinWorker1,
            "lockin_w2" => Self::LockinWorker2,
            "python_copy" => Self::PythonCopy,
            "raw_to_csv" => Self::RawToCsv,
            "analysis_pipeline" => Self::AnalysisPipeline,
            other => panic!("unknown benchmark case: {other}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::RawWordDecode => "raw_word_decode",
            Self::SensorIntegral => "sensor_integral",
            Self::LockinWorker1 => "lockin_w1",
            Self::LockinWorker2 => "lockin_w2",
            Self::PythonCopy => "python_copy",
            Self::RawToCsv => "raw_to_csv",
            Self::AnalysisPipeline => "analysis_pipeline",
        }
    }

    fn runs(self, target: Self) -> bool {
        self == target || (self == Self::All && target != Self::AnalysisPipeline)
    }
}

fn main() {
    let options = parse_options();
    let words = (options.case.runs(BenchmarkCase::RawWordDecode)
        || options.case.runs(BenchmarkCase::RawToCsv))
    .then(|| synthetic_words(options.samples));
    let needs_lockin_input = options.case.runs(BenchmarkCase::LockinWorker1)
        || options.case.runs(BenchmarkCase::LockinWorker2);
    let time_and_signal = needs_lockin_input.then(|| {
        let times = synthetic_times(options.samples);
        let signal = synthetic_signal(&times);
        (times, signal)
    });
    let standalone_signal = (!needs_lockin_input
        && (options.case.runs(BenchmarkCase::SensorIntegral)
            || options.case.runs(BenchmarkCase::PythonCopy)))
    .then(|| {
        let times = synthetic_times(options.samples);
        synthetic_signal(&times)
    });
    let raw_csv = options.case.runs(BenchmarkCase::RawToCsv).then(|| {
        BenchmarkRawCsv::new(
            words.as_deref().expect("RAW words are available"),
            options.channels,
        )
    });
    let analysis = options
        .case
        .runs(BenchmarkCase::AnalysisPipeline)
        .then(|| AnalysisFixture::new(options.samples));

    // One untimed run per selected case catches invalid inputs and reduces first-use noise.
    if options.case.runs(BenchmarkCase::RawWordDecode) {
        black_box(
            convert_raw_word_to_voltages(
                words.as_deref().expect("RAW words are available"),
                2.5e-4,
                0.0,
                32_768.0,
            )
            .unwrap(),
        );
    }
    if options.case.runs(BenchmarkCase::SensorIntegral) {
        let signal = selected_signal(&time_and_signal, &standalone_signal);
        black_box(PulseIntegralCalculator::new(1.0e-7).integrate(signal, 0.125, -2.0));
    }
    for (case, workers) in [
        (BenchmarkCase::LockinWorker1, 1),
        (BenchmarkCase::LockinWorker2, 2),
    ] {
        if options.case.runs(case) {
            let (times, signal) = time_and_signal.as_ref().expect("signal is available");
            black_box(run_lockin_harmonics(times, signal, workers));
        }
    }
    if options.case.runs(BenchmarkCase::PythonCopy) {
        let signal = selected_signal(&time_and_signal, &standalone_signal);
        Python::attach(|py| black_box(python::f64_array1(py, signal).len()));
    }
    if let Some(raw_csv) = &raw_csv {
        raw_csv.write(options.samples);
    }
    if let Some(analysis) = &analysis {
        analysis.run();
    }
    python::reset_transfer_stats();

    let mut results = Vec::new();
    if options.case.runs(BenchmarkCase::RawWordDecode) {
        let words = words.as_deref().expect("RAW words are available");
        results.push(measure(
            "raw_word_decode",
            options.samples,
            words.len(),
            options.iterations,
            || {
                convert_raw_word_to_voltages(black_box(words), 2.5e-4, 0.0, 32_768.0)
                    .unwrap()
                    .len()
            },
        ));
    }
    if options.case.runs(BenchmarkCase::SensorIntegral) {
        let signal = selected_signal(&time_and_signal, &standalone_signal);
        results.push(measure(
            "sensor_integral",
            options.samples,
            std::mem::size_of_val(signal),
            options.iterations,
            || {
                PulseIntegralCalculator::new(1.0e-7)
                    .integrate(black_box(signal), 0.125, -2.0)
                    .len()
            },
        ));
    }
    for (case, name, workers) in [
        (
            BenchmarkCase::LockinWorker1,
            "boxcar_legacy_harmonics_w1",
            1,
        ),
        (
            BenchmarkCase::LockinWorker2,
            "boxcar_legacy_harmonics_w2",
            2,
        ),
    ] {
        if options.case.runs(case) {
            let (times, signal) = time_and_signal.as_ref().expect("signal is available");
            results.push(measure(
                name,
                options.samples,
                (times.len() + signal.len()) * std::mem::size_of::<f64>(),
                options.iterations,
                || run_lockin_harmonics(black_box(times), black_box(signal), workers),
            ));
        }
    }
    if options.case.runs(BenchmarkCase::PythonCopy) {
        let signal = selected_signal(&time_and_signal, &standalone_signal);
        results.push(measure(
            "python_f64_array_copy",
            options.samples,
            std::mem::size_of_val(signal),
            options.iterations,
            || Python::attach(|py| python::f64_array1(py, black_box(signal)).len()),
        ));
    }
    if let Some(raw_csv) = &raw_csv {
        let words = words.as_deref().expect("RAW words are available");
        results.push(measure(
            "raw_to_csv",
            options.samples,
            words.len() * options.channels,
            options.iterations,
            || {
                raw_csv.write(options.samples);
                options.samples
            },
        ));
    }
    if let Some(analysis) = &analysis {
        results.push(measure(
            "analysis_pipeline",
            options.samples,
            options.samples * 4 * std::mem::size_of::<f64>(),
            options.iterations,
            || {
                analysis.run();
                options.samples
            },
        ));
    }

    let report = Report {
        schema_version: 5,
        case: options.case.name(),
        commit: command_output("git", &["rev-parse", "HEAD"]),
        rustc: command_output("rustc", &["--version"]),
        os: env::consts::OS,
        arch: env::consts::ARCH,
        samples: options.samples,
        raw_csv_channels: options.channels,
        iterations: options.iterations,
        results,
        python_transfer: python::transfer_stats(),
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
    let mut case = BenchmarkCase::All;
    let mut channels = 2;
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
            "--case" => {
                case = BenchmarkCase::parse(&args.next().expect("--case requires a value"));
            }
            "--iterations" => {
                iterations_explicit = true;
                iterations = args
                    .next()
                    .expect("--iterations requires a value")
                    .parse()
                    .expect("--iterations must be a positive integer");
            }
            "--channels" => {
                channels = args
                    .next()
                    .expect("--channels requires a value")
                    .parse()
                    .expect("--channels must be 2 or 4");
            }
            "--output" => output = Some(args.next().expect("--output requires a path").into()),
            "--end-to-end" => case = BenchmarkCase::AnalysisPipeline,
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
    assert!(matches!(channels, 2 | 4), "channels must be 2 or 4");
    assert!(
        case != BenchmarkCase::AnalysisPipeline || samples >= 100_000,
        "analysis_pipeline requires at least 100000 samples"
    );
    Options {
        case,
        samples,
        channels,
        iterations,
        output,
    }
}

fn measure(
    name: &'static str,
    samples: usize,
    input_bytes: usize,
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
        input_bytes,
        output_values,
    }
}

fn selected_signal<'a>(
    time_and_signal: &'a Option<(Vec<f64>, Vec<f64>)>,
    standalone_signal: &'a Option<Vec<f64>>,
) -> &'a [f64] {
    time_and_signal
        .as_ref()
        .map(|(_, signal)| signal.as_slice())
        .or(standalone_signal.as_deref())
        .expect("signal is available")
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

fn run_lockin_harmonics(times: &[f64], signal: &[f64], workers: usize) -> usize {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("create benchmark thread pool")
        .install(|| {
            (1..=6)
                .into_par_iter()
                .map(|harmonic| {
                    let lockin = benchmark_lockin();
                    let processor = LockinProcessor::new(times, signal, 10_000.0, 0.2, &lockin)
                        .expect("valid benchmark lock-in configuration");
                    let result = processor.compute_harmonic_detailed(harmonic, false);
                    result.li_x.len() + result.li_y.len()
                })
                .sum()
        })
}

fn benchmark_lockin() -> Lockin {
    Lockin {
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
    }
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

struct BenchmarkRawCsv {
    dir: PathBuf,
    output: PathBuf,
    files: Vec<String>,
}

impl BenchmarkRawCsv {
    fn new(words: &[u8], channels: usize) -> Self {
        let unique = format!(
            "pmoke-performance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before Unix epoch")
                .as_nanos()
        );
        let dir = env::temp_dir().join(unique);
        fs::create_dir(&dir).expect("create benchmark directory");
        let files = (1..=channels)
            .map(|channel| format!("ch{channel}.u16le"))
            .collect::<Vec<_>>();
        for file in &files {
            fs::write(dir.join(file), words).expect("write benchmark raw data");
        }
        let output = dir.join("waveform.csv");
        Self { dir, output, files }
    }

    fn write(&self, sample_count: usize) {
        if self.output.exists() {
            fs::remove_file(&self.output).expect("remove previous benchmark CSV");
        }
        let channels = self
            .files
            .iter()
            .map(|file| RawCsvChannel {
                file,
                sample_count,
                x_increment: 1.0e-7,
                x_origin: -0.001,
                x_reference: 0.0,
                y_increment: 2.5e-4,
                y_origin: 0.0,
                y_reference: 32_768.0,
            })
            .collect::<Vec<_>>();
        let headers = std::iter::once("time".to_owned())
            .chain((1..=self.files.len()).map(|channel| format!("ch{channel}")))
            .collect::<Vec<_>>();
        let header_refs = headers.iter().map(String::as_str).collect::<Vec<_>>();
        write_raw_csv(&self.output, &header_refs, &self.dir, &channels)
            .expect("write benchmark raw CSV");
    }
}

impl Drop for BenchmarkRawCsv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct AnalysisFixture {
    dir: PathBuf,
    config: pmoke::config::Config,
    data: WaveformData,
}

impl AnalysisFixture {
    fn new(samples: usize) -> Self {
        let dir = unique_temp_dir("analysis");
        fs::create_dir(&dir).expect("create analysis benchmark directory");
        let config_path = dir.join("config.toml");
        fs::write(&config_path, analysis_config()).expect("write analysis benchmark config");
        let (config, warnings) = pmoke::config::load_from_path(&config_path)
            .into_ready()
            .expect("load analysis benchmark config");
        assert!(warnings.is_empty());

        let dt = 1.0e-7;
        let t0 = -0.001;
        let omega = 2.0 * std::f64::consts::PI * 10_000.0;
        let mut sensor1 = Vec::with_capacity(samples);
        let mut sensor2 = Vec::with_capacity(samples);
        let mut reference = Vec::with_capacity(samples);
        let mut signal = Vec::with_capacity(samples);
        for index in 0..samples {
            let time = t0 + index as f64 * dt;
            let pulse = if (0.0..=0.006).contains(&time) {
                (std::f64::consts::PI * time / 0.006).sin().max(0.0)
            } else {
                0.0
            };
            sensor1.push(pulse);
            sensor2.push(0.5 * pulse);
            reference.push((omega * time).sin());
            signal.push(
                (1..=6)
                    .map(|harmonic| {
                        0.2 / harmonic as f64
                            * (harmonic as f64 * omega * time + 0.1 * harmonic as f64).sin()
                    })
                    .sum(),
            );
        }

        Self {
            dir,
            config,
            data: WaveformData {
                t: WaveformTime::Uniform(RawTimeAxis {
                    sample_count: samples,
                    x_increment: dt,
                    x_origin: t0,
                    x_reference: 0.0,
                }),
                channels: vec![sensor1, sensor2, reference, signal],
            },
        }
    }

    fn run(&self) {
        let _cwd = CurrentDirGuard::enter(&self.dir);
        pmoke::run_analysis_pipeline(&self.config, &self.data)
            .expect("run end-to-end analysis benchmark");
    }
}

struct CurrentDirGuard {
    previous: PathBuf,
}

impl CurrentDirGuard {
    fn enter(path: &std::path::Path) -> Self {
        let previous = env::current_dir().expect("read benchmark current directory");
        env::set_current_dir(path).expect("enter analysis benchmark directory");
        Self { previous }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.previous).expect("restore benchmark current directory");
    }
}

impl Drop for AnalysisFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn unique_temp_dir(label: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "pmoke-performance-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos()
    ))
}

fn analysis_config() -> &'static str {
    r#"version = 4

[scope]
model = "DHO5108"
connection = "tcp://127.0.0.1:55255"

[data]
output = "raw"
input = "raw"
screenshot = false

[[sensors]]
channel = 1
scale = { factor = 1.0 }
label = "B"
unit = "T"

[[sensors]]
channel = 2
scale = { factor = 1.0 }
label = "V"
unit = "V"

[pulse]
background_before = { start = -1e-3, end = -0.2e-3 }
background_after = { start = 8e-3, end = 9e-3 }

[reference]
channel = 3
fft_window = { start = 0.0, end = 8e-3 }
stride_samples = 10000
window_samples = 1000

[lockin]
signal_channels = [4]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0, 0, 0, 0, 0, 0]

[kerr]
sensor = 1
method = "standard"
factor = 1.0

[plot]
mode = "off"
"#
}
