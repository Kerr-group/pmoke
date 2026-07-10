use crate::config::{Config, FetchAnalysisInput};
use crate::constants::{FETCHED_FNAME, RAW_METADATA_FNAME, RAW_METADATA_VERSION, RAW_WAVEFORM_DIR};
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, anyhow, bail};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

struct CsvColumns {
    time_index: Option<usize>,
    channels: Vec<(usize, u8)>,
    column_count: usize,
}

#[derive(Debug, Deserialize)]
struct RawWaveformMetadata {
    version: u32,
    oscilloscope: RawOscilloscopeMetadata,
    channels: BTreeMap<String, RawChannelMetadata>,
}

#[derive(Debug, Deserialize)]
struct RawOscilloscopeMetadata {
    waveform_format: String,
    byte_order: String,
}

#[derive(Debug, Deserialize)]
struct RawChannelMetadata {
    file: String,
    sample_count: usize,
    x_increment: f64,
    x_origin: f64,
    x_reference: f64,
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
}

#[derive(Debug)]
pub struct WaveformData {
    pub t: Vec<f64>,
    pub channels: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Copy)]
struct TimeAxis {
    sample_count: usize,
    x_increment: f64,
    x_origin: f64,
    x_reference: f64,
}

impl TimeAxis {
    fn value_at(self, index: usize) -> f64 {
        self.x_origin + (index as f64 - self.x_reference) * self.x_increment
    }

    fn build(self) -> Vec<f64> {
        (0..self.sample_count).map(|i| self.value_at(i)).collect()
    }

    fn validate(self, key: &str) -> Result<()> {
        if self.sample_count == 0 {
            bail!("raw channel sample_count must be positive for {key}");
        }
        if self.x_increment <= 0.0 {
            bail!(
                "raw metadata x_increment must be positive for {key}: {}",
                self.x_increment
            );
        }
        for index in [0, self.sample_count - 1] {
            let value = self.value_at(index);
            if !value.is_finite() {
                bail!("raw metadata produces non-finite time for {key} at sample {index}: {value}");
            }
        }
        if self.sample_count > 1 {
            for (left, right) in [(0, 1), (self.sample_count - 2, self.sample_count - 1)] {
                if self.value_at(right) <= self.value_at(left) {
                    bail!(
                        "raw metadata time axis does not advance for {key} between samples {left} and {right}"
                    );
                }
            }
        }
        Ok(())
    }
}

pub fn read_all_fetched_waveforms(cfg: &Config) -> Result<WaveformData> {
    let channels = build_channel_list(cfg)?;
    read_waveform_channels(cfg, &channels)
}

pub fn read_waveform_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    match cfg.fetch.analysis_input {
        FetchAnalysisInput::Csv => read_csv_channels(cfg, channels),
        FetchAnalysisInput::Raw => read_raw_channels(cfg, channels),
        FetchAnalysisInput::Auto => read_auto_channels(cfg, channels),
    }
}

fn read_csv_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    let csv_path = cfg.artifact_path(FETCHED_FNAME);
    let (time_index, column_indices) = csv_column_indices(&csv_path, channels)?;
    let mut read_indices =
        Vec::with_capacity(column_indices.len() + usize::from(time_index.is_some()));
    if let Some(time_index) = time_index {
        read_indices.push(time_index);
    }
    read_indices.extend(column_indices.iter().copied());

    let mut columns = read_selected_columns(&csv_path, &read_indices).with_context(|| {
        format!(
            "failed to read waveform columns from {}",
            csv_path.display()
        )
    })?;

    let t = if time_index.is_some() {
        columns.remove(0)
    } else if let Some(timebase) = &cfg.legacy_timebase {
        let sample_count = columns.first().map_or(0, Vec::len);
        TimeAxis {
            sample_count,
            x_increment: timebase.dt,
            x_origin: timebase.t0,
            x_reference: 0.0,
        }
        .build()
    } else {
        bail!(
            "{} has no time column; fetch again with the current version or use raw_waveform metadata",
            csv_path.display()
        );
    };

    Ok(WaveformData {
        t,
        channels: columns,
    })
}

fn read_auto_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    match raw_status(cfg, channels)? {
        RawStatus::Complete => read_raw_channels(cfg, channels),
        RawStatus::Missing => read_csv_channels(cfg, channels),
        RawStatus::Invalid(message) => bail!("{message}"),
    }
}

fn read_raw_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    read_raw_waveform_channels_from_dir(&cfg.artifact_path(RAW_WAVEFORM_DIR), channels)
}

