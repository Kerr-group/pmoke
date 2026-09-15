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
