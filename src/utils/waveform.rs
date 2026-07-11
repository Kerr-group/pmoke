use crate::config::{Config, FetchAnalysisInput};
use crate::constants::{
    FETCHED_FNAME, RAW_METADATA_FNAME, RAW_METADATA_LEGACY_VERSION, RAW_METADATA_VERSION,
    RAW_WAVEFORM_DIR,
};
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use crate::utils::raw_data::{
    RawTimeAxis, RawVoltageScale, TimeAxisError, TimeAxisMismatch, VoltageScaleError,
};
use crate::utils::time_axis::WaveformTime;
use anyhow::{Context, Result, anyhow, bail};
use rayon::prelude::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const RAW_READ_CHUNK_BYTES: usize = 8 * 1024 * 1024;

struct CsvColumns {
    time_index: Option<usize>,
    channels: Vec<(usize, u8)>,
    column_count: usize,
}

#[derive(Debug, Deserialize)]
struct RawWaveformMetadata {
    version: u32,
    status: Option<String>,
    pmoke_version: Option<String>,
    created_at: Option<String>,
    config_file: Option<String>,
    config_sha256: Option<String>,
    oscilloscope: RawOscilloscopeMetadata,
    channels: BTreeMap<String, RawChannelMetadata>,
}

#[derive(Debug, Deserialize)]
struct RawOscilloscopeMetadata {
    idn_raw: Option<String>,
    waveform_format: String,
    byte_order: String,
    memory_depth: Option<usize>,
    sample_count: Option<usize>,
    channels: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
struct RawChannelMetadata {
    file: String,
    bytes: Option<usize>,
    sha256: Option<String>,
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
    pub t: WaveformTime,
    pub channels: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawVerification {
    pub metadata_version: u32,
    pub channel_count: usize,
    pub sample_count: usize,
    pub total_bytes: u64,
    pub checksums_verified: bool,
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
        WaveformTime::Explicit(columns.remove(0))
    } else if let Some(timebase) = &cfg.legacy_timebase {
        let sample_count = columns.first().map_or(0, Vec::len);
        WaveformTime::Uniform(RawTimeAxis {
            sample_count,
            x_increment: timebase.dt,
            x_origin: timebase.t0,
            x_reference: 0.0,
        })
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
    validate_manifest_config(base_dir, &metadata)?;

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
    let t = WaveformTime::Uniform(time_axis.ok_or_else(|| anyhow!("no raw channels requested"))?);

    Ok(WaveformData { t, channels })
}

pub fn verify_raw_waveform_dir(base_dir: &Path) -> Result<RawVerification> {
    let metadata = read_raw_metadata(base_dir)?;
    validate_raw_format(&metadata)?;
    validate_manifest_config(base_dir, &metadata)?;

    let declared_channels = metadata
        .channels
        .keys()
        .map(|key| {
            key.strip_prefix("ch")
                .ok_or_else(|| anyhow!("raw channel metadata key must start with ch: {key}"))?
                .parse::<u8>()
                .with_context(|| format!("invalid raw channel metadata key: {key}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if declared_channels.is_empty() {
        bail!("raw metadata contains no channels");
    }
    if metadata.version == RAW_METADATA_VERSION
        && metadata.oscilloscope.channels.as_deref() != Some(declared_channels.as_slice())
    {
        bail!("raw metadata channel list does not match channel entries");
    }

    let mut time_axis = None;
    let specs = declared_channels
        .iter()
        .map(|&channel| raw_channel_spec(base_dir, &metadata, channel, &mut time_axis))
        .collect::<Result<Vec<_>>>()?;
    let mut total_bytes = 0u64;
    for spec in &specs {
        validate_raw_channel_file_size(spec)?;
        verify_raw_channel_checksum(spec)?;
        total_bytes = total_bytes
            .checked_add(spec.expected_bytes as u64)
            .ok_or_else(|| anyhow!("raw verification total byte count overflows"))?;
    }
    let sample_count = time_axis
        .ok_or_else(|| anyhow!("raw metadata contains no channel time axis"))?
        .sample_count;

    Ok(RawVerification {
        metadata_version: metadata.version,
        channel_count: specs.len(),
        sample_count,
        total_bytes,
        checksums_verified: metadata.version == RAW_METADATA_VERSION,
    })
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
    if !matches!(
        metadata.version,
        RAW_METADATA_LEGACY_VERSION | RAW_METADATA_VERSION
    ) {
        bail!(
            "unsupported raw metadata version: {} (supported: {} and {})",
            metadata.version,
            RAW_METADATA_LEGACY_VERSION,
            RAW_METADATA_VERSION
        );
    }
    if metadata.version == RAW_METADATA_VERSION && metadata.status.as_deref() != Some("complete") {
        bail!("raw metadata version 2 requires status = \"complete\"");
    }
    if metadata.version == RAW_METADATA_VERSION {
        for (name, value) in [
            ("pmoke_version", metadata.pmoke_version.as_deref()),
            ("created_at", metadata.created_at.as_deref()),
            (
                "oscilloscope.idn_raw",
                metadata.oscilloscope.idn_raw.as_deref(),
            ),
        ] {
            if value.is_none_or(|value| value.trim().is_empty()) {
                bail!("raw metadata version 2 requires non-empty {name}");
            }
        }
        let memory_depth = metadata
            .oscilloscope
            .memory_depth
            .ok_or_else(|| anyhow!("raw metadata version 2 requires oscilloscope.memory_depth"))?;
        let sample_count = metadata
            .oscilloscope
            .sample_count
            .ok_or_else(|| anyhow!("raw metadata version 2 requires oscilloscope.sample_count"))?;
        if memory_depth == 0 || sample_count == 0 || memory_depth != sample_count {
            bail!(
                "raw metadata oscilloscope memory_depth/sample_count mismatch: {memory_depth} != {sample_count}"
            );
        }
        if metadata
            .oscilloscope
            .channels
            .as_ref()
            .is_none_or(Vec::is_empty)
        {
            bail!("raw metadata version 2 requires oscilloscope.channels");
        }
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

fn validate_manifest_config(base_dir: &Path, metadata: &RawWaveformMetadata) -> Result<()> {
    if metadata.version != RAW_METADATA_VERSION {
        return Ok(());
    }
    let file = metadata
        .config_file
        .as_deref()
        .ok_or_else(|| anyhow!("raw metadata version 2 requires config_file"))?;
    let expected = metadata
        .config_sha256
        .as_deref()
        .ok_or_else(|| anyhow!("raw metadata version 2 requires config_sha256"))?;
    validate_sha256(expected, "config.source.toml")?;
    let path = resolve_raw_channel_path(base_dir, file, "config.source.toml")?;
    let file_type = fs::symlink_metadata(&path)
        .with_context(|| format!("raw config snapshot not found: {}", path.display()))?;
    if !file_type.file_type().is_file() {
        bail!(
            "raw config snapshot must be a regular file: {}",
            path.display()
        );
    }
    let contents = fs::read(&path)
        .with_context(|| format!("failed to read raw config snapshot: {}", path.display()))?;
    let actual = format!("{:x}", Sha256::digest(&contents));
    if actual != expected {
        bail!("raw config snapshot checksum mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("raw metadata sha256 must be 64 hexadecimal characters for {label}");
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        bail!("raw metadata sha256 must use lowercase hexadecimal for {label}");
    }
    Ok(())
}

#[derive(Debug)]
struct RawChannelSpec {
    key: String,
    path: PathBuf,
    expected_bytes: usize,
    expected_sha256: Option<String>,
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
}

fn raw_channel_spec(
    base_dir: &Path,
    metadata: &RawWaveformMetadata,
    ch: u8,
    time_axis: &mut Option<RawTimeAxis>,
) -> Result<RawChannelSpec> {
    let key = format!("ch{ch}");
    if metadata.version == RAW_METADATA_VERSION
        && !metadata
            .oscilloscope
            .channels
            .as_ref()
            .is_some_and(|channels| channels.contains(&ch))
    {
        bail!("raw channel {ch} is not declared in oscilloscope.channels");
    }
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
    if metadata.version == RAW_METADATA_VERSION
        && metadata.oscilloscope.sample_count != Some(channel.sample_count)
    {
        bail!(
            "raw channel sample_count mismatch for {key}: {} != {}",
            channel.sample_count,
            metadata.oscilloscope.sample_count.unwrap_or_default()
        );
    }
    let expected_sha256 = if metadata.version == RAW_METADATA_VERSION {
        let declared_bytes = channel
            .bytes
            .ok_or_else(|| anyhow!("raw metadata bytes missing for {key}"))?;
        if declared_bytes != expected_bytes {
            bail!(
                "raw metadata byte count mismatch for {key}: {declared_bytes} != {expected_bytes}"
            );
        }
        let checksum = channel
            .sha256
            .as_deref()
            .ok_or_else(|| anyhow!("raw metadata sha256 missing for {key}"))?;
        validate_sha256(checksum, &key)?;
        Some(checksum.to_owned())
    } else {
        None
    };

    let channel_axis = RawTimeAxis {
        sample_count: channel.sample_count,
        x_increment: channel.x_increment,
        x_origin: channel.x_origin,
        x_reference: channel.x_reference,
    };
    validate_raw_time_axis(channel_axis, &key)?;
    match time_axis {
        Some(expected) => validate_time_axis(*expected, channel_axis, &key)?,
        None => *time_axis = Some(channel_axis),
    }

    Ok(RawChannelSpec {
        key,
        path,
        expected_bytes,
        expected_sha256,
        y_increment: channel.y_increment,
        y_origin: channel.y_origin,
        y_reference: channel.y_reference,
    })
}

fn read_raw_channel_data(spec: &RawChannelSpec) -> Result<Vec<f64>> {
    read_raw_channel_data_with_chunk_size(spec, RAW_READ_CHUNK_BYTES)
}

fn read_raw_channel_data_with_chunk_size(
    spec: &RawChannelSpec,
    chunk_bytes: usize,
) -> Result<Vec<f64>> {
    if chunk_bytes == 0 || !chunk_bytes.is_multiple_of(2) {
        bail!("raw read chunk size must be a positive even number: {chunk_bytes}");
    }
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

    let sample_count = spec.expected_bytes / 2;
    let mut voltages = Vec::new();
    voltages.try_reserve_exact(sample_count).with_context(|| {
        format!(
            "failed to allocate {sample_count} voltage samples for {}",
            spec.key
        )
    })?;
    voltages.resize(sample_count, 0.0);

    let scale = RawVoltageScale {
        y_increment: spec.y_increment,
        y_origin: spec.y_origin,
        y_reference: spec.y_reference,
    };
    let mut buffer = vec![0_u8; chunk_bytes.min(spec.expected_bytes)];
    let mut hasher = spec.expected_sha256.as_ref().map(|_| Sha256::new());
    let mut byte_offset = 0;
    while byte_offset < spec.expected_bytes {
        let bytes_to_read = buffer.len().min(spec.expected_bytes - byte_offset);
        let chunk = &mut buffer[..bytes_to_read];
        file.read_exact(chunk).with_context(|| {
            format!(
                "failed to read raw channel file {} at byte {byte_offset}",
                spec.path.display()
            )
        })?;
        if let Some(hasher) = &mut hasher {
            hasher.update(&*chunk);
        }
        let sample_offset = byte_offset / 2;
        decode_raw_word_chunk_into(
            chunk,
            &mut voltages[sample_offset..sample_offset + bytes_to_read / 2],
            scale,
        );
        byte_offset += bytes_to_read;
    }
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

    if let (Some(hasher), Some(expected)) = (hasher, &spec.expected_sha256) {
        let actual = format!("{:x}", hasher.finalize());
        if &actual != expected {
            bail!(
                "raw channel checksum mismatch for {}: expected {expected}, got {actual}",
                spec.key
            );
        }
    }

    Ok(voltages)
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

fn verify_raw_channel_checksum(spec: &RawChannelSpec) -> Result<()> {
    let Some(expected) = &spec.expected_sha256 else {
        return Ok(());
    };
    let mut file = File::open(&spec.path)
        .with_context(|| format!("failed to open raw channel file: {}", spec.path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; RAW_READ_CHUNK_BYTES.min(spec.expected_bytes)];
    let mut remaining = spec.expected_bytes;
    while remaining > 0 {
        let read_len = remaining.min(buffer.len());
        file.read_exact(&mut buffer[..read_len]).with_context(|| {
            format!("failed to verify raw channel file: {}", spec.path.display())
        })?;
        hasher.update(&buffer[..read_len]);
        remaining -= read_len;
    }
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        bail!("raw channel file grew while verifying for {}", spec.key);
    }
    let actual = format!("{:x}", hasher.finalize());
    if &actual != expected {
        bail!(
            "raw channel checksum mismatch for {}: expected {expected}, got {actual}",
            spec.key
        );
    }
    Ok(())
}

#[doc(hidden)]
pub fn convert_raw_word_to_voltages(
    data: &[u8],
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
) -> Result<Vec<f64>> {
    let scale = RawVoltageScale {
        y_increment,
        y_origin,
        y_reference,
    };
    if !data.len().is_multiple_of(2) {
        bail!(
            "raw WORD data has an incomplete final sample: {} bytes",
            data.len()
        );
    }
    let mut output = vec![0.0; data.len() / 2];
    decode_raw_word_chunk_into(data, &mut output, scale);
    Ok(output)
}

fn decode_raw_word_chunk_into(data: &[u8], output: &mut [f64], scale: RawVoltageScale) {
    debug_assert_eq!(data.len(), output.len() * 2);
    output
        .par_iter_mut()
        .zip(data.par_chunks_exact(2))
        .for_each(|(output, chunk)| {
            let word = u16::from_le_bytes([chunk[0], chunk[1]]);
            *output = scale.value_at(word);
        });
}

fn validate_raw_time_axis(axis: RawTimeAxis, key: &str) -> Result<()> {
    match axis.validate_geometry() {
        Ok(()) => Ok(()),
        Err(TimeAxisError::Empty) => {
            bail!("raw channel sample_count must be positive for {key}")
        }
        Err(TimeAxisError::NonPositiveIncrement(value)) => {
            bail!("raw metadata x_increment must be positive for {key}: {value}")
        }
        Err(TimeAxisError::NonFiniteTime { index, value }) => {
            bail!("raw metadata produces non-finite time for {key} at sample {index}: {value}")
        }
        Err(TimeAxisError::NonIncreasing { left, right }) => {
            bail!(
                "raw metadata time axis does not advance for {key} between samples {left} and {right}"
            )
        }
    }
}

fn validate_time_axis(expected: RawTimeAxis, actual: RawTimeAxis, key: &str) -> Result<()> {
    match expected.compare(actual) {
        Ok(()) => Ok(()),
        Err(TimeAxisMismatch::SampleCount { expected, actual }) => {
            bail!("raw timebase mismatch for {key}: sample_count {actual} != {expected}")
        }
        Err(TimeAxisMismatch::NonFinite { name }) => {
            bail!("raw metadata value must be finite for {key}: {name}=non-finite")
        }
        Err(TimeAxisMismatch::Value {
            name,
            expected,
            actual,
        }) => bail!("raw timebase mismatch for {key}: {name} {actual} != {expected}"),
    }
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
    let scale = RawVoltageScale {
        y_increment,
        y_origin,
        y_reference,
    };
    match scale.validate_geometry() {
        Ok(()) => Ok(()),
        Err(VoltageScaleError::InvalidIncrement(value)) => {
            bail!("raw metadata y_increment must be positive for {key}: {value}")
        }
        Err(VoltageScaleError::NonFinite { word, value }) => {
            bail!(
                "raw metadata produces non-finite voltage for {key} at WORD value {word}: {value}"
            )
        }
        Err(VoltageScaleError::Indistinguishable { left, right }) => {
            bail!(
                "raw metadata voltage scaling does not distinguish adjacent WORD values {left} and {right} for {key}"
            )
        }
    }
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
    if let Err(error) = validate_manifest_config(base_dir, &metadata) {
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
#[path = "waveform/tests.rs"]
mod tests;
