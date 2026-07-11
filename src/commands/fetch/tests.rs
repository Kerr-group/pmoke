use super::*;
use instruments::rigol::dho5108::DhoWaveformPreamble;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn raw_temp_dir_uses_hidden_sibling_directory() {
    assert_eq!(
        raw_temp_dir(Path::new("shot_001")).unwrap(),
        PathBuf::from(".shot_001.tmp")
    );
    assert_eq!(
        raw_temp_dir(Path::new("data/shot_001")).unwrap(),
        PathBuf::from("data/.shot_001.tmp")
    );
}

#[cfg(unix)]
#[test]
fn temp_paths_preserve_non_utf8_file_name_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let output = PathBuf::from(OsString::from_vec(vec![b'd', b'a', b't', b'a', 0xff]));
    let expected = b".data\xff.tmp";

    assert_eq!(
        output_temp_file(&output).unwrap().as_os_str().as_bytes(),
        expected
    );
    assert_eq!(
        raw_temp_dir(&output).unwrap().as_os_str().as_bytes(),
        expected
    );
}

#[test]
fn write_raw_channel_preserves_saturated_constant_word_bytes() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();

    let raw_bytes = vec![0xff, 0xff, 0xff, 0xff];
    let raw = DhoRawWaveform {
        preamble: DhoWaveformPreamble {
            raw: "0,0,0,0,5e-10,-0.03,0,0.001,0,32768".to_string(),
            x_increment: 5.0e-10,
            x_origin: -0.03,
            x_reference: 0.0,
            y_increment: 0.001,
            y_origin: 0.0,
            y_reference: 32768.0,
            vertical_offset: 0.0,
            vertical_scale: 0.1,
        },
        data: raw_bytes.clone(),
    };

    let metadata = write_raw_channel(&dir, 1, 2, raw).unwrap();

    assert_eq!(metadata.file, "ch1.u16le");
    assert_eq!(metadata.sample_count, 2);
    assert_eq!(metadata.y_reference, 32768.0);
    assert_eq!(fs::read(dir.join("ch1.u16le")).unwrap(), raw_bytes);
    assert!(!dir.join("ch1.u16le.tmp").exists());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_word_byte_count_requires_exact_word_payload() {
    validate_raw_word_byte_count(2, 4, 2).unwrap();

    let odd = validate_raw_word_byte_count(2, 5, 2).unwrap_err();
    assert!(odd.to_string().contains("returned 5 raw WORD bytes"));

    let short = validate_raw_word_byte_count(2, 2, 2).unwrap_err();
    assert!(short.to_string().contains("expected 4 bytes"));
}

#[test]
fn fetch_scaling_rejects_invalid_time_and_degenerate_voltage_mapping() {
    let zero_samples = validate_fetch_time_axis_geometry(
        RawTimeAxis {
            sample_count: 0,
            x_increment: 0.5,
            x_origin: 0.0,
            x_reference: 0.0,
        },
        1,
    )
    .unwrap_err();
    assert!(
        zero_samples
            .to_string()
            .contains("sample_count must be positive")
    );

    let zero_increment = validate_fetch_time_axis_geometry(
        RawTimeAxis {
            sample_count: 1,
            x_increment: 0.0,
            x_origin: 0.0,
            x_reference: 0.0,
        },
        1,
    )
    .unwrap_err();
    assert!(
        zero_increment
            .to_string()
            .contains("x_increment must be positive")
    );

    let time_overflow = validate_fetch_time_axis_geometry(
        RawTimeAxis {
            sample_count: 2,
            x_increment: f64::MAX,
            x_origin: f64::MAX,
            x_reference: 0.0,
        },
        1,
    )
    .unwrap_err();
    assert!(time_overflow.to_string().contains("non-finite time"));

    let rounded_to_zero = validate_fetch_time_axis_geometry(
        RawTimeAxis {
            sample_count: 2,
            x_increment: 1.0,
            x_origin: f64::MAX,
            x_reference: 0.0,
        },
        1,
    )
    .unwrap_err();
    assert!(rounded_to_zero.to_string().contains("does not advance"));

    let voltage_overflow = validate_fetch_voltage_range(f64::MAX, 0.0, 0.0, 1).unwrap_err();
    assert!(voltage_overflow.to_string().contains("non-finite voltage"));

    let zero_increment = validate_fetch_voltage_range(0.0, 0.0, 0.0, 1).unwrap_err();
    assert!(
        zero_increment
            .to_string()
            .contains("y_increment must be positive")
    );

    let rounded_voltage = validate_fetch_voltage_range(1.0, f64::MAX / 4.0, 0.0, 1).unwrap_err();
    assert!(rounded_voltage.to_string().contains("does not distinguish"));
}

