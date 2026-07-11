use super::*;
use std::path::PathBuf;

#[test]
fn raw_word_conversion_matches_dho_formula() {
    let bytes = [0x00, 0x00, 0x01, 0x00, 0x10, 0x00];
    let values = convert_raw_word_to_voltages(&bytes, 0.5, 1.0, 2.0).unwrap();
    assert_eq!(values, vec![-1.5, -1.0, 6.5]);
}

#[test]
fn csv_header_channels_preserve_order() {
    let path = unique_test_path("waveform_header.csv");
    fs::write(&path, "CH3,ch1,ch4\n1,2,3\n").unwrap();

    let channels = csv_columns_from_header(&path)
        .unwrap()
        .channels
        .into_iter()
        .map(|(_, ch)| ch)
        .collect::<Vec<_>>();

    assert_eq!(channels, vec![3, 1, 4]);
    fs::remove_file(path).unwrap();
}

#[test]
fn csv_column_indices_use_actual_header_positions() {
    let path = unique_test_path("waveform_header_with_time.csv");
    fs::write(&path, "time,ch3,ch1,ch4\n0,3,1,4\n").unwrap();

    let columns = csv_columns_from_header(&path).unwrap();

    assert_eq!(columns.time_index, Some(0));
    assert_eq!(columns.channels, vec![(1, 3), (2, 1), (3, 4)]);
    fs::remove_file(path).unwrap();
}

#[test]
fn csv_column_fallback_skips_time_column_when_channels_are_unlabeled() {
    let path = unique_test_path("waveform_unlabeled_with_time.csv");
    fs::write(&path, "time,a,b,c\n0,3,1,4\n").unwrap();

    let columns = csv_columns_from_header(&path).unwrap();
    assert_eq!(columns.time_index, Some(0));
    assert!(columns.channels.is_empty());

    fs::remove_file(path).unwrap();
}

#[test]
fn csv_column_fallback_indices_skip_time_column() {
    let (time_index, column_indices) = resolve_csv_column_indices(
        CsvColumns {
            time_index: Some(0),
            channels: Vec::new(),
            column_count: 4,
        },
        &[1, 2, 3],
    )
    .unwrap();

    assert_eq!(time_index, Some(0));
    assert_eq!(column_indices, vec![1, 2, 3]);
}

#[test]
fn csv_column_fallback_rejects_too_few_data_columns() {
    let error = resolve_csv_column_indices(
        CsvColumns {
            time_index: Some(0),
            channels: Vec::new(),
            column_count: 2,
        },
        &[1, 2],
    )
    .unwrap_err();

    assert!(error.to_string().contains("2 channels were requested"));
}

