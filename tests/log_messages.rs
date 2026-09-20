//! CLI/TUI log-message contract for the analysis pipeline.
//!
//! These tests drive the real CLI in a child process with
//! `PMOKE_OUTPUT=jsonl` and assert the machine protocol and the
//! human-message contract together:
//!
//! - every JSONL line stays parseable with the documented `UiEvent` shape
//!   (no new fields, levels/kinds from the known sets, strictly increasing
//!   sequence numbers);
//! - stage timings attribute compute vs save truthfully instead of folding
//!   compute time into a save line;
//! - the monitor timeline marker substrings keep matching (case-insensitive)
//!   so every TUI stage step can complete.

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
            "pmoke_log_messages_{}_{}_{}",
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

/// v7 configuration mirroring the signal-stage acceptance fixture: sensor
/// ch1, reference ch2, lock-in ch3, one raw `[[signals]]` readout on ch4.
fn config_text() -> String {
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

[[signals]]
channel = 4
label = "DC4"
unit = "V"
"#
    )
}

fn signal_time(index: usize) -> f64 {
    ORIGIN_S + index as f64 * DT_S
}

/// Recorded waveform matching the documented channels: ch1 sensor pulse,
/// ch2 reference sine, ch3 lock-in signal (Bessel harmonics), ch4
/// deterministic raw readout channel.
fn write_waveform_csv(run_dir: &Path) {
    let bessel = [
        0.581_864_936_842_083_3,
        0.315_745_306_087_972_3,
        0.104_537_902_479_595_42,
        0.025_139_158_519_404_087,
        0.004_762_786_735_204_94,
        0.000_745_551_998_014_054_3,
    ];
    let mut csv = String::from("time,ch1,ch2,ch3,ch4\n");
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
        let readout = 0.05 * 4.0
            + 0.01 * 4.0 * t
            + 0.001 * 4.0 * (2.0 * std::f64::consts::PI * 7.3 * t).sin();
        csv.push_str(&format!(
            "{t:.15e},{sensor:.15e},{reference:.15e},{lockin:.15e},{readout:.15e}\n"
        ));
    }
    fs::write(run_dir.join("raw.csv"), csv).unwrap();
}