pub fn read_raw_waveform_channels_from_dir(
    base_dir: &Path,
    channels: &[u8],
) -> Result<WaveformData> {
    let metadata = read_raw_metadata(base_dir)?;
    validate_raw_format(&metadata)?;

    let mut time_axis = None;
    let specs = channels
        .iter()
        .map(|ch| raw_channel_spec(base_dir, &metadata, *ch, &mut time_axis))
        .collect::<Result<Vec<_>>>()?;
    for spec in &specs {
        validate_raw_channel_file_size(spec)?;
    }
    let channels = specs
        .iter()
        .map(read_raw_channel_data)
        .collect::<Result<Vec<_>>>()?;
    let t = time_axis
        .ok_or_else(|| anyhow!("no raw channels requested"))?
        .build();

    Ok(WaveformData { t, channels })
}

fn read_raw_metadata(base_dir: &Path) -> Result<RawWaveformMetadata> {
    let metadata_path = base_dir.join(RAW_METADATA_FNAME);
    let metadata = fs::symlink_metadata(&metadata_path)
        .with_context(|| format!("raw metadata not found: {}", metadata_path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "raw metadata must be a regular file: {}",
            metadata_path.display()
        );
    }
    let text = fs::read_to_string(&metadata_path)
        .with_context(|| format!("raw metadata not found: {}", metadata_path.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("failed to parse raw metadata: {}", metadata_path.display()))
}

fn validate_raw_format(metadata: &RawWaveformMetadata) -> Result<()> {
    if metadata.version != RAW_METADATA_VERSION {
        bail!(
            "unsupported raw metadata version: {} (expected {})",
            metadata.version,
            RAW_METADATA_VERSION
        );
    }
    if metadata.oscilloscope.waveform_format != "WORD" {
        bail!(
            "unsupported raw waveform format: {}",
            metadata.oscilloscope.waveform_format
        );
    }
    if metadata.oscilloscope.byte_order != "little-endian" {
        bail!(
            "unsupported raw byte order: {}",
            metadata.oscilloscope.byte_order
        );
    }
    Ok(())
}

#[derive(Debug)]
struct RawChannelSpec {
    key: String,
    path: PathBuf,
    expected_bytes: usize,
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
}

fn raw_channel_spec(
    base_dir: &Path,
    metadata: &RawWaveformMetadata,
    ch: u8,
    time_axis: &mut Option<TimeAxis>,
) -> Result<RawChannelSpec> {
    let key = format!("ch{ch}");
    let channel = metadata
        .channels
        .get(&key)
        .ok_or_else(|| anyhow!("raw channel missing in metadata: {key}"))?;

    validate_finite("x_increment", channel.x_increment, &key)?;
    validate_finite("x_origin", channel.x_origin, &key)?;
    validate_finite("x_reference", channel.x_reference, &key)?;
    validate_finite("y_increment", channel.y_increment, &key)?;
    validate_finite("y_origin", channel.y_origin, &key)?;
    validate_finite("y_reference", channel.y_reference, &key)?;
    validate_voltage_range(
        channel.y_increment,
        channel.y_origin,
        channel.y_reference,
        &key,
    )?;

    let path = resolve_raw_channel_path(base_dir, &channel.file, &key)?;
    let expected_bytes = channel
        .sample_count
        .checked_mul(2)
        .ok_or_else(|| anyhow!("raw channel sample count overflows for {key}"))?;

    let channel_axis = TimeAxis {
        sample_count: channel.sample_count,
        x_increment: channel.x_increment,
        x_origin: channel.x_origin,
        x_reference: channel.x_reference,
    };
    channel_axis.validate(&key)?;
    match time_axis {
        Some(expected) => validate_time_axis(*expected, channel_axis, &key)?,
        None => *time_axis = Some(channel_axis),
    }

    Ok(RawChannelSpec {
        key,
        path,
        expected_bytes,
        y_increment: channel.y_increment,
        y_origin: channel.y_origin,
        y_reference: channel.y_reference,
    })
}

fn read_raw_channel_data(spec: &RawChannelSpec) -> Result<Vec<f64>> {
    validate_raw_channel_file_size(spec)?;

    let mut file = File::open(&spec.path)
        .with_context(|| format!("failed to open raw channel file: {}", spec.path.display()))?;
    let opened_bytes = file
        .metadata()
        .with_context(|| format!("failed to stat raw channel file: {}", spec.path.display()))?
        .len();
    if opened_bytes != spec.expected_bytes as u64 {
        bail!(
            "raw channel file size mismatch for {}: expected {} bytes, got {}",
            spec.key,
            spec.expected_bytes,
            opened_bytes
        );
    }

    let mut data = vec![0_u8; spec.expected_bytes];
    file.read_exact(&mut data)
        .with_context(|| format!("failed to read raw channel file: {}", spec.path.display()))?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra).with_context(|| {
        format!(
            "failed to verify raw channel file end: {}",
            spec.path.display()
        )
    })? != 0
    {
        bail!(
            "raw channel file grew while reading for {}: {}",
            spec.key,
            spec.path.display()
        );
    }

    Ok(convert_raw_word_to_voltages(
        &data,
        spec.y_increment,
        spec.y_origin,
        spec.y_reference,
    ))
}

