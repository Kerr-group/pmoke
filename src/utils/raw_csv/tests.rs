use super::*;
use crate::utils::csv::write_csv;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn streamed_raw_csv_matches_existing_csv_formatting() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();

    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 1, 0, 2, 0]).unwrap();
    fs::write(dir.join("ch2.u16le"), [10_u8, 0, 20, 0, 30, 0]).unwrap();

    let expected = dir.join("expected.csv");
    let actual = dir.join("actual.csv");
    let t = vec![-1.0, -0.5, 0.0];
    let ch1 = vec![-0.75, -0.5, -0.25];
    let ch2 = vec![1.0, 2.25, 3.5];
    write_csv(&expected, &["time (s)", "ch1", "ch2"], &[&t, &ch1, &ch2]).unwrap();

    write_raw_csv(
        &actual,
        &["time (s)", "ch1", "ch2"],
        &dir,
        &[
            channel("ch1.u16le", 3, -1.0, 0.25, 1.0, 2.0),
            channel("ch2.u16le", 3, -1.0, 0.125, -2.0, 4.0),
        ],
    )
    .unwrap();

    assert_eq!(fs::read(actual).unwrap(), fs::read(expected).unwrap());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_matches_across_chunk_boundaries_with_x_reference() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let words = [10_u16, 20, 30, 40, 50];
    let raw = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(dir.join("ch1.u16le"), raw).unwrap();

    let expected = dir.join("expected.csv");
    let actual = dir.join("actual.csv");
    let x_increment = 0.25;
    let x_origin = -1.0;
    let x_reference = 2.0;
    let y_increment = 0.125;
    let y_origin = 3.0;
    let y_reference = 7.0;
    let t = (0..words.len())
        .map(|idx| x_origin + (idx as f64 - x_reference) * x_increment)
        .collect::<Vec<_>>();
    let voltage = words
        .iter()
        .map(|&word| (word as f64 - y_origin - y_reference) * y_increment)
        .collect::<Vec<_>>();
    write_csv(&expected, &["time (s)", "ch1"], &[&t, &voltage]).unwrap();

    write_raw_csv_with_chunk_samples(
        &actual,
        &["time (s)", "ch1"],
        &dir,
        &[RawCsvChannel {
            file: "ch1.u16le",
            sample_count: words.len(),
            x_increment,
            x_origin,
            x_reference,
            y_increment,
            y_origin,
            y_reference,
        }],
        2,
    )
    .unwrap();

    assert_eq!(fs::read(actual).unwrap(), fs::read(expected).unwrap());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_rejects_invalid_shape_before_creating_output() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let output = dir.join("actual.csv");

    let empty_error = write_raw_csv(&output, &["time (s)"], &dir, &[]).unwrap_err();
    assert!(empty_error.to_string().contains("no channels"));
    assert!(!output.exists());

    let header_error = write_raw_csv(
        &output,
        &["time (s)"],
        &dir,
        &[channel("ch1.u16le", 1, 0.0, 1.0, 0.0, 0.0)],
    )
    .unwrap_err();
    assert!(header_error.to_string().contains("header len"));
    assert!(!output.exists());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_accepts_saturated_constant_words() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("ch1.u16le"),
        [0xff_u8, 0xff, 0xff, 0xff, 0xff, 0xff],
    )
    .unwrap();

    let actual = dir.join("actual.csv");
    write_raw_csv(
        &actual,
        &["time (s)", "ch1"],
        &dir,
        &[channel("ch1.u16le", 3, 0.0, 0.5, 0.0, 0.0)],
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(actual).unwrap(),
        "time (s),ch1\n0,32767.5\n0.5,32767.5\n1,32767.5\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_matches_existing_dho_scaling_precision() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let words = [32_000_u16, 32_768, 45_000];
    let raw = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(dir.join("ch1.u16le"), raw).unwrap();

    let expected = dir.join("expected.csv");
    let actual = dir.join("actual.csv");
    let x_increment = 5.0e-10;
    let x_origin = -3.0e-2;
    let x_reference = 0.0;
    let y_increment = 2.693_333e-5;
    let y_origin = 9_600.0;
    let y_reference = 32_768.0;
    let t = (0..words.len())
        .map(|idx| x_origin + (idx as f64 - x_reference) * x_increment)
        .collect::<Vec<_>>();
    let voltage = words
        .iter()
        .map(|&word| (word as f64 - y_origin - y_reference) * y_increment)
        .collect::<Vec<_>>();
    write_csv(&expected, &["time (s)", "ch1"], &[&t, &voltage]).unwrap();

    write_raw_csv(
        &actual,
        &["time (s)", "ch1"],
        &dir,
        &[RawCsvChannel {
            file: "ch1.u16le",
            sample_count: words.len(),
            x_increment,
            x_origin,
            x_reference,
            y_increment,
            y_origin,
            y_reference,
        }],
    )
    .unwrap();

    assert_eq!(fs::read(actual).unwrap(), fs::read(expected).unwrap());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_preserves_requested_channel_order() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 1, 0]).unwrap();
    fs::write(dir.join("ch2.u16le"), [10_u8, 0, 11, 0]).unwrap();

    let expected = dir.join("expected.csv");
    let actual = dir.join("actual.csv");
    let t = vec![0.0, 0.5];
    let ch2 = vec![20.0, 22.0];
    let ch1 = vec![0.0, 1.0];
    write_csv(&expected, &["time (s)", "ch2", "ch1"], &[&t, &ch2, &ch1]).unwrap();

    write_raw_csv(
        &actual,
        &["time (s)", "ch2", "ch1"],
        &dir,
        &[
            channel("ch2.u16le", 2, 0.0, 2.0, 0.0, 0.0),
            channel("ch1.u16le", 2, 0.0, 1.0, 0.0, 0.0),
        ],
    )
    .unwrap();

    assert_eq!(fs::read(actual).unwrap(), fs::read(expected).unwrap());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_rejects_timebase_mismatch() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 1, 0]).unwrap();
    fs::write(dir.join("ch2.u16le"), [0_u8, 0, 1, 0]).unwrap();

    let mut ch2 = channel("ch2.u16le", 2, 0.0, 1.0, 0.0, 0.0);
    ch2.x_increment = 0.25;
    let error = write_raw_csv(
        &dir.join("actual.csv"),
        &["time (s)", "ch1", "ch2"],
        &dir,
        &[channel("ch1.u16le", 2, 0.0, 1.0, 0.0, 0.0), ch2],
    )
    .unwrap_err();

    assert!(error.to_string().contains("x_increment"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_rejects_raw_file_size_mismatch() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 1, 0]).unwrap();

    let error = write_raw_csv(
        &dir.join("actual.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[channel("ch1.u16le", 1, 0.0, 1.0, 0.0, 0.0)],
    )
    .unwrap_err();

    assert!(error.to_string().contains("raw channel file size mismatch"));
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn streamed_raw_csv_rejects_symbolic_link() {
    use std::os::unix::fs::symlink;

    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let outside = dir.with_extension("outside.u16le");
    fs::write(&outside, [0_u8, 0]).unwrap();
    symlink(&outside, dir.join("ch1.u16le")).unwrap();

    let error = write_raw_csv(
        &dir.join("actual.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[channel("ch1.u16le", 1, 0.0, 1.0, 0.0, 0.0)],
    )
    .unwrap_err();

    assert!(error.to_string().contains("must be a regular file"));
    fs::remove_dir_all(dir).unwrap();
    fs::remove_file(outside).unwrap();
}

#[test]
fn streamed_raw_csv_rejects_non_finite_scaling() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0]).unwrap();
    let mut invalid = channel("ch1.u16le", 1, 0.0, 1.0, 0.0, 0.0);
    invalid.y_increment = f64::NAN;

    let error = write_raw_csv(
        &dir.join("actual.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[invalid],
    )
    .unwrap_err();

    assert!(error.to_string().contains("y_increment=NaN"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streamed_raw_csv_rejects_invalid_time_and_degenerate_voltage_mapping() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();

    let error = write_raw_csv(
        &dir.join("zero.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[channel("ch1.u16le", 0, 0.0, 1.0, 0.0, 0.0)],
    )
    .unwrap_err();
    assert!(error.to_string().contains("sample_count must be positive"));

    let mut invalid_time = channel("ch1.u16le", 2, f64::MAX, 1.0, 0.0, 0.0);
    invalid_time.x_increment = f64::MAX;
    let error = write_raw_csv(
        &dir.join("time.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[invalid_time],
    )
    .unwrap_err();
    assert!(error.to_string().contains("non-finite time"));

    let invalid_spacing = channel("ch1.u16le", 2, f64::MAX, 1.0, 0.0, 0.0);
    let error = write_raw_csv(
        &dir.join("spacing.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[invalid_spacing],
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not advance"));

    let mut invalid_voltage = channel("ch1.u16le", 1, 0.0, f64::MAX, 0.0, 0.0);
    invalid_voltage.x_increment = 0.5;
    let error = write_raw_csv(
        &dir.join("voltage.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[invalid_voltage],
    )
    .unwrap_err();
    assert!(error.to_string().contains("non-finite voltage"));

    let error = validate_voltage_range(channel("ch1.u16le", 1, 0.0, 0.0, 0.0, 0.0), 0).unwrap_err();
    assert!(error.to_string().contains("y_increment must be positive"));

    let mut rounded_voltage = channel("ch1.u16le", 1, 0.0, 1.0, 0.0, 0.0);
    rounded_voltage.y_origin = f64::MAX / 4.0;
    let error = validate_voltage_range(rounded_voltage, 0).unwrap_err();
    assert!(error.to_string().contains("does not distinguish"));

    let error = write_raw_csv(
        &dir.join("outside.csv"),
        &["time (s)", "ch1"],
        &dir,
        &[channel("../outside.u16le", 1, 0.0, 1.0, 0.0, 0.0)],
    )
    .unwrap_err();
    assert!(error.to_string().contains("safe relative path"));

    fs::remove_dir_all(dir).unwrap();
}

fn channel(
    file: &'static str,
    sample_count: usize,
    x_origin: f64,
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
) -> RawCsvChannel<'static> {
    RawCsvChannel {
        file,
        sample_count,
        x_increment: 0.5,
        x_origin,
        x_reference: 0.0,
        y_increment,
        y_origin,
        y_reference,
    }
}

fn unique_test_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "pmoke_raw_csv_test_{}_{}_{}",
        std::process::id(),
        nanos,
        sequence
    ))
}
