use super::*;
use std::fs;
use std::path::PathBuf;

fn unique_test_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pmoke_noise_source_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn raw_fixture_dir(name: &str) -> PathBuf {
    let dir = unique_test_dir(name);
    // Two samples: words [0, 1] on ch3, words [10, 11] on ch1.
    fs::write(dir.join("ch3.u16le"), [0_u8, 0, 1, 0]).unwrap();
    fs::write(dir.join("ch1.u16le"), [10_u8, 0, 11, 0]).unwrap();
    fs::write(
        dir.join("manifest.toml"),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch3]
file = "ch3.u16le"
sample_count = 2
x_increment = 0.5
x_origin = -1.0
x_reference = 2.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0

[channels.ch1]
file = "ch1.u16le"
sample_count = 2
x_increment = 0.5
x_origin = -1.0
x_reference = 2.0
y_increment = 2.0
y_origin = 0.0
y_reference = 0.0
"#,
    )
    .unwrap();
    dir
}

fn raw_request(path: &str) -> RecordedSourceRequest {
    RecordedSourceRequest {
        kind: RecordedSourceKind::RecordedRaw,
        path: path.to_owned(),
        channels: ChannelBinding {
            detector: 3,
            reference: Some(1),
            witness: None,
        },
        grid: GridBinding { stride: 100 },
    }
}

