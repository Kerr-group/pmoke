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

#[derive(Debug, Clone, Copy)]
struct TimeAxis {
    sample_count: usize,
    x_increment: f64,
    x_origin: f64,
    x_reference: f64,
}

impl TimeAxis {
    fn from_channel(channel: RawCsvChannel<'_>) -> Self {
        Self {
            sample_count: channel.sample_count,
            x_increment: channel.x_increment,
            x_origin: channel.x_origin,
            x_reference: channel.x_reference,
        }
    }

    fn value_at(self, index: usize) -> f64 {
        self.x_origin + (index as f64 - self.x_reference) * self.x_increment
    }

    fn validate(self, idx: usize) -> Result<()> {
        if self.sample_count == 0 {
            bail!("raw csv sample_count must be positive for channel index {idx}");
        }
        if self.x_increment <= 0.0 {
            bail!(
                "raw csv x_increment must be positive for channel index {idx}: {}",
                self.x_increment
            );
        }
        for index in [0, self.sample_count - 1] {
            let value = self.value_at(index);
            if !value.is_finite() {
                bail!(
                    "raw csv metadata produces non-finite time for channel index {idx} at sample {index}: {value}"
                );
            }
        }
        if self.sample_count > 1 {
            for (left, right) in [(0, 1), (self.sample_count - 2, self.sample_count - 1)] {
                if self.value_at(right) <= self.value_at(left) {
                    bail!(
                        "raw csv time axis does not advance for channel index {idx} between samples {left} and {right}"
                    );
                }
            }
        }
        Ok(())
    }
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

    let time_axis = TimeAxis::from_channel(channels[0]);
    for (idx, &channel) in channels.iter().enumerate() {
        validate_finite("y_increment", channel.y_increment, idx)?;
        validate_finite("y_origin", channel.y_origin, idx)?;
        validate_finite("y_reference", channel.y_reference, idx)?;
        validate_voltage_range(channel, idx)?;
        let channel_time_axis = TimeAxis::from_channel(channel);
        channel_time_axis.validate(idx)?;
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
                let word = u16::from_le_bytes([buffer[byte_index], buffer[byte_index + 1]]) as f64;
                let voltage = (word - channel.y_origin - channel.y_reference) * channel.y_increment;
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

fn validate_time_axis(expected: TimeAxis, actual: TimeAxis, idx: usize) -> Result<()> {
    if expected.sample_count != actual.sample_count {
        bail!(
            "raw csv timebase mismatch for channel index {idx}: sample_count {} != {}",
            actual.sample_count,
            expected.sample_count
        );
    }
    validate_close("x_increment", expected.x_increment, actual.x_increment, idx)?;
    validate_close("x_origin", expected.x_origin, actual.x_origin, idx)?;
    validate_close("x_reference", expected.x_reference, actual.x_reference, idx)?;
    Ok(())
}

fn validate_close(name: &str, expected: f64, actual: f64, idx: usize) -> Result<()> {
    validate_finite(name, expected, idx)?;
    validate_finite(name, actual, idx)?;
    let scale = expected.abs().max(actual.abs());
    let tolerance = (scale * 1.0e-12).max(1.0e-18);
    if (expected - actual).abs() > tolerance {
        bail!("raw csv timebase mismatch for channel index {idx}: {name} {actual} != {expected}");
    }
    Ok(())
}

fn validate_finite(name: &str, value: f64, idx: usize) -> Result<()> {
    if !value.is_finite() {
        bail!("raw csv metadata value must be finite for channel index {idx}: {name}={value}");
    }
    Ok(())
}

fn validate_voltage_range(channel: RawCsvChannel<'_>, idx: usize) -> Result<()> {
    if channel.y_increment <= 0.0 {
        bail!(
            "raw csv y_increment must be positive for channel index {idx}: {}",
            channel.y_increment
        );
    }
    let voltage_at =
        |word: u16| (word as f64 - channel.y_origin - channel.y_reference) * channel.y_increment;
    for word in [u16::MIN, u16::MAX] {
        let voltage = voltage_at(word);
        if !voltage.is_finite() {
            bail!(
                "raw csv metadata produces non-finite voltage for channel index {idx} at WORD value {word}: {voltage}"
            );
        }
    }
    for (left, right) in [(u16::MIN, 1), (u16::MAX - 1, u16::MAX)] {
        if voltage_at(right) <= voltage_at(left) {
            bail!(
                "raw csv voltage scaling does not distinguish adjacent WORD values {left} and {right} for channel index {idx}"
            );
        }
    }
    Ok(())
}

fn resolve_raw_channel_path(raw_dir: &Path, file: &str, idx: usize) -> Result<PathBuf> {
    let relative = Path::new(file);
    let mut components = relative.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        bail!("raw csv channel file must be a plain file name for channel index {idx}: {file}");
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
mod tests {
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

        let error =
            validate_voltage_range(channel("ch1.u16le", 1, 0.0, 0.0, 0.0, 0.0), 0).unwrap_err();
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
        assert!(error.to_string().contains("plain file name"));

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
}