#[derive(Debug)]
struct JsonlEvent {
    message: String,
    level: String,
    kind: String,
    sequence: u64,
    has_duration_ms: bool,
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

/// Parse one JSONL event line into its contract fields. Panics with the raw
/// line attached when the machine protocol shape is violated.
fn parse_event(line: &str) -> JsonlEvent {
    assert!(
        line.contains("\"type\":\"event\""),
        "JSONL line is not a UiEvent: {line}"
    );
    let level = json_string(line, "level");
    assert!(
        ["success", "info", "warning", "error"].contains(&level.as_str()),
        "unknown event level in: {line}"
    );
    let kind = json_string(line, "kind");
    assert!(
        [
            "status", "read", "save", "skip", "progress", "section", "metric", "system", "raw"
        ]
        .contains(&kind.as_str()),
        "unknown event kind in: {line}"
    );
    let sequence = json_u64(line, "sequence")
        .unwrap_or_else(|| panic!("event has no sequence number: {line}"));
    let message = json_string(line, "message");
    assert!(!message.is_empty(), "event has an empty message: {line}");
    JsonlEvent {
        message,
        level,
        kind,
        sequence,
        has_duration_ms: line.contains("\"duration_ms\""),
    }
}

struct AnalyzeRun {
    success: bool,
    stderr: String,
    events: Vec<JsonlEvent>,
}

impl AnalyzeRun {
    fn single_event_index(&self, marker: &str) -> usize {
        let matches = self
            .events
            .iter()
            .enumerate()
            .filter(|(_, event)| event.message.contains(marker))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one event containing {marker:?} in {:?}",
            self.events
                .iter()
                .map(|event| &event.message)
                .collect::<Vec<_>>()
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
        .map(parse_event)
        .collect::<Vec<_>>();
    AnalyzeRun {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        events,
    }
}

fn messages(run: &AnalyzeRun) -> Vec<&str> {
    run.events
        .iter()
        .map(|event| event.message.as_str())
        .collect()
}

#[test]
fn log_message_contract_for_boxcar_analyze() {
    let temp = TempDir::new();
    let run_dir = temp.0.join("run");
    let config = temp.0.join("config.toml");
    fs::create_dir_all(&run_dir).unwrap();
    write_waveform_csv(&run_dir);
    fs::write(&config, config_text()).unwrap();

    let run = run_analyze(&run_dir, &config);
    assert!(run.success, "analyze failed:\n{}", run.stderr);
    assert!(
        !run.events.is_empty(),
        "expected JSONL events on stdout, stderr was:\n{}",
        run.stderr
    );

    // Machine protocol: strictly increasing sequence numbers, no error-level
    // events on a passing run.
    let mut previous: Option<u64> = None;
    for event in &run.events {
        if let Some(previous) = previous {
            assert!(
                event.sequence > previous,
                "event sequences are not strictly increasing: {} !> {previous} in {:?}",
                event.sequence,
                event.message
            );
        }
        previous = Some(event.sequence);
        assert_ne!(
            event.level, "error",
            "unexpected error-level event: {}",
            event.message
        );
    }

    // Stage order and timeline markers (matched case-insensitively downstream).
    let mut previous_index: Option<usize> = None;
    for marker in [
        "sensor integrations completed",
        "reference FFT: f_ref",
        "signal means for channels",
        "lock-in processing completed",
        "phase analysis completed",
        "Moke analysis completed",
    ] {
        let index = run.single_event_index(marker);
        if let Some(previous_index) = previous_index {
            assert!(
                previous_index < index,
                "stage order violated at {marker:?}: {previous_index} !< {index}"
            );
        }
        previous_index = Some(index);
    }
    let lowered = messages(&run).join("\n").to_ascii_lowercase();
    for marker in [
        "fetched data",
        "reference plot completed",
        "sensor integrations completed",
        "lock-in processing completed",
        "signal means",
        "phase analysis completed",
        "moke analysis completed",
    ] {
        assert!(
            lowered.contains(marker),
            "monitor timeline marker {marker:?} has no matching event"
        );
    }

    // Truthful timings: the signal line attributes compute vs save instead
    // of folding everything into one save duration.
    let signal = run
        .events
        .iter()
        .find(|event| event.message.contains("signal means for channels"))
        .expect("signal completion event is missing");
    assert_eq!(signal.level, "success");
    assert_eq!(signal.kind, "save");
    assert!(
        signal
            .message
            .contains("signal means for channels [4] (compute "),
        "signal line lost its compute attribution: {}",
        signal.message
    );
    assert!(
        signal.message.contains(", save "),
        "signal line lost its save attribution: {}",
        signal.message
    );

    // Phase and MOKE completions now carry their elapsed compute+save time.
    for marker in [
        "phase-rotated results for channels",
        "Moke analysis results for channels",
    ] {
        let event = run
            .events
            .iter()
            .find(|event| event.message.contains(marker))
            .unwrap_or_else(|| panic!("{marker:?} event is missing"));
        assert_eq!(event.level, "success", "{}", event.message);
        assert_eq!(event.kind, "save", "{}", event.message);
        assert!(
            event.message.contains("] (") && event.message.ends_with(')'),
            "{marker:?} line carries no duration: {}",
            event.message
        );
    }

    // Lock-in progress completion reports its measured wall-clock duration
    // in the event payload the TUI renders next to the message.
    let lockin = run
        .events
        .iter()
        .find(|event| event.message.contains("lock-in processing completed"))
        .expect("lock-in completion event is missing");
    assert_eq!(lockin.level, "success");
    assert!(
        lockin.has_duration_ms,
        "lock-in completion carries no duration_ms: {}",
        lockin.message
    );
}
