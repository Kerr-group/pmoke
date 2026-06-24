use crate::config::{Config, FetchAnalysisInput};
use crate::constants::{FETCHED_FNAME, RAW_METADATA_FNAME, RAW_WAVEFORM_DIR};
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

struct CsvColumns {
    time_index: Option<usize>,
    channels: Vec<(usize, u8)>,
    column_count: usize,
}

#[derive(Debug, Deserialize)]
struct RawWaveformMetadata {
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
    fn build(self) -> Vec<f64> {
        (0..self.sample_count)
            .map(|i| self.x_origin + (i as f64 - self.x_reference) * self.x_increment)
            .collect()
    }
}

pub fn read_all_fetched_waveforms(cfg: &Config) -> Result<WaveformData> {
    let channels = build_channel_list(cfg)?;
    read_waveform_channels(cfg, &channels)
}

pub fn read_waveform_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    match cfg.fetch.analysis_input {
        FetchAnalysisInput::Csv => read_csv_channels(cfg, channels),
        FetchAnalysisInput::Raw => read_raw_channels(channels),
        FetchAnalysisInput::Auto => read_auto_channels(cfg, channels),
    }
}

fn read_csv_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    let (time_index, column_indices) = csv_column_indices(channels)?;
    let mut read_indices =
        Vec::with_capacity(column_indices.len() + usize::from(time_index.is_some()));
    if let Some(time_index) = time_index {
        read_indices.push(time_index);
    }
    read_indices.extend(column_indices.iter().copied());

    let mut columns = read_selected_columns(FETCHED_FNAME, &read_indices)
        .with_context(|| format!("failed to read waveform columns from {FETCHED_FNAME}"))?;

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
            "{FETCHED_FNAME} has no time column; fetch again with the current version or use raw_waveform metadata"
        );
    };

    Ok(WaveformData {
        t,
        channels: columns,
    })
}

fn read_auto_channels(cfg: &Config, channels: &[u8]) -> Result<WaveformData> {
    match raw_status(channels)? {
        RawStatus::Complete => read_raw_channels(channels),
        RawStatus::Missing => read_csv_channels(cfg, channels),
        RawStatus::Invalid(message) => bail!("{message}"),
    }
}

fn read_raw_channels(channels: &[u8]) -> Result<WaveformData> {
    let base_dir = Path::new(RAW_WAVEFORM_DIR);
    let metadata = read_raw_metadata(base_dir)?;
    validate_raw_format(&metadata)?;

    let mut time_axis = None;
    let channels = channels
        .iter()
        .map(|ch| read_raw_channel(base_dir, &metadata, *ch, &mut time_axis))
        .collect::<Result<Vec<_>>>()?;
    let t = time_axis
        .ok_or_else(|| anyhow!("no raw channels requested"))?
        .build();

    Ok(WaveformData { t, channels })
}

fn read_raw_metadata(base_dir: &Path) -> Result<RawWaveformMetadata> {
    let metadata_path = base_dir.join(RAW_METADATA_FNAME);
    let text = fs::read_to_string(&metadata_path)
        .with_context(|| format!("raw metadata not found: {}", metadata_path.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("failed to parse raw metadata: {}", metadata_path.display()))
}

fn validate_raw_format(metadata: &RawWaveformMetadata) -> Result<()> {
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

fn read_raw_channel(
    base_dir: &Path,
    metadata: &RawWaveformMetadata,
    ch: u8,
    time_axis: &mut Option<TimeAxis>,
) -> Result<Vec<f64>> {
    let key = format!("ch{ch}");
    let channel = metadata
        .channels
        .get(&key)
        .ok_or_else(|| anyhow!("raw channel missing in metadata: {key}"))?;

    let path = base_dir.join(&channel.file);
    let expected_bytes = channel
        .sample_count
        .checked_mul(2)
        .ok_or_else(|| anyhow!("raw channel sample count overflows for {key}"))?;

    let data = fs::read(&path)
        .with_context(|| format!("raw channel file not found: {}", path.display()))?;
    if data.len() != expected_bytes {
        bail!(
            "raw channel file size mismatch for {key}: expected {expected_bytes} bytes, got {}",
            data.len()
        );
    }

    let channel_axis = TimeAxis {
        sample_count: channel.sample_count,
        x_increment: channel.x_increment,
        x_origin: channel.x_origin,
        x_reference: channel.x_reference,
    };
    match time_axis {
        Some(expected) => validate_time_axis(*expected, channel_axis, &key)?,
        None => *time_axis = Some(channel_axis),
    }

    Ok(convert_raw_word_to_voltages(
        &data,
        channel.y_increment,
        channel.y_origin,
        channel.y_reference,
    ))
}

fn convert_raw_word_to_voltages(
    data: &[u8],
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
) -> Vec<f64> {
    data.chunks_exact(2)
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
    let scale = expected.abs().max(actual.abs());
    let tolerance = (scale * 1.0e-12).max(1.0e-18);
    if (expected - actual).abs() > tolerance {
        bail!("raw timebase mismatch for {key}: {name} {actual} != {expected}");
    }
    Ok(())
}

enum RawStatus {
    Complete,
    Missing,
    Invalid(String),
}

fn raw_status(channels: &[u8]) -> Result<RawStatus> {
    raw_status_in_dir(Path::new(RAW_WAVEFORM_DIR), channels)
}

fn raw_status_in_dir(base_dir: &Path, channels: &[u8]) -> Result<RawStatus> {
    let metadata_path = base_dir.join(RAW_METADATA_FNAME);
    if !metadata_path.exists() {
        return if base_dir.exists() {
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

    for &ch in channels {
        let key = format!("ch{ch}");
        let Some(channel) = metadata.channels.get(&key) else {
            return Ok(RawStatus::Invalid(format!(
                "raw channel missing in metadata: {key}"
            )));
        };
        let expected_bytes = match channel.sample_count.checked_mul(2) {
            Some(value) => value,
            None => {
                return Ok(RawStatus::Invalid(format!(
                    "raw channel sample count overflows for {key}"
                )));
            }
        };
        let path = base_dir.join(&channel.file);
        let actual_bytes = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(_) => {
                return Ok(RawStatus::Invalid(format!(
                    "raw channel file not found: {}",
                    path.display()
                )));
            }
        };
        if actual_bytes != expected_bytes as u64 {
            return Ok(RawStatus::Invalid(format!(
                "raw channel file size mismatch for {key}: expected {expected_bytes} bytes, got {actual_bytes}"
            )));
        }
    }

    Ok(RawStatus::Complete)
}

fn csv_column_indices(channels: &[u8]) -> Result<(Option<usize>, Vec<usize>)> {
    let csv_columns = csv_columns_from_header(Path::new(FETCHED_FNAME))?;
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
        let values = read_raw_channel(&dir, &metadata, 4, &mut time_axis).unwrap();

        assert_eq!(values, vec![-1.5, 6.5]);
        assert_eq!(time_axis.unwrap().build(), vec![1.0, 1.5]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_raw_channel_rejects_file_size_mismatch() {
        let dir = unique_test_dir("raw_size_mismatch");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("ch2.u16le"), [0x00, 0x00]).unwrap();

        let metadata = raw_metadata_with_channel(2, "ch2.u16le", 2);
        let mut time_axis = None;
        let error = read_raw_channel(&dir, &metadata, 2, &mut time_axis).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("raw channel file size mismatch for ch2")
        );
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
        read_raw_channel(&dir, &metadata, 1, &mut time_axis).unwrap();
        let error = read_raw_channel(&dir, &metadata, 2, &mut time_axis).unwrap_err();

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
