//! Recorded-data `pmoke analyze` acceptance for the stage-order and
//! combined-signal-plot contract.
//!
//! These tests drive the real CLI in a child process because the embedded
//! Python plotting stack renders on the process main thread (matplotlib's
//! macOS GUI backend refuses to build figures from cargo-test worker
//! threads), and because the JSONL event stream of a real run is the
//! evidence surface for the execution order and the single signal plot
//! completion line.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const ORIGIN_S: f64 = -0.005;
const DT_S: f64 = 1.0e-5;
const SAMPLES: usize = 8_000;
const FREQUENCY_HZ: f64 = 1_000.0;
const THETA: f64 = 0.01;
const STRIDE_SAMPLES: usize = 20;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pmoke_analyze_signal_stage_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// v7 configuration: sensor ch1, reference ch2, lock-in ch3, raw `[[signals]]`
/// readout entries for the requested channels.
fn config_text(signal_channels: &[u8]) -> String {
    let signals = signal_channels
        .iter()
        .map(|channel| {
            format!("\n[[signals]]\nchannel = {channel}\nlabel = \"DC{channel}\"\nunit = \"V\"\n")
        })
        .collect::<String>();
    format!(
        r#"version = 7

[scope]
model = "DHO5108"
connection = "tcp://127.0.0.1:55255"

[data]
output = "raw"
input = "csv"
screenshot = false

[[sensors]]
channel = 1
scale = {{ factor = 1.0 }}
label = "field"
unit = "T"

[pulse]
background_before = {{ start = -4.5e-3, end = -0.5e-3 }}
background_after = {{ start = 60e-3, end = 75e-3 }}

[reference]
channel = 2
fft_window = {{ start = 0.0, end = 30e-3 }}
stride_samples = 1_000
window_samples = 100

[lockin]
channels = [3]
workers = 2
stride_samples = {STRIDE_SAMPLES}

[lockin.window]
kind = "reference_cycles"
half_window_cycles = 1.3
edge_policy = "legacy_trim"

[lockin.estimator]
kind = "boxcar_legacy"

[phase]
offsets = [0, 0, 0, 0, 0, 0]

[moke]
sensor = 1
method = "harmonics"
factor = 1.0

[plot]
mode = "save"
decimation = "none"
on_error = "fail"
{signals}"#
    )
}

fn signal_time(index: usize) -> f64 {
    ORIGIN_S + index as f64 * DT_S
}

/// Recorded waveform matching the documented channels: ch1 sensor pulse,
/// ch2 reference sine, ch3 lock-in signal (Bessel harmonics), ch4..
/// deterministic raw readout channels.
fn write_waveform_csv(run_dir: &Path, max_channel: u8) {
    let bessel = [
        0.581_864_936_842_083_3,
        0.315_745_306_087_972_3,
        0.104_537_902_479_595_42,
        0.025_139_158_519_404_087,
        0.004_762_786_735_204_94,
        0.000_745_551_998_014_054_3,
    ];
    let mut csv = String::from("time");
    for channel in 1..=max_channel {
        csv.push_str(&format!(",ch{channel}"));
    }
    csv.push('\n');
    for index in 0..SAMPLES {
        let t = signal_time(index);
        let sensor = if (0.01..0.05).contains(&t) { 1.0 } else { 0.0 };
        let reference = {
            let amplitude_drift = 1.0 + 0.01 * (2.0 * std::f64::consts::PI * 3.0 * t).sin();
            0.02 + 0.01 * t
                + amplitude_drift * (2.0 * std::f64::consts::PI * FREQUENCY_HZ * t).sin()
        };
        let lockin = {
            let harmonics = bessel
                .iter()
                .enumerate()
                .map(|(index, coefficient)| {
                    let harmonic = index + 1;
                    let amplitude = if harmonic % 2 == 0 {
                        (2.0 * THETA).cos() * coefficient
                    } else {
                        (2.0 * THETA).sin() * coefficient
                    };
                    let phase = if harmonic % 2 == 0 {
                        std::f64::consts::FRAC_PI_2
                    } else {
                        std::f64::consts::PI
                    };
                    2.0 * amplitude
                        * (harmonic as f64 * 2.0 * std::f64::consts::PI * FREQUENCY_HZ * t + phase)
                            .sin()
                })
                .sum::<f64>();
            let deterministic_noise =
                1.0e-5 * (2.0 * std::f64::consts::PI * 12_345.0 * t + 0.4).sin();
            0.01 + 0.002 * t + harmonics + deterministic_noise
        };
        csv.push_str(&format!(
            "{t:.15e},{sensor:.15e},{reference:.15e},{lockin:.15e}"
        ));
        for channel in 4..=max_channel {
            let scale = f64::from(channel);
            let value = 0.05 * scale
                + 0.01 * scale * t
                + 0.001 * scale * (2.0 * std::f64::consts::PI * 7.3 * t).sin();
            csv.push_str(&format!(",{value:.15e}"));
        }
        csv.push('\n');
    }
    fs::write(run_dir.join("raw.csv"), csv).unwrap();
}