#[test]
fn raw_metadata_serializes_horizontal_settings() {
    let mut metadata = RawFetchMetadata {
        version: RAW_METADATA_VERSION,
        created_at_unix_seconds: 0,
        config_version: 3,
        oscilloscope: RawOscilloscopeMetadata {
            model: "DHO5108".to_string(),
            connection: Connection::Tcpip {
                ip: "192.168.10.100".to_string(),
                port: 55255,
            },
            memory_depth: 200_000_000,
            waveform_mode: "RAW",
            waveform_format: "WORD",
            byte_order: "little-endian",
            sample_count: 200_000_000,
            channels: vec![1, 2, 3, 4],
            horizontal_offset: -0.03,
            horizontal_scale: 0.005,
        },
        channels: BTreeMap::new(),
    };
    metadata.channels.insert(
        "ch1".to_string(),
        RawChannelMetadata {
            file: "ch1.u16le".to_string(),
            sample_count: 200_000_000,
            preamble_raw: "1,2,200000000,1,5.0E-10,-3.0E-02,0,0.000027,9600,32768".to_string(),
            x_increment: 5.0e-10,
            x_origin: -0.03,
            x_reference: 0.125,
            y_increment: 2.693_333e-5,
            y_origin: 9_600.0,
            y_reference: 32_768.0,
            vertical_offset: 0.258_56,
            vertical_scale: 0.202,
        },
    );

    let encoded = toml::to_string_pretty(&metadata).unwrap();
    let decoded: toml::Value = toml::from_str(&encoded).unwrap();

    assert!(encoded.contains("horizontal_offset = -0.03"));
    assert!(encoded.contains("horizontal_scale = 0.005"));
    for (path, expected) in [
        (
            &["oscilloscope", "horizontal_offset"][..],
            metadata.oscilloscope.horizontal_offset,
        ),
        (
            &["oscilloscope", "horizontal_scale"][..],
            metadata.oscilloscope.horizontal_scale,
        ),
        (
            &["channels", "ch1", "x_increment"][..],
            metadata.channels["ch1"].x_increment,
        ),
        (
            &["channels", "ch1", "x_origin"][..],
            metadata.channels["ch1"].x_origin,
        ),
        (
            &["channels", "ch1", "x_reference"][..],
            metadata.channels["ch1"].x_reference,
        ),
        (
            &["channels", "ch1", "y_increment"][..],
            metadata.channels["ch1"].y_increment,
        ),
        (
            &["channels", "ch1", "y_origin"][..],
            metadata.channels["ch1"].y_origin,
        ),
        (
            &["channels", "ch1", "y_reference"][..],
            metadata.channels["ch1"].y_reference,
        ),
        (
            &["channels", "ch1", "vertical_offset"][..],
            metadata.channels["ch1"].vertical_offset,
        ),
        (
            &["channels", "ch1", "vertical_scale"][..],
            metadata.channels["ch1"].vertical_scale,
        ),
    ] {
        let actual = path
            .iter()
            .fold(&decoded, |value, key| &value[*key])
            .as_float()
            .unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits(), "path={path:?}");
    }
    assert_eq!(
        decoded["channels"]["ch1"]["preamble_raw"].as_str(),
        Some(metadata.channels["ch1"].preamble_raw.as_str())
    );
}