fn validate_raw_channel_file_size(spec: &RawChannelSpec) -> Result<()> {
    let actual_bytes = raw_channel_file_size(&spec.path, &spec.key)?;
    if actual_bytes != spec.expected_bytes as u64 {
        bail!(
            "raw channel file size mismatch for {}: expected {} bytes, got {}",
            spec.key,
            spec.expected_bytes,
            actual_bytes
        );
    }
    Ok(())
}

fn convert_raw_word_to_voltages(
    data: &[u8],
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
) -> Vec<f64> {
    data.par_chunks_exact(2)
        .map(|chunk| {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]) as f64;
            (v - y_origin - y_reference) * y_increment
        })
        .collect()
}

fn validate_time_axis(expected: TimeAxis, actual: TimeAxis, key: &str) -> Result<()> {
    if expected.sample_count != actual.sample_count {
        bail!(
            "raw timebase mismatch for {key}: sample_count {} != {}",
            actual.sample_count,
            expected.sample_count
        );
    }
    validate_close("x_increment", expected.x_increment, actual.x_increment, key)?;
    validate_close("x_origin", expected.x_origin, actual.x_origin, key)?;
    validate_close("x_reference", expected.x_reference, actual.x_reference, key)?;
    Ok(())
}

fn validate_close(name: &str, expected: f64, actual: f64, key: &str) -> Result<()> {
    validate_finite(name, expected, key)?;
    validate_finite(name, actual, key)?;
    let scale = expected.abs().max(actual.abs());
    let tolerance = (scale * 1.0e-12).max(1.0e-18);
    if (expected - actual).abs() > tolerance {
        bail!("raw timebase mismatch for {key}: {name} {actual} != {expected}");
    }
    Ok(())
}

fn validate_finite(name: &str, value: f64, key: &str) -> Result<()> {
    if !value.is_finite() {
        bail!("raw metadata value must be finite for {key}: {name}={value}");
    }
    Ok(())
}

fn validate_voltage_range(
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
    key: &str,
) -> Result<()> {
    if y_increment <= 0.0 {
        bail!("raw metadata y_increment must be positive for {key}: {y_increment}");
    }
    let voltage_at = |word: u16| (word as f64 - y_origin - y_reference) * y_increment;
    for word in [u16::MIN, u16::MAX] {
        let voltage = voltage_at(word);
        if !voltage.is_finite() {
            bail!(
                "raw metadata produces non-finite voltage for {key} at WORD value {word}: {voltage}"
            );
        }
    }
    for (left, right) in [(u16::MIN, 1), (u16::MAX - 1, u16::MAX)] {
        if voltage_at(right) <= voltage_at(left) {
            bail!(
                "raw metadata voltage scaling does not distinguish adjacent WORD values {left} and {right} for {key}"
            );
        }
    }
    Ok(())
}

fn resolve_raw_channel_path(base_dir: &Path, file: &str, key: &str) -> Result<PathBuf> {
    let relative = Path::new(file);
    let mut components = relative.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        bail!("raw channel file must be a plain file name for {key}: {file}");
    }
    Ok(base_dir.join(relative))
}

fn raw_channel_file_size(path: &Path, key: &str) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("raw channel file not found: {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "raw channel file must be a regular file for {key}: {}",
            path.display()
        );
    }
    Ok(metadata.len())
}

enum RawStatus {
    Complete,
    Missing,
    Invalid(String),
}

fn raw_status(cfg: &Config, channels: &[u8]) -> Result<RawStatus> {
    raw_status_in_dir(&cfg.artifact_path(RAW_WAVEFORM_DIR), channels)
}