fn write_fixture(run_dir: &Path, config: &Path, signal_channels: &[u8]) {
    fs::create_dir_all(run_dir).unwrap();
    let max_channel = signal_channels.iter().copied().max().unwrap_or(4).max(4);
    write_waveform_csv(run_dir, max_channel);
    fs::write(config, config_text(signal_channels)).unwrap();
}

struct AnalyzeRun {
    success: bool,
    stderr: String,
    events: Vec<(String, Option<u64>)>,
}

impl AnalyzeRun {
    fn messages(&self) -> Vec<&str> {
        self.events
            .iter()
            .map(|(message, _)| message.as_str())
            .collect()
    }

    fn single_event_index(&self, marker: &str) -> usize {
        let matches = self
            .events
            .iter()
            .enumerate()
            .filter(|(_, (message, _))| message.contains(marker))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one event containing {marker:?} in {:?}",
            self.messages()
        );
        matches[0]
    }
}

fn run_analyze(run_dir: &Path, config: &Path) -> AnalyzeRun {
    let output = Command::new(env!("CARGO_BIN_EXE_pmoke"))
        .arg("--config")
        .arg(config)
        .arg("--run-dir")
        .arg(run_dir)
        .arg("analyze")
        .env("PMOKE_OUTPUT", "jsonl")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("MPLBACKEND", "Agg")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .map(|line| (json_string(line, "message"), json_u64(line, "duration_ms")))
        .collect::<Vec<_>>();
    AnalyzeRun {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        events,
    }
}

fn json_string(line: &str, key: &str) -> String {
    let marker = format!("\"{key}\":\"");
    let start = line.find(&marker).map(|index| index + marker.len());
    let Some(start) = start else {
        return String::new();
    };
    let mut value = String::new();
    let mut chars = line[start..].chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => break,
            '\\' => {
                if let Some(escaped) = chars.next() {
                    value.push(escaped);
                }
            }
            _ => value.push(character),
        }
    }
    value
}

