use crate::config::{Config, FetchAnalysisInput};
use crate::constants::{FETCHED_FNAME, RAW_METADATA_FNAME, RAW_WAVEFORM_DIR};
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
}

pub fn read_all_fetched_waveforms(cfg: &Config) -> Result<Vec<Vec<f64>>> {
    let channels = build_channel_list(cfg)?;
    read_waveform_channels(cfg, &channels)
}

pub fn read_waveform_channels(cfg: &Config, channels: &[u8]) -> Result<Vec<Vec<f64>>> {
    match cfg.fetch.analysis_input {
        FetchAnalysisInput::Csv => read_csv_channels(channels),
        FetchAnalysisInput::Raw => read_raw_channels(channels),
        FetchAnalysisInput::Auto => read_auto_channels(channels),
    }
}

fn read_csv_channels(channels: &[u8]) -> Result<Vec<Vec<f64>>> {
    let column_indices = csv_column_indices(channels)?;

    read_selected_columns(FETCHED_FNAME, &column_indices)
        .with_context(|| format!("failed to read waveform columns from {FETCHED_FNAME}"))
}

fn read_auto_channels(channels: &[u8]) -> Result<Vec<Vec<f64>>> {
    match raw_status(channels)? {
        RawStatus::Complete => read_raw_channels(channels),
        RawStatus::Missing => read_csv_channels(channels),
        RawStatus::Invalid(message) => bail!("{message}"),
    }
}

fn read_raw_channels(channels: &[u8]) -> Result<Vec<Vec<f64>>> {
    let base_dir = Path::new(RAW_WAVEFORM_DIR);
    let metadata = read_raw_metadata(base_dir)?;
    validate_raw_format(&metadata)?;

    channels
        .iter()
        .map(|ch| read_raw_channel(base_dir, &metadata, *ch))
        .collect()
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

fn read_raw_channel(base_dir: &Path, metadata: &RawWaveformMetadata, ch: u8) -> Result<Vec<f64>> {
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

fn csv_column_indices(channels: &[u8]) -> Result<Vec<usize>> {
    let fetched_channels = channel_columns_from_csv_header(Path::new(FETCHED_FNAME))?;
    if fetched_channels.is_empty() {
        return Ok((0..channels.len()).collect());
    }

    channels
        .iter()
        .map(|ch| {
            fetched_channels
                .iter()
                .find_map(|(col_idx, fetched_ch)| (fetched_ch == ch).then_some(*col_idx))
                .ok_or_else(|| {
                    let available = fetched_channels
                        .iter()
                        .map(|(_, fetched_ch)| *fetched_ch)
                        .collect::<Vec<_>>();
                    anyhow!("channel {ch} not found in fetched channels {available:?}")
                })
        })
        .collect()
}

fn channel_columns_from_csv_header(path: &Path) -> Result<Vec<(usize, u8)>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .with_context(|| format!("failed to open csv: {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed to read csv header: {}", path.display()))?;

    Ok(headers
        .iter()
        .enumerate()
        .filter_map(|(col_idx, header)| {
            header
                .trim()
                .strip_prefix("ch")
                .and_then(|number| number.parse::<u8>().ok())
                .map(|ch| (col_idx, ch))
        })
        .collect())
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
        fs::write(&path, "ch3,ch1,ch4\n1,2,3\n").unwrap();

        let channels = channel_columns_from_csv_header(&path)
            .unwrap()
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

        let columns = channel_columns_from_csv_header(&path).unwrap();

        assert_eq!(columns, vec![(1, 3), (2, 1), (3, 4)]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_raw_channel_converts_u16le_with_metadata_scaling() {
        let dir = unique_test_dir("raw_channel");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("ch4.u16le"), [0x00, 0x00, 0x10, 0x00]).unwrap();

        let metadata = raw_metadata_with_channel(4, "ch4.u16le", 2);
        let values = read_raw_channel(&dir, &metadata, 4).unwrap();

        assert_eq!(values, vec![-1.5, 6.5]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_raw_channel_rejects_file_size_mismatch() {
        let dir = unique_test_dir("raw_size_mismatch");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("ch2.u16le"), [0x00, 0x00]).unwrap();

        let metadata = raw_metadata_with_channel(2, "ch2.u16le", 2);
        let error = read_raw_channel(&dir, &metadata, 2).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("raw channel file size mismatch for ch2")
        );
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
        let mut channels = BTreeMap::new();
        channels.insert(
            format!("ch{ch}"),
            RawChannelMetadata {
                file: file.into(),
                sample_count,
                y_increment: 0.5,
                y_origin: 1.0,
                y_reference: 2.0,
            },
        );

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