#[test]
fn read_raw_channel_converts_u16le_with_metadata_scaling() {
    let dir = unique_test_dir("raw_channel");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch4.u16le"), [0x00, 0x00, 0x10, 0x00]).unwrap();

    let metadata = raw_metadata_with_channel(4, "ch4.u16le", 2);
    let mut time_axis = None;
    let spec = raw_channel_spec(&dir, &metadata, 4, &mut time_axis).unwrap();
    let values = read_raw_channel_data(&spec).unwrap();

    assert_eq!(values, vec![-1.5, 6.5]);
    assert_eq!(time_axis.unwrap().build(), vec![1.0, 1.5]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_channel_matches_conversion_across_chunks() {
    let dir = unique_test_dir("raw_chunk_read");
    fs::create_dir(&dir).unwrap();
    let path = dir.join("ch1.u16le");
    let words = [0_u16, 1, 32_767, 32_768, 65_535];
    let bytes = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let spec = RawChannelSpec {
        key: "ch1".to_owned(),
        path,
        expected_bytes: bytes.len(),
        expected_sha256: None,
        y_increment: 0.25,
        y_origin: 2.0,
        y_reference: 32_768.0,
    };

    let chunked = read_raw_channel_data_with_chunk_size(&spec, 4).unwrap();
    let contiguous =
        convert_raw_word_to_voltages(&bytes, spec.y_increment, spec.y_origin, spec.y_reference)
            .unwrap();

    assert_eq!(chunked, contiguous);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_word_conversion_rejects_incomplete_word() {
    let error = convert_raw_word_to_voltages(&[1], 1.0, 0.0, 0.0).unwrap_err();
    assert!(error.to_string().contains("incomplete final sample"));
}

#[test]
fn read_raw_channel_accepts_saturated_constant_words() {
    let dir = unique_test_dir("raw_saturated_channel");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("ch4.u16le"),
        [0xff_u8, 0xff, 0xff, 0xff, 0xff, 0xff],
    )
    .unwrap();

    let metadata = raw_metadata_with_channel(4, "ch4.u16le", 3);
    let spec = raw_channel_spec(&dir, &metadata, 4, &mut None).unwrap();
    let values = read_raw_channel_data(&spec).unwrap();

    assert_eq!(values, vec![32_766.0; 3]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_channel_rejects_file_size_mismatch() {
    let dir = unique_test_dir("raw_size_mismatch");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch2.u16le"), [0x00, 0x00]).unwrap();

    let metadata = raw_metadata_with_channel(2, "ch2.u16le", 2);
    let mut time_axis = None;
    let spec = raw_channel_spec(&dir, &metadata, 2, &mut time_axis).unwrap();
    let error = read_raw_channel_data(&spec).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("raw channel file size mismatch for ch2")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn read_raw_channel_rejects_symbolic_link() {
    use std::os::unix::fs::symlink;

    let dir = unique_test_dir("raw_symlink");
    fs::create_dir(&dir).unwrap();
    let outside = unique_test_path("raw_symlink_target.u16le");
    fs::write(&outside, [0_u8, 0]).unwrap();
    symlink(&outside, dir.join("ch2.u16le")).unwrap();

    let metadata = raw_metadata_with_channel(2, "ch2.u16le", 1);
    let spec = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap();
    let error = read_raw_channel_data(&spec).unwrap_err();

    assert!(error.to_string().contains("must be a regular file"));
    fs::remove_dir_all(dir).unwrap();
    fs::remove_file(outside).unwrap();
}

#[test]
fn read_raw_channel_rejects_non_finite_metadata() {
    let dir = unique_test_dir("raw_non_finite");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch2.u16le"), [0_u8, 0]).unwrap();

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 1);
    metadata.channels.get_mut("ch2").unwrap().y_increment = f64::NAN;
    let mut time_axis = None;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut time_axis).unwrap_err();

    assert!(error.to_string().contains("y_increment=NaN"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_channel_rejects_invalid_time_and_degenerate_voltage_mapping() {
    let dir = unique_test_dir("raw_unusable_finite");
    fs::create_dir(&dir).unwrap();

    let metadata = raw_metadata_with_channel(2, "ch2.u16le", 0);
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("sample_count must be positive"));

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 1);
    metadata.channels.get_mut("ch2").unwrap().x_increment = 0.0;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("x_increment must be positive"));

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 2);
    let channel = metadata.channels.get_mut("ch2").unwrap();
    channel.x_increment = f64::MAX;
    channel.x_origin = f64::MAX;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("non-finite time"));

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 2);
    let channel = metadata.channels.get_mut("ch2").unwrap();
    channel.x_increment = 1.0;
    channel.x_origin = f64::MAX;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("does not advance"));

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 1);
    metadata.channels.get_mut("ch2").unwrap().y_increment = f64::MAX;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("non-finite voltage"));

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 1);
    metadata.channels.get_mut("ch2").unwrap().y_increment = 0.0;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("y_increment must be positive"));

    let mut metadata = raw_metadata_with_channel(2, "ch2.u16le", 1);
    metadata.channels.get_mut("ch2").unwrap().y_origin = f64::MAX / 4.0;
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("does not distinguish"));

    let metadata = raw_metadata_with_channel(2, "../outside.u16le", 1);
    let error = raw_channel_spec(&dir, &metadata, 2, &mut None).unwrap_err();
    assert!(error.to_string().contains("plain file name"));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_channel_rejects_sub_ps_time_increment_mismatch() {
    let dir = unique_test_dir("raw_time_mismatch");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0x00, 0x00, 0x01, 0x00]).unwrap();
    fs::write(dir.join("ch2.u16le"), [0x00, 0x00, 0x01, 0x00]).unwrap();

    let metadata =
        raw_metadata_with_channels([(1, "ch1.u16le", 2, 5.0e-10), (2, "ch2.u16le", 2, 5.01e-10)]);
    let mut time_axis = None;
    raw_channel_spec(&dir, &metadata, 1, &mut time_axis).unwrap();
    let error = raw_channel_spec(&dir, &metadata, 2, &mut time_axis).unwrap_err();

    assert!(error.to_string().contains("x_increment"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_status_missing_only_when_raw_directory_is_absent() {
    let dir = unique_test_dir("raw_missing");

    assert!(matches!(
        raw_status_in_dir(&dir, &[1]).unwrap(),
        RawStatus::Missing
    ));
}

#[cfg(unix)]
#[test]
fn raw_status_treats_dangling_raw_directory_symlink_as_invalid() {
    use std::os::unix::fs::symlink;

    let dir = unique_test_dir("raw_dangling_directory");
    let target = unique_test_dir("raw_dangling_directory_target");
    symlink(&target, &dir).unwrap();

    let status = raw_status_in_dir(&dir, &[1]).unwrap();

    assert!(matches!(
        status,
        RawStatus::Invalid(message) if message.contains("raw metadata not found")
    ));
    fs::remove_file(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn raw_status_rejects_symbolic_link_metadata() {
    use std::os::unix::fs::symlink;

    let dir = unique_test_dir("raw_metadata_symlink");
    fs::create_dir(&dir).unwrap();
    let outside = unique_test_path("raw_metadata_symlink_target.toml");
    fs::write(
        &outside,
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels]
"#,
    )
    .unwrap();
    symlink(&outside, dir.join(RAW_METADATA_FNAME)).unwrap();

    let status = raw_status_in_dir(&dir, &[1]).unwrap();

    assert!(matches!(
        status,
        RawStatus::Invalid(message) if message.contains("metadata must be a regular file")
    ));
    fs::remove_dir_all(dir).unwrap();
    fs::remove_file(outside).unwrap();
}

#[test]
fn raw_status_invalid_when_directory_exists_without_metadata() {
    let dir = unique_test_dir("raw_no_metadata");
    fs::create_dir(&dir).unwrap();

    let status = raw_status_in_dir(&dir, &[1]).unwrap();

    assert!(
        matches!(status, RawStatus::Invalid(message) if message.contains("raw metadata not found"))
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_status_complete_when_metadata_and_requested_files_match() {
    let dir = unique_test_dir("raw_complete");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0x00, 0x00, 0x01, 0x00]).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch1]
file = "ch1.u16le"
sample_count = 2
x_increment = 0.5
x_origin = 1.0
x_reference = 0.0
y_increment = 0.5
y_origin = 1.0
y_reference = 2.0
"#,
    )
    .unwrap();

    assert!(matches!(
        raw_status_in_dir(&dir, &[1]).unwrap(),
        RawStatus::Complete
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_status_rejects_degenerate_voltage_scaling() {
    let dir = unique_test_dir("raw_invalid_scaling");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0]).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch1]
file = "ch1.u16le"
sample_count = 1
x_increment = 0.5
x_origin = 0.0
x_reference = 0.0
y_increment = 0.0
y_origin = 0.0
y_reference = 0.0
"#,
    )
    .unwrap();

    let status = raw_status_in_dir(&dir, &[1]).unwrap();

    assert!(matches!(
        status,
        RawStatus::Invalid(message) if message.contains("y_increment must be positive")
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_status_rejects_channel_path_outside_raw_directory() {
    let dir = unique_test_dir("raw_outside_path");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch1]
file = "../outside.u16le"
sample_count = 1
x_increment = 0.5
x_origin = 0.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0
"#,
    )
    .unwrap();

    let status = raw_status_in_dir(&dir, &[1]).unwrap();

    assert!(matches!(
        status,
        RawStatus::Invalid(message) if message.contains("plain file name")
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_status_rejects_unsupported_metadata_version() {
    let dir = unique_test_dir("raw_unsupported_version");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 3

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels]
"#,
    )
    .unwrap();

    let status = raw_status_in_dir(&dir, &[1]).unwrap();

    assert!(matches!(
        status,
        RawStatus::Invalid(message) if message.contains("unsupported raw metadata version: 3")
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_waveform_preserves_channel_order_and_applies_x_reference() {
    let dir = unique_test_dir("raw_order");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 1, 0]).unwrap();
    fs::write(dir.join("ch2.u16le"), [10_u8, 0, 11, 0]).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch1]
file = "ch1.u16le"
sample_count = 2
x_increment = 0.5
x_origin = -1.0
x_reference = 2.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0

[channels.ch2]
file = "ch2.u16le"
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

    let waveform = read_raw_waveform_channels_from_dir(&dir, &[2, 1]).unwrap();

    assert!(matches!(waveform.t, WaveformTime::Uniform(_)));
    assert_eq!(waveform.t.to_vec(), vec![-2.0, -1.5]);
    assert_eq!(waveform.channels, vec![vec![20.0, 22.0], vec![0.0, 1.0]]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_waveform_v2_verifies_config_and_channel_checksums() {
    let dir = unique_test_dir("raw_v2_checksums");
    fs::create_dir(&dir).unwrap();
    let config = b"version = 4\n";
    let resolved_config = b"version = 4\n# resolved\n";
    let raw = [0_u8, 0, 1, 0];
    fs::write(dir.join("config.source.toml"), config).unwrap();
    fs::write(dir.join("config.resolved.toml"), resolved_config).unwrap();
    fs::write(dir.join("ch1.u16le"), raw).unwrap();
    let config_sha = format!("{:x}", Sha256::digest(config));
    let resolved_config_sha = format!("{:x}", Sha256::digest(resolved_config));
    let raw_sha = format!("{:x}", Sha256::digest(raw));
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        format!(
            r#"version = 2
status = "complete"
pmoke_version = "0.2.0"
created_at = "2026-07-11T00:00:00Z"
config_file = "config.source.toml"
config_sha256 = "{config_sha}"
resolved_config_file = "config.resolved.toml"
resolved_config_sha256 = "{resolved_config_sha}"

[oscilloscope]
idn_raw = "RIGOL,DHO5108,serial,firmware"
waveform_format = "WORD"
byte_order = "little-endian"
memory_depth = 2
sample_count = 2
channels = [1]

[channels.ch1]
file = "ch1.u16le"
bytes = 4
sha256 = "{raw_sha}"
sample_count = 2
x_increment = 0.5
x_origin = 0.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0
"#
        ),
    )
    .unwrap();

    let waveform = read_raw_waveform_channels_from_dir(&dir, &[1]).unwrap();
    assert_eq!(waveform.channels, vec![vec![0.0, 1.0]]);
    assert_eq!(
        verify_raw_waveform_dir(&dir).unwrap(),
        RawVerification {
            metadata_version: 2,
            channel_count: 1,
            sample_count: 2,
            total_bytes: 4,
            checksums_verified: true,
        }
    );

    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 2, 0]).unwrap();
    let error = read_raw_waveform_channels_from_dir(&dir, &[1]).unwrap_err();
    assert!(error.to_string().contains("checksum mismatch"), "{error:#}");

    fs::write(dir.join("ch1.u16le"), raw).unwrap();
    fs::write(dir.join("config.source.toml"), b"version = 3\n").unwrap();
    let error = read_raw_waveform_channels_from_dir(&dir, &[1]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("config snapshot checksum mismatch"),
        "{error:#}"
    );
    fs::write(dir.join("config.source.toml"), config).unwrap();
    fs::write(
        dir.join("config.resolved.toml"),
        b"version = 4\n# changed\n",
    )
    .unwrap();
    let error = read_raw_waveform_channels_from_dir(&dir, &[1]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("config snapshot checksum mismatch"),
        "{error:#}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_metadata_v2_rejects_incomplete_status() {
    let mut metadata = raw_metadata_with_channel(1, "ch1.u16le", 1);
    metadata.version = RAW_METADATA_VERSION;
    metadata.status = Some("writing".to_string());

    let error = validate_raw_format(&metadata).unwrap_err();

    assert!(error.to_string().contains("status = \"complete\""));
}

#[cfg(unix)]
#[test]
fn read_raw_waveform_preflights_all_files_before_loading_channels() {
    use std::os::unix::fs::PermissionsExt;

    let dir = unique_test_dir("raw_preflight");
    fs::create_dir(&dir).unwrap();
    let first_path = dir.join("ch1.u16le");
    fs::write(&first_path, [0_u8, 0]).unwrap();
    fs::set_permissions(&first_path, fs::Permissions::from_mode(0o000)).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch1]
file = "ch1.u16le"
sample_count = 1
x_increment = 0.5
x_origin = 0.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0

[channels.ch2]
file = "missing-ch2.u16le"
sample_count = 1
x_increment = 0.5
x_origin = 0.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0
"#,
    )
    .unwrap();

    let error = read_raw_waveform_channels_from_dir(&dir, &[1, 2]).unwrap_err();

    assert!(error.to_string().contains("missing-ch2.u16le"), "{error:#}");
    fs::set_permissions(&first_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_waveform_rejects_unrepresentable_time_axis_before_allocating() {
    let dir = unique_test_dir("raw_unrepresentable_time_axis");
    fs::create_dir(&dir).unwrap();
    let sample_count = usize::MAX / 2;
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        format!(
            r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch1]
file = "missing.u16le"
sample_count = {sample_count}
x_increment = 0.5
x_origin = 0.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0
"#
        ),
    )
    .unwrap();

    let error = read_raw_waveform_channels_from_dir(&dir, &[1]).unwrap_err();

    assert!(error.to_string().contains("does not advance"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn raw_csv_export_verifies_and_preserves_numeric_channel_order() {
    let dir = unique_test_dir("raw_csv_export");
    let output = unique_test_path("raw_csv_export.csv");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("ch1.u16le"), [0_u8, 0, 2, 0]).unwrap();
    fs::write(dir.join("ch2.u16le"), [1_u8, 0, 3, 0]).unwrap();
    fs::write(
        dir.join(RAW_METADATA_FNAME),
        r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch2]
file = "ch2.u16le"
sample_count = 2
x_increment = 0.5
x_origin = 1.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0

[channels.ch1]
file = "ch1.u16le"
sample_count = 2
x_increment = 0.5
x_origin = 1.0
x_reference = 0.0
y_increment = 1.0
y_origin = 0.0
y_reference = 0.0
"#,
    )
    .unwrap();

    let report = export_raw_waveform_csv(&dir, &output).unwrap();
    let csv = fs::read_to_string(&output).unwrap();

    assert_eq!(report.channel_count, 2);
    assert_eq!(report.sample_count, 2);
    assert_eq!(csv, "time (s),ch1,ch2\n1,0,1\n1.5,2,3\n");
    let error = export_raw_waveform_csv(&dir, &output).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(fs::read_to_string(&output).unwrap(), csv);
    fs::remove_file(output).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

fn raw_metadata_with_channel(
    ch: u8,
    file: impl Into<String>,
    sample_count: usize,
) -> RawWaveformMetadata {
    raw_metadata_with_channels([(ch, file.into(), sample_count, 0.5)])
}

fn raw_metadata_with_channels<const N: usize>(
    entries: [(u8, impl Into<String>, usize, f64); N],
) -> RawWaveformMetadata {
    let mut channels = BTreeMap::new();
    for (ch, file, sample_count, x_increment) in entries {
        channels.insert(
            format!("ch{ch}"),
            RawChannelMetadata {
                file: file.into(),
                bytes: None,
                sha256: None,
                sample_count,
                x_increment,
                x_origin: 1.0,
                x_reference: 0.0,
                y_increment: 0.5,
                y_origin: 1.0,
                y_reference: 2.0,
            },
        );
    }

    RawWaveformMetadata {
        version: RAW_METADATA_LEGACY_VERSION,
        status: None,
        pmoke_version: None,
        created_at: None,
        config_file: None,
        config_sha256: None,
        resolved_config_file: None,
        resolved_config_sha256: None,
        oscilloscope: RawOscilloscopeMetadata {
            idn_raw: None,
            waveform_format: "WORD".to_string(),
            byte_order: "little-endian".to_string(),
            memory_depth: None,
            sample_count: None,
            channels: None,
        },
        channels,
    }
}

fn unique_test_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("pmoke_{name}_{}_{}", std::process::id(), nanos))
}

fn unique_test_dir(name: &str) -> PathBuf {
    unique_test_path(name)
}