fn json_u64(line: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = line.find(&marker)? + marker.len();
    let digits = line[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok()
}

fn plot_names(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn assert_stage_order(run: &AnalyzeRun) -> Vec<usize> {
    let mut previous = None;
    let mut indexes = Vec::new();
    for marker in [
        "sensor integrations completed",
        "reference FFT: f_ref",
        "signal means for channels",
        "lock-in processing completed",
        "phase analysis completed",
        "Moke analysis completed",
    ] {
        let index = run.single_event_index(marker);
        if let Some(previous) = previous {
            assert!(
                previous < index,
                "stage order violated at {marker:?}: {previous} !< {index}"
            );
        }
        previous = Some(index);
        indexes.push(index);
    }
    indexes
}

#[test]
fn analyze_cli_runs_sensor_reference_signal_lockin_phase_moke_and_one_combined_plot() {
    let temp = TempDir::new();
    let run_dir = temp.0.join("run");
    let config = temp.0.join("config.toml");
    write_fixture(&run_dir, &config, &[4]);

    let run = run_analyze(&run_dir, &config);
    assert!(
        run.success,
        "analyze failed; stderr:\n{}\nevents:\n{:?}",
        run.stderr,
        run.messages()
    );
    assert_stage_order(&run);

    // Exactly one signal plot completion line, with a measured duration, and
    // no per-channel signal plot lines.
    let plot_events = run
        .events
        .iter()
        .filter(|(message, _)| message.starts_with("signal plot completed"))
        .collect::<Vec<_>>();
    assert_eq!(plot_events.len(), 1, "events: {:?}", run.messages());
    assert!(
        plot_events[0].0.contains("signal plot completed ("),
        "completion line must report the measured duration: {}",
        plot_events[0].0
    );
    assert!(plot_events[0].1.unwrap_or(0) > 0);
    assert!(
        !run.messages()
            .iter()
            .any(|message| message.contains("signal ch") && message.contains("plot completed"))
    );

    // Only the combined figure is written.
    assert_eq!(
        plot_names(&run_dir.join("analysis/plots/signal")),
        vec!["mean.png".to_string()]
    );

    // The manifest registers the combined CSV and exactly one signal plot.
    let manifest = fs::read_to_string(run_dir.join("analysis/manifest.toml")).unwrap();
    assert_eq!(manifest.matches("kind = \"signal\"").count(), 1);
    assert_eq!(manifest.matches("kind = \"signal_plot\"").count(), 1);
    assert!(manifest.contains("file = \"plots/signal/mean.png\""));
    assert!(!manifest.contains("_mean.png\""));
    assert!(manifest.contains("csv = \"signal/signal.csv\""));
}

#[test]
fn analyze_cli_rerun_with_multiple_signals_keeps_only_the_combined_plot() {
    let temp = TempDir::new();
    let run_dir = temp.0.join("run");
    let config = temp.0.join("config.toml");
    write_fixture(&run_dir, &config, &[4, 8]);

    let first = run_analyze(&run_dir, &config);
    assert!(first.success, "first analyze failed:\n{}", first.stderr);
    assert_stage_order(&first);

    let signal_csv = fs::read(run_dir.join("analysis/signal/signal.csv")).unwrap();
    let header = String::from_utf8_lossy(&signal_csv)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        header.starts_with(
            "time (s),field rate (T/s),field integral (T),Ch4 DC4 mean (V),Ch8 DC8 mean (V)"
        ),
        "unexpected signal CSV header: {header}"
    );

    // Simulate an obsolete per-channel figure left by an older generation.
    fs::write(
        run_dir.join("analysis/plots/signal/ch4_mean.png"),
        b"\x89PNG\r\n\x1a\nstale",
    )
    .unwrap();

    let second = run_analyze(&run_dir, &config);
    assert!(second.success, "rerun failed:\n{}", second.stderr);
    assert_eq!(
        plot_names(&run_dir.join("analysis/plots/signal")),
        vec!["mean.png".to_string()]
    );
    assert_eq!(
        fs::read(run_dir.join("analysis/signal/signal.csv")).unwrap(),
        signal_csv
    );

    let manifest = fs::read_to_string(run_dir.join("analysis/manifest.toml")).unwrap();
    assert_eq!(manifest.matches("generation = 2").count(), 1);
    assert_eq!(manifest.matches("kind = \"signal_plot\"").count(), 1);
    assert!(!manifest.contains("ch4_mean.png"));
    assert!(!manifest.contains("ch8_mean.png"));
}

#[test]
fn analyze_cli_without_signals_writes_no_signal_artifact_or_plot() {
    let temp = TempDir::new();
    let run_dir = temp.0.join("run");
    let config = temp.0.join("config.toml");
    write_fixture(&run_dir, &config, &[]);

    let run = run_analyze(&run_dir, &config);
    assert!(run.success, "analyze failed:\n{}", run.stderr);
    assert!(
        run.messages()
            .iter()
            .any(|message| message.contains("lock-in processing completed"))
    );
    assert!(
        run.events
            .iter()
            .any(|(message, _)| message.contains("no [[signals]] entries specified"))
    );
    assert!(
        !run.events
            .iter()
            .any(|(message, _)| message.starts_with("signal plot completed"))
    );
    assert!(!run_dir.join("analysis/signal").exists());
    assert!(!run_dir.join("analysis/plots/signal").exists());
    let manifest = fs::read_to_string(run_dir.join("analysis/manifest.toml")).unwrap();
    assert!(!manifest.contains("kind = \"signal_plot\""));
    assert!(manifest.contains("kind = \"moke\""));
}

#[test]
fn signal_cli_keeps_its_stage_and_artifacts_with_the_shared_preparation() {
    let temp = TempDir::new();
    let run_dir = temp.0.join("run");
    let config = temp.0.join("config.toml");
    write_fixture(&run_dir, &config, &[4]);

    let output = Command::new(env!("CARGO_BIN_EXE_pmoke"))
        .arg("--config")
        .arg(&config)
        .arg("--run-dir")
        .arg(&run_dir)
        .arg("signal")
        .env("PMOKE_OUTPUT", "jsonl")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("MPLBACKEND", "Agg")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "signal failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The standalone command keeps its documented artifacts: the signal CSV
    // and combined plot plus the lock-in results it publishes.
    assert!(run_dir.join("analysis/signal/signal.csv").is_file());
    assert_eq!(
        plot_names(&run_dir.join("analysis/plots/signal")),
        vec!["mean.png".to_string()]
    );
    assert!(run_dir.join("analysis/lockin/ch3_xy.csv").is_file());
    assert!(run_dir.join("analysis/sensor/sensor.csv").is_file());
    let manifest = fs::read_to_string(run_dir.join("analysis/manifest.toml")).unwrap();
    assert!(manifest.contains("[stages.signal]"));
    assert_eq!(manifest.matches("kind = \"signal_plot\"").count(), 1);
    assert!(!manifest.contains("ch4_mean.png"));
}