#[test]
fn csv_and_raw_outputs_stay_staged_on_csv_stream_error() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let staging_dir = dir.join(".raw_waveform.tmp");
    fs::create_dir(&staging_dir).unwrap();

    let metadata = single_channel_raw_metadata("missing.u16le", 1);
    let csv_out = dir.join("actual.csv");
    let raw_out = dir.join("raw_waveform");
    let tmp_csv = output_temp_file(&csv_out).unwrap();
    let error =
        write_raw_csv_and_finalize_outputs(&csv_out, &raw_out, &staging_dir, &[1], &metadata)
            .unwrap_err();

    let error = format!("{error:#}");
    assert!(error.contains("failed to write csv output"));
    assert!(error.contains("staging directory was preserved"));
    assert!(staging_dir.exists());
    assert!(!raw_out.exists());
    assert!(!csv_out.exists());
    assert!(!tmp_csv.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn csv_and_raw_outputs_are_finalized_after_streaming_succeeds() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let staging_dir = dir.join(".raw_waveform.tmp");
    fs::create_dir(&staging_dir).unwrap();
    fs::write(staging_dir.join("ch1.u16le"), [2_u8, 0, 4, 0]).unwrap();
    fs::write(staging_dir.join(RAW_METADATA_FNAME), "version = 1\n").unwrap();

    let csv_out = dir.join("actual.csv");
    let raw_out = dir.join("raw_waveform");
    let metadata = single_channel_raw_metadata("ch1.u16le", 2);
    write_raw_csv_and_finalize_outputs(&csv_out, &raw_out, &staging_dir, &[1], &metadata).unwrap();

    assert!(!staging_dir.exists());
    assert_eq!(fs::read(raw_out.join("ch1.u16le")).unwrap(), [2, 0, 4, 0]);
    assert_eq!(
        fs::read_to_string(raw_out.join(RAW_METADATA_FNAME)).unwrap(),
        "version = 1\n"
    );
    assert_eq!(
        fs::read_to_string(csv_out).unwrap(),
        "time (s),ch1\n0,2\n0.5,4\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn csv_finalize_error_restores_raw_staging_directory() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let staging_dir = dir.join(".raw_waveform.tmp");
    fs::create_dir(&staging_dir).unwrap();
    fs::write(staging_dir.join("ch1.u16le"), [2_u8, 0, 4, 0]).unwrap();

    let csv_out = dir.join("actual.csv");
    fs::create_dir(&csv_out).unwrap();
    let raw_out = dir.join("raw_waveform");
    let tmp_csv = output_temp_file(&csv_out).unwrap();
    let metadata = single_channel_raw_metadata("ch1.u16le", 2);

    let error =
        write_raw_csv_and_finalize_outputs(&csv_out, &raw_out, &staging_dir, &[1], &metadata)
            .unwrap_err();

    assert!(
        format!("{error:#}").contains("staging directory was restored"),
        "{error:#}"
    );
    assert!(staging_dir.is_dir());
    assert_eq!(
        fs::read(staging_dir.join("ch1.u16le")).unwrap(),
        [2, 0, 4, 0]
    );
    assert!(!raw_out.exists());
    assert!(csv_out.is_dir());
    assert!(!tmp_csv.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn late_raw_output_collision_preserves_staging_and_existing_output() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let staging_dir = dir.join(".raw_waveform.tmp");
    fs::create_dir(&staging_dir).unwrap();
    fs::write(staging_dir.join("ch1.u16le"), [2_u8, 0, 4, 0]).unwrap();

    let csv_out = dir.join("actual.csv");
    let raw_out = dir.join("raw_waveform");
    fs::create_dir(&raw_out).unwrap();
    let metadata = single_channel_raw_metadata("ch1.u16le", 2);

    let error =
        write_raw_csv_and_finalize_outputs(&csv_out, &raw_out, &staging_dir, &[1], &metadata)
            .unwrap_err();

    let error = format!("{error:#}");
    assert!(error.contains("output already exists"));
    assert!(error.contains("staging directory was preserved"));
    assert!(staging_dir.is_dir());
    assert_eq!(
        fs::read(staging_dir.join("ch1.u16le")).unwrap(),
        [2, 0, 4, 0]
    );
    assert!(raw_out.is_dir());
    assert!(!csv_out.exists());
    assert!(!output_temp_file(&csv_out).unwrap().exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_finalize_error_preserves_staging_and_removes_temporary_csv() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let staging_dir = dir.join(".raw_waveform.tmp");
    fs::create_dir(&staging_dir).unwrap();
    fs::write(staging_dir.join("ch1.u16le"), [2_u8, 0, 4, 0]).unwrap();

    let csv_out = dir.join("actual.csv");
    let raw_out = dir.join("raw_waveform");
    fs::create_dir(&raw_out).unwrap();
    fs::write(raw_out.join("existing.txt"), "do not replace").unwrap();
    let tmp_csv = output_temp_file(&csv_out).unwrap();
    let metadata = single_channel_raw_metadata("ch1.u16le", 2);

    let error =
        write_raw_csv_and_finalize_outputs(&csv_out, &raw_out, &staging_dir, &[1], &metadata)
            .unwrap_err();

    let error = format!("{error:#}");
    assert!(error.contains("output already exists"), "{error}");
    assert!(error.contains("staging directory was preserved"), "{error}");
    assert!(staging_dir.is_dir());
    assert_eq!(
        fs::read(staging_dir.join("ch1.u16le")).unwrap(),
        [2, 0, 4, 0]
    );
    assert_eq!(
        fs::read_to_string(raw_out.join("existing.txt")).unwrap(),
        "do not replace"
    );
    assert!(!csv_out.exists());
    assert!(!tmp_csv.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ensure_output_parent_creates_missing_parent_directories() {
    let dir = unique_test_dir();
    let output = dir.join("nested").join("raw.csv");

    ensure_output_parent(&output).unwrap();

    assert!(dir.join("nested").is_dir());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn csv_late_output_collision_preserves_existing_file_and_removes_temp() {
    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let output = dir.join("actual.csv");
    fs::write(&output, "existing\n").unwrap();
    let tmp = output_temp_file(&output).unwrap();

    let error = write_csv_atomic(&output, &["value"], &[&[1.0, 2.0]]).unwrap_err();

    assert!(error.to_string().contains("output already exists"));
    assert_eq!(fs::read_to_string(output).unwrap(), "existing\n");
    assert!(!tmp.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn ensure_path_not_exists_rejects_dangling_symbolic_link() {
    use std::os::unix::fs::symlink;

    let dir = unique_test_dir();
    fs::create_dir(&dir).unwrap();
    let target = dir.join("missing-target.csv");
    let output = dir.join("output.csv");
    symlink(&target, &output).unwrap();

    let error = ensure_path_not_exists(&output).unwrap_err();

    assert!(error.to_string().contains("output already exists"));
    assert!(!target.exists());
    fs::remove_dir_all(dir).unwrap();
}

fn single_channel_raw_metadata(file: &str, sample_count: usize) -> RawFetchMetadata {
    let channels = BTreeMap::from([(
        "ch1".to_string(),
        RawChannelMetadata {
            file: file.to_string(),
            sample_count,
            preamble_raw: "preamble ch1".to_string(),
            x_increment: 0.5,
            x_origin: 0.0,
            x_reference: 0.0,
            y_increment: 1.0,
            y_origin: 0.0,
            y_reference: 0.0,
            vertical_offset: 0.0,
            vertical_scale: 0.1,
        },
    )]);
    RawFetchMetadata {
        version: RAW_METADATA_VERSION,
        created_at_unix_seconds: 0,
        config_version: 3,
        oscilloscope: RawOscilloscopeMetadata {
            model: "DHO5108".to_string(),
            connection: Connection::Tcpip {
                ip: "192.168.10.100".to_string(),
                port: 55255,
            },
            memory_depth: sample_count,
            waveform_mode: "RAW",
            waveform_format: "WORD",
            byte_order: "little-endian",
            sample_count,
            channels: vec![1],
            horizontal_offset: 0.0,
            horizontal_scale: 1.0,
        },
        channels,
    }
}

fn unique_test_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "pmoke_raw_fetch_test_{}_{}_{}",
        std::process::id(),
        nanos,
        sequence
    ))
}