#[test]
fn raw_and_csv_routes_decode_identical_detector_blocks() {
    let dir = raw_fixture_dir("parity");
    let parent = dir.parent().unwrap().to_path_buf();
    let name = dir.file_name().unwrap().to_str().unwrap().to_owned();

    // RAW route: detector ch3 words [0,1] -> [0.0, 1.0]; reference ch1
    // words [10,11] with y_increment 2.0 -> [20.0, 22.0].
    let source = RecordedSource::open(&parent, &raw_request(&name)).unwrap();
    assert_eq!(source.sample_count(), 2);
    let block = source.read_block(0, 2).unwrap();
    assert_eq!(block.detector, vec![0.0, 1.0]);
    assert_eq!(block.reference, Some(vec![20.0, 22.0]));
    // Legacy interpolated-endpoint timebase: t = x_origin + (i - x_ref) * dx.
    assert_eq!(block.times, vec![-2.0, -1.5]);
    assert_eq!(source.frozen_view().schema_version, 1);

    // CSV route over the same voltages: identical decoded values.
    let csv_path = dir.join("wave.csv");
    fs::write(
        &csv_path,
        "time (s),ch3,ch1\n-2.0,0.0,20.0\n-1.5,1.0,22.0\n",
    )
    .unwrap();
    let csv_request = RecordedSourceRequest {
        kind: RecordedSourceKind::RecordedCsv,
        path: format!("{name}/wave.csv"),
        channels: ChannelBinding {
            detector: 3,
            reference: Some(1),
            witness: None,
        },
        grid: GridBinding { stride: 100 },
    };
    let csv = RecordedSource::open(&parent, &csv_request).unwrap();
    let csv_block = csv.read_block(0, 2).unwrap();
    assert_eq!(csv_block.detector, block.detector);
    assert_eq!(csv_block.reference, block.reference);
    assert_eq!(csv_block.times, block.times);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn channel_conflicts_are_rejected_without_substitution() {
    let dir = raw_fixture_dir("conflict");
    let parent = dir.parent().unwrap().to_path_buf();
    let name = dir.file_name().unwrap().to_str().unwrap().to_owned();

    // Reference equal to detector.
    let mut request = raw_request(&name);
    request.channels.reference = Some(3);
    assert!(RecordedSource::open(&parent, &request).is_err());

    // Unknown detector channel.
    let mut request = raw_request(&name);
    request.channels.detector = 7;
    assert!(RecordedSource::open(&parent, &request).is_err());

    // Zero stride grid.
    let mut request = raw_request(&name);
    request.grid.stride = 0;
    assert!(RecordedSource::open(&parent, &request).is_err());

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn source_mutation_after_freeze_is_a_hard_error() {
    let dir = raw_fixture_dir("mutation");
    let parent = dir.parent().unwrap().to_path_buf();
    let name = dir.file_name().unwrap().to_str().unwrap().to_owned();
    let source = RecordedSource::open(&parent, &raw_request(&name)).unwrap();

    // Mutate one waveform byte behind the frozen view.
    fs::write(dir.join("ch3.u16le"), [9_u8, 0, 1, 0]).unwrap();
    let error = source.read_block(0, 2).unwrap_err();
    assert!(
        error.to_string().contains("source_changed"),
        "unexpected error: {error:#}"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn symlinked_sources_are_rejected() {
    let dir = raw_fixture_dir("symlink");
    let parent = dir.parent().unwrap().to_path_buf();
    let name = dir.file_name().unwrap().to_str().unwrap().to_owned();
    let link = parent.join(format!("{name}-link"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    #[cfg(not(unix))]
    fs::create_dir(&link).unwrap();

    #[cfg(unix)]
    {
        let request = raw_request(&format!("{name}-link"));
        let error = RecordedSource::open(&parent, &request)
            .map(|_| ())
            .unwrap_err();
        assert!(
            error.to_string().contains("symlink"),
            "unexpected error: {error:#}"
        );
        fs::remove_file(&link).unwrap();
    }

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn oversized_reads_are_bounded() {
    let dir = raw_fixture_dir("bounded");
    let parent = dir.parent().unwrap().to_path_buf();
    let name = dir.file_name().unwrap().to_str().unwrap().to_owned();
    let source = RecordedSource::open(&parent, &raw_request(&name)).unwrap();
    let error = source.read_block(0, 3).unwrap_err();
    assert!(error.to_string().contains("exceeds recorded length"));
    fs::remove_dir_all(&dir).unwrap();
}

/// RAW fixture whose ch3 words sit on both rail codes plus in-range codes.
fn rail_fixture_dir(name: &str) -> PathBuf {
    let dir = unique_test_dir(name);
    let mut ch3 = Vec::new();
    for word in [0_u16, 65_535, 5, 10] {
        ch3.extend_from_slice(&word.to_le_bytes());
    }
    let mut ch1 = Vec::new();
    for word in [7_u16, 0, 65_535, 9] {
        ch1.extend_from_slice(&word.to_le_bytes());
    }
    fs::write(dir.join("ch3.u16le"), ch3).unwrap();
    fs::write(dir.join("ch1.u16le"), ch1).unwrap();
    fs::write(
        dir.join("manifest.toml"),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch3]
file = "ch3.u16le"
sample_count = 4
x_increment = 0.5
x_origin = -1.0
x_reference = 2.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0

[channels.ch1]
file = "ch1.u16le"
sample_count = 4
x_increment = 0.5
x_origin = -1.0
x_reference = 2.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0
"#,
    )
    .unwrap();
    dir
}

#[test]
fn raw_rail_codes_are_reported_without_repairing_values() {
    // PN-AT-001 rail control: saturating rail codes are reported as
    // evidence and the decoded values are preserved bit-for-bit (raw
    // codes are never clipped, replaced or repaired).
    let dir = rail_fixture_dir("rails");
    let parent = dir.parent().unwrap().to_path_buf();
    let name = dir.file_name().unwrap().to_str().unwrap().to_owned();

    let source = RecordedSource::open(&parent, &raw_request(&name)).unwrap();
    let block = source.read_block(0, 4).unwrap();
    // y_increment 1.0 with zero origin/reference: value == raw code.
    assert_eq!(block.detector, vec![0.0, 65_535.0, 5.0, 10.0]);
    assert_eq!(
        block.detector_rails,
        Some(RailCounts { low: 1, high: 1 }),
        "one sample per rail code must be reported"
    );
    assert_eq!(
        block.reference_rails,
        Some(RailCounts { low: 1, high: 1 }),
        "reference rail codes are reported too"
    );
    assert_eq!(RailCounts { low: 1, high: 1 }.total(), 2);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn csv_sources_report_no_rail_codes() {
    // CSV carries decoded voltages and no ADC codes: rail evidence is
    // explicitly absent, never invented.
    let dir = unique_test_dir("csv_rails");
    fs::write(dir.join("wave.csv"), "time (s),ch3\n0.0,1.0\n1.0,2.0\n").unwrap();
    let request = RecordedSourceRequest {
        kind: RecordedSourceKind::RecordedCsv,
        path: "wave.csv".to_owned(),
        channels: ChannelBinding {
            detector: 3,
            reference: None,
            witness: None,
        },
        grid: GridBinding { stride: 100 },
    };
    let source = RecordedSource::open(&dir, &request).unwrap();
    let block = source.read_block(0, 2).unwrap();
    assert_eq!(block.detector_rails, None);
    assert_eq!(block.reference_rails, None);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn csv_non_finite_and_non_monotonic_inputs_are_refused() {
    // PN-AT-001 NaN control: non-finite detector values are a named
    // refusal, never replaced or dropped.
    let dir = unique_test_dir("csv_nan");
    let request = RecordedSourceRequest {
        kind: RecordedSourceKind::RecordedCsv,
        path: "wave.csv".to_owned(),
        channels: ChannelBinding {
            detector: 3,
            reference: None,
            witness: None,
        },
        grid: GridBinding { stride: 100 },
    };
    fs::write(
        dir.join("wave.csv"),
        "time (s),ch3\n0.0,1.0\n1.0e-3,nan\n2.0e-3,3.0\n",
    )
    .unwrap();
    let source = RecordedSource::open(&dir, &request).unwrap();
    let error = source.read_block(0, 3).unwrap_err();
    assert!(
        error.to_string().contains("non-finite"),
        "unexpected error: {error:#}"
    );

    // Duplicated (non-increasing) timestamps are refused as well.
    fs::write(dir.join("wave.csv"), "time (s),ch3\n0.0,1.0\n0.0,2.0\n").unwrap();
    let source = RecordedSource::open(&dir, &request).unwrap();
    let error = source.read_block(0, 2).unwrap_err();
    assert!(
        error.to_string().contains("not strictly increasing"),
        "unexpected error: {error:#}"
    );

    fs::remove_dir_all(&dir).unwrap();
}
