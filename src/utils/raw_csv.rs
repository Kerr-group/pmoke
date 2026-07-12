use crate::utils::raw_data::{
    RawTimeAxis, RawVoltageScale, TimeAxisError, TimeAxisMismatch, VoltageScaleError,
};
use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

const RAW_READ_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const CSV_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const CSV_STREAM_CHUNK_SAMPLES: usize = 1_000_000;

#[derive(Debug, Clone, Copy)]
pub struct RawCsvChannel<'a> {
    pub file: &'a str,
    pub sample_count: usize,
    pub x_increment: f64,
    pub x_origin: f64,
    pub x_reference: f64,
    pub y_increment: f64,
    pub y_origin: f64,
    pub y_reference: f64,
}

pub fn write_raw_csv(
    path: &Path,
    headers: &[&str],
    raw_dir: &Path,
    channels: &[RawCsvChannel<'_>],
) -> Result<()> {
    write_raw_csv_with_chunk_samples(path, headers, raw_dir, channels, CSV_STREAM_CHUNK_SAMPLES)
}

fn write_raw_csv_with_chunk_samples(
    path: &Path,
    headers: &[&str],
    raw_dir: &Path,
    channels: &[RawCsvChannel<'_>],
    chunk_samples: usize,
) -> Result<()> {
    if channels.is_empty() {
        bail!("no channels available for raw csv output");
    }
    if headers.len() != channels.len() + 1 {
        bail!(
            "header len ({}) and raw csv column len ({}) mismatch",
            headers.len(),
            channels.len() + 1
        );
    }
    if chunk_samples == 0 {
        bail!("raw csv stream chunk size must be positive");
    }
    let chunk_buffer_bytes = chunk_samples
        .checked_mul(2)
        .ok_or_else(|| anyhow!("raw csv stream chunk byte count overflows"))?;

    let time_axis = raw_time_axis(channels[0]);
    for (idx, &channel) in channels.iter().enumerate() {
        validate_finite("y_increment", channel.y_increment, idx)?;
        validate_finite("y_origin", channel.y_origin, idx)?;
        validate_finite("y_reference", channel.y_reference, idx)?;
        validate_voltage_range(channel, idx)?;
        let channel_time_axis = raw_time_axis(channel);
        validate_time_axis_geometry(channel_time_axis, idx)?;
        validate_time_axis(time_axis, channel_time_axis, idx)?;
    }

    let sample_count = time_axis.sample_count;
    let mut readers = channels
        .iter()
        .enumerate()
        .map(|(idx, channel)| {
            let path = resolve_raw_channel_path(raw_dir, channel.file, idx)?;
            let expected_bytes = channel.sample_count.checked_mul(2).ok_or_else(|| {
                anyhow!("raw channel sample count overflows for {}", channel.file)
            })?;
            let actual_bytes = raw_channel_file_size(&path, idx)?;
            if actual_bytes != expected_bytes as u64 {
                bail!(
                    "raw channel file size mismatch for {}: expected {} bytes, got {}",
                    path.display(),
                    expected_bytes,
                    actual_bytes
                );
            }
            let file = File::open(&path)
                .with_context(|| format!("failed to open raw channel file: {}", path.display()))?;
            let opened_bytes = file
                .metadata()
                .with_context(|| format!("failed to stat raw channel file: {}", path.display()))?
                .len();
            if opened_bytes != expected_bytes as u64 {
                bail!(
                    "raw channel file size mismatch after open for {}: expected {} bytes, got {}",
                    path.display(),
                    expected_bytes,
                    opened_bytes
                );
            }
            Ok((path, BufReader::with_capacity(RAW_READ_BUFFER_BYTES, file)))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut buffers = channels
        .iter()
        .map(|_| vec![0_u8; chunk_buffer_bytes])
        .collect::<Vec<_>>();

    let file = File::create(path).context("failed to create csv file")?;
    let mut writer = BufWriter::with_capacity(CSV_WRITE_BUFFER_BYTES, file);

    for (idx, header) in headers.iter().enumerate() {
        if idx + 1 == headers.len() {
            write!(writer, "{header}")?;
        } else {
            write!(writer, "{header},")?;
        }
    }
    writeln!(writer)?;

    let mut sample_start = 0usize;
    while sample_start < sample_count {
        let current_chunk_samples = (sample_count - sample_start).min(chunk_samples);
        let chunk_bytes = current_chunk_samples * 2;
        for ((path, reader), buffer) in readers.iter_mut().zip(buffers.iter_mut()) {
            reader
                .read_exact(&mut buffer[..chunk_bytes])
                .with_context(|| format!("failed to read raw channel file: {}", path.display()))?;
        }

        for offset in 0..current_chunk_samples {
            let sample_index = sample_start + offset;
            let t = time_axis.value_at(sample_index);
            write!(writer, "{t}")?;
            for (channel, buffer) in channels.iter().zip(buffers.iter()) {
                let byte_index = offset * 2;
                let word = u16::from_le_bytes([buffer[byte_index], buffer[byte_index + 1]]);
                let voltage = voltage_scale(*channel).value_at(word);
                write!(writer, ",{voltage}")?;
            }
            writeln!(writer)?;
        }

        sample_start += current_chunk_samples;
    }

    let mut extra = [0_u8; 1];
    for (path, reader) in &mut readers {
        if reader
            .read(&mut extra)
            .with_context(|| format!("failed to verify raw channel file end: {}", path.display()))?
            != 0
        {
            bail!("raw channel file grew while streaming: {}", path.display());
        }
    }

    writer.flush()?;
    Ok(())
}

fn raw_time_axis(channel: RawCsvChannel<'_>) -> RawTimeAxis {
    RawTimeAxis {
        sample_count: channel.sample_count,
        x_increment: channel.x_increment,
        x_origin: channel.x_origin,
        x_reference: channel.x_reference,
    }
}

fn validate_time_axis_geometry(axis: RawTimeAxis, idx: usize) -> Result<()> {
    match axis.validate_geometry() {
        Ok(()) => Ok(()),
        Err(TimeAxisError::Empty) => {
            bail!("raw csv sample_count must be positive for channel index {idx}")
        }
        Err(TimeAxisError::NonPositiveIncrement(value)) => {
            bail!("raw csv x_increment must be positive for channel index {idx}: {value}")
        }
        Err(TimeAxisError::NonFiniteTime { index, value }) => {
            bail!(
                "raw csv metadata produces non-finite time for channel index {idx} at sample {index}: {value}"
            )
        }
        Err(TimeAxisError::NonIncreasing { left, right }) => {
            bail!(
                "raw csv time axis does not advance for channel index {idx} between samples {left} and {right}"
            )
        }
    }
}

fn validate_time_axis(expected: RawTimeAxis, actual: RawTimeAxis, idx: usize) -> Result<()> {
    match expected.compare(actual) {
        Ok(()) => Ok(()),
        Err(TimeAxisMismatch::SampleCount { expected, actual }) => bail!(
            "raw csv timebase mismatch for channel index {idx}: sample_count {actual} != {expected}"
        ),
        Err(TimeAxisMismatch::NonFinite { name }) => {
            bail!("raw csv timebase mismatch for channel index {idx}: {name} must be finite")
        }
        Err(TimeAxisMismatch::Value {
            name,
            expected,
            actual,
        }) => {
            bail!(
                "raw csv timebase mismatch for channel index {idx}: {name} {actual} != {expected}"
            )
        }
    }
}

fn validate_finite(name: &str, value: f64, idx: usize) -> Result<()> {
    if !value.is_finite() {
        bail!("raw csv metadata value must be finite for channel index {idx}: {name}={value}");
    }
    Ok(())
}

fn validate_voltage_range(channel: RawCsvChannel<'_>, idx: usize) -> Result<()> {
    match voltage_scale(channel).validate_geometry() {
        Ok(()) => Ok(()),
        Err(VoltageScaleError::InvalidIncrement(value)) => {
            bail!("raw csv y_increment must be positive for channel index {idx}: {value}")
        }
        Err(VoltageScaleError::NonFinite { word, value }) => {
            bail!(
                "raw csv metadata produces non-finite voltage for channel index {idx} at WORD value {word}: {value}"
            )
        }
        Err(VoltageScaleError::Indistinguishable { left, right }) => {
            bail!(
                "raw csv voltage scaling does not distinguish adjacent WORD values {left} and {right} for channel index {idx}"
            )
        }
    }
}

fn voltage_scale(channel: RawCsvChannel<'_>) -> RawVoltageScale {
    RawVoltageScale {
        y_increment: channel.y_increment,
        y_origin: channel.y_origin,
        y_reference: channel.y_reference,
    }
}

fn resolve_raw_channel_path(raw_dir: &Path, file: &str, idx: usize) -> Result<PathBuf> {
    let relative = Path::new(file);
    if relative.is_absolute() {
        bail!("raw csv channel file must be a safe relative path for channel index {idx}: {file}");
    }
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("raw csv channel file must be a safe relative path for channel index {idx}: {file}");
        }
    }
    Ok(raw_dir.join(relative))
}

fn raw_channel_file_size(path: &Path, idx: usize) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to stat raw channel file: {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "raw csv channel file must be a regular file for channel index {idx}: {}",
            path.display()
        );
    }
    Ok(metadata.len())
}

#[cfg(test)]
#[path = "raw_csv/tests.rs"]
mod tests;