fn raw_status_in_dir(base_dir: &Path, channels: &[u8]) -> Result<RawStatus> {
    let metadata_path = base_dir.join(RAW_METADATA_FNAME);
    if !path_entry_exists(&metadata_path)? {
        return if path_entry_exists(base_dir)? {
            Ok(RawStatus::Invalid(format!(
                "raw metadata not found: {}",
                metadata_path.display()
            )))
        } else {
            Ok(RawStatus::Missing)
        };
    }

    let metadata = match read_raw_metadata(base_dir) {
        Ok(metadata) => metadata,
        Err(error) => return Ok(RawStatus::Invalid(error.to_string())),
    };
    if let Err(error) = validate_raw_format(&metadata) {
        return Ok(RawStatus::Invalid(error.to_string()));
    }

    let mut time_axis = None;
    for &ch in channels {
        let spec = match raw_channel_spec(base_dir, &metadata, ch, &mut time_axis) {
            Ok(spec) => spec,
            Err(error) => return Ok(RawStatus::Invalid(error.to_string())),
        };
        if let Err(error) = validate_raw_channel_file_size(&spec) {
            return Ok(RawStatus::Invalid(error.to_string()));
        }
    }

    Ok(RawStatus::Complete)
}

fn path_entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect raw waveform path: {}", path.display())),
    }
}

fn csv_column_indices(path: &Path, channels: &[u8]) -> Result<(Option<usize>, Vec<usize>)> {
    let csv_columns = csv_columns_from_header(path)?;
    resolve_csv_column_indices(csv_columns, channels)
}

fn resolve_csv_column_indices(
    csv_columns: CsvColumns,
    channels: &[u8],
) -> Result<(Option<usize>, Vec<usize>)> {
    if csv_columns.channels.is_empty() {
        let column_indices = (0..csv_columns.column_count)
            .filter(|idx| Some(*idx) != csv_columns.time_index)
            .take(channels.len())
            .collect::<Vec<_>>();
        if column_indices.len() != channels.len() {
            bail!(
                "csv contains {} data columns, but {} channels were requested",
                column_indices.len(),
                channels.len()
            );
        }
        return Ok((csv_columns.time_index, column_indices));
    }

    let columns = channels
        .iter()
        .map(|ch| {
            csv_columns
                .channels
                .iter()
                .find_map(|(col_idx, fetched_ch)| (fetched_ch == ch).then_some(*col_idx))
                .ok_or_else(|| {
                    let available = csv_columns
                        .channels
                        .iter()
                        .map(|(_, fetched_ch)| *fetched_ch)
                        .collect::<Vec<_>>();
                    anyhow!("channel {ch} not found in fetched channels {available:?}")
                })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((csv_columns.time_index, columns))
}

fn csv_columns_from_header(path: &Path) -> Result<CsvColumns> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .with_context(|| format!("failed to open csv: {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed to read csv header: {}", path.display()))?;

    let time_index = headers.iter().enumerate().find_map(|(col_idx, header)| {
        let normalized = header.trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "time" | "time (s)" | "t" | "t (s)").then_some(col_idx)
    });

    let channels = headers
        .iter()
        .enumerate()
        .filter_map(|(col_idx, header)| {
            header
                .trim()
                .to_ascii_lowercase()
                .strip_prefix("ch")
                .and_then(|number| number.parse::<u8>().ok())
                .map(|ch| (col_idx, ch))
        })
        .collect();

    Ok(CsvColumns {
        time_index,
        channels,
        column_count: headers.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn raw_word_conversion_matches_dho_formula() {
        let bytes = [0x00, 0x00, 0x01, 0x00, 0x10, 0x00];
        let values = convert_raw_word_to_voltages(&bytes, 0.5, 1.0, 2.0);
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

        let metadata = raw_metadata_with_channels([
            (1, "ch1.u16le", 2, 5.0e-10),
            (2, "ch2.u16le", 2, 5.01e-10),
        ]);
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
version = 2

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
            RawStatus::Invalid(message) if message.contains("unsupported raw metadata version: 2")
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

        assert_eq!(waveform.t, vec![-2.0, -1.5]);
        assert_eq!(waveform.channels, vec![vec![20.0, 22.0], vec![0.0, 1.0]]);
        fs::remove_dir_all(dir).unwrap();
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
            version: RAW_METADATA_VERSION,
            oscilloscope: RawOscilloscopeMetadata {
                waveform_format: "WORD".to_string(),
                byte_order: "little-endian".to_string(),
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
}
