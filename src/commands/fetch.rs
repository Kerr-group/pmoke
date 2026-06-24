use crate::cli::FetchFormat;
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, Connection, FetchOutput};
use crate::constants::{FETCHED_FNAME, RAW_METADATA_FNAME, RAW_WAVEFORM_DIR, T_HEADER};
use crate::ui;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::write_csv;
use crate::utils::waveform::WaveformData;
use anyhow::{Context, Result, anyhow, bail};
use instruments::rigol::{DhoHorizontalSettings, DhoRawWaveform};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const RAW_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct RawFetchMetadata {
    version: u32,
    created_at_unix_seconds: u64,
    config_version: u32,
    oscilloscope: RawOscilloscopeMetadata,
    channels: BTreeMap<String, RawChannelMetadata>,
}

#[derive(Debug, Serialize)]
struct RawOscilloscopeMetadata {
    model: String,
    connection: Connection,
    memory_depth: usize,
    waveform_mode: &'static str,
    waveform_format: &'static str,
    byte_order: &'static str,
    sample_count: usize,
    channels: Vec<u8>,
    horizontal_offset: f64,
    horizontal_scale: f64,
}

#[derive(Debug, Serialize)]
struct RawChannelMetadata {
    file: String,
    sample_count: usize,
    preamble_raw: String,
    x_increment: f64,
    x_origin: f64,
    x_reference: f64,
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
    vertical_offset: f64,
    vertical_scale: f64,
}

pub fn fetch(cfg: &Config) -> Result<()> {
    fetch_with_options(cfg, None, None)
}

pub fn fetch_with_options(
    cfg: &Config,
    format: Option<FetchFormat>,
    out: Option<&Path>,
) -> Result<()> {
    let output = format.map(FetchOutput::from).unwrap_or(cfg.fetch.output);
    match format {
        Some(FetchFormat::Csv) => fetch_csv(cfg, out.unwrap_or_else(|| Path::new(FETCHED_FNAME))),
        Some(FetchFormat::Raw) => {
            fetch_raw(cfg, out.unwrap_or_else(|| Path::new(RAW_WAVEFORM_DIR)))
        }
        Some(FetchFormat::CsvAndRaw) if out.is_some() => {
            bail!(
                "--out cannot be used with --format csv-and-raw; use the default raw.csv and raw_waveform outputs"
            )
        }
        _ => match (output, out) {
            (FetchOutput::Csv, out) => {
                fetch_csv(cfg, out.unwrap_or_else(|| Path::new(FETCHED_FNAME)))
            }
            (FetchOutput::Raw, out) => {
                fetch_raw(cfg, out.unwrap_or_else(|| Path::new(RAW_WAVEFORM_DIR)))
            }
            (FetchOutput::CsvAndRaw, Some(_)) => {
                bail!(
                    "--out cannot be used with fetch.output = \"csv_and_raw\"; use the default raw.csv and raw_waveform outputs"
                )
            }
            (FetchOutput::CsvAndRaw, None) => {
                fetch_csv_and_raw(cfg, Path::new(FETCHED_FNAME), Path::new(RAW_WAVEFORM_DIR))
            }
        },
    }
}

impl From<FetchFormat> for FetchOutput {
    fn from(value: FetchFormat) -> Self {
        match value {
            FetchFormat::Csv => Self::Csv,
            FetchFormat::Raw => Self::Raw,
            FetchFormat::CsvAndRaw => Self::CsvAndRaw,
        }
    }
}

fn fetch_csv(cfg: &Config, out: &Path) -> Result<()> {
    ensure_output_parent(out)?;
    let data = run_fetch_to_csv_path(cfg, out)?;
    write_fetched_csv(cfg, out, &data)?;
    Ok(())
}

pub fn run_fetch_for_process(cfg: &Config) -> Result<WaveformData> {
    match cfg.fetch.output {
        FetchOutput::Csv => {
            let out = Path::new(FETCHED_FNAME);
            ensure_output_parent(out)?;
            let data = run_fetch_to_csv_path(cfg, out)?;
            write_fetched_csv(cfg, out, &data)?;
            Ok(data)
        }
        FetchOutput::Raw => fetch_raw_collect(cfg, Path::new(RAW_WAVEFORM_DIR)),
        FetchOutput::CsvAndRaw => {
            fetch_csv_and_raw_collect(cfg, Path::new(FETCHED_FNAME), Path::new(RAW_WAVEFORM_DIR))
        }
    }
}

fn run_fetch_to_csv_path(cfg: &Config, out: &Path) -> Result<WaveformData> {
    ensure_path_not_exists(out)?;

    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    let depth = handler
        .query_memory_depth()
        .context("failed to query oscilloscope memory depth")?;
    if depth == 0 {
        bail!("oscilloscope returned zero memory depth");
    }

    let channels = build_channel_list(cfg)?;
    let pb = ui::progress(
        format!("fetching {depth} samples from {} channels", channels.len()),
        channels.len() as u64,
    );

    let t_fetch_start = Instant::now();
    let data = fetch_all_channels(&mut handler, &channels, depth, &pb)?;
    let t_fetch_end = Instant::now();

    let fetch_elapsed = t_fetch_end - t_fetch_start;

    ui::finish_success(
        pb,
        format!(
            "fetched {} samples from {} channels ({}, {:.2} samples/sec)",
            depth,
            channels.len(),
            ui::fmt_duration(fetch_elapsed),
            (depth * channels.len()) as f64 / fetch_elapsed.as_secs_f64()
        ),
    );
    Ok(data)
}

fn fetch_raw(cfg: &Config, out: &Path) -> Result<()> {
    ensure_path_not_exists(out)?;
    ensure_output_parent(out)?;

    let tmp_dir = raw_temp_dir(out)?;
    ensure_path_not_exists(&tmp_dir)?;
    fs::create_dir(&tmp_dir).with_context(|| {
        format!(
            "failed to create temp output directory: {}",
            tmp_dir.display()
        )
    })?;

    match fetch_raw_into_dir(cfg, &tmp_dir) {
        Ok(()) => {
            fs::rename(&tmp_dir, out).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    tmp_dir.display(),
                    out.display()
                )
            })?;
            ui::saved(out.display().to_string());
            Ok(())
        }
        Err(error) => Err(error.context(format!(
            "raw waveform output was left incomplete in {}",
            tmp_dir.display()
        ))),
    }
}

fn fetch_raw_collect(cfg: &Config, out: &Path) -> Result<WaveformData> {
    ensure_path_not_exists(out)?;
    ensure_output_parent(out)?;

    let tmp_dir = raw_temp_dir(out)?;
    ensure_path_not_exists(&tmp_dir)?;
    fs::create_dir(&tmp_dir).with_context(|| {
        format!(
            "failed to create temp output directory: {}",
            tmp_dir.display()
        )
    })?;

    match fetch_raw_into_dir_and_collect_csv(cfg, &tmp_dir, true) {
        Ok((_, data)) => {
            fs::rename(&tmp_dir, out).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    tmp_dir.display(),
                    out.display()
                )
            })?;
            ui::saved(out.display().to_string());
            Ok(data)
        }
        Err(error) => Err(error.context(format!(
            "raw waveform output was left incomplete in {}",
            tmp_dir.display()
        ))),
    }
}

fn fetch_csv_and_raw(cfg: &Config, csv_out: &Path, raw_out: &Path) -> Result<()> {
    fetch_csv_and_raw_collect(cfg, csv_out, raw_out).map(|_| ())
}

fn fetch_csv_and_raw_collect(cfg: &Config, csv_out: &Path, raw_out: &Path) -> Result<WaveformData> {
    ensure_output_parent(csv_out)?;
    ensure_path_not_exists(csv_out)?;
    ensure_path_not_exists(raw_out)?;
    ensure_output_parent(raw_out)?;

    let tmp_dir = raw_temp_dir(raw_out)?;
    ensure_path_not_exists(&tmp_dir)?;
    fs::create_dir(&tmp_dir).with_context(|| {
        format!(
            "failed to create temp output directory: {}",
            tmp_dir.display()
        )
    })?;

    match fetch_raw_into_dir_and_collect_csv(cfg, &tmp_dir, true) {
        Ok((channels, data)) => {
            let t_write_start = Instant::now();
            let headers: Vec<String> = std::iter::once(T_HEADER.to_string())
                .chain(channels.iter().map(|ch| format!("ch{ch}")))
                .collect();
            let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();
            let csv_columns = csv_columns_with_time(&data);
            let tmp_csv = write_csv_temp(csv_out, &header_refs, &csv_columns)?;
            if let Err(error) = fs::rename(&tmp_dir, raw_out).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    tmp_dir.display(),
                    raw_out.display()
                )
            }) {
                let _ = fs::remove_file(&tmp_csv);
                return Err(error);
            }
            finalize_temp_file(&tmp_csv, csv_out)?;
            let t_write_end = Instant::now();

            ui::saved(format!(
                "{} and {} ({})",
                csv_out.display(),
                raw_out.display(),
                ui::fmt_duration(t_write_end - t_write_start)
            ));
            Ok(data)
        }
        Err(error) => Err(error.context(format!(
            "raw waveform output was left incomplete in {}",
            tmp_dir.display()
        ))),
    }
}

fn write_fetched_csv(cfg: &Config, out: &Path, data: &WaveformData) -> Result<()> {
    let channels = build_channel_list(cfg)?;
    let headers: Vec<String> = std::iter::once(T_HEADER.to_string())
        .chain(channels.iter().map(|ch| format!("ch{ch}")))
        .collect();
    let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    let t_write_start = Instant::now();
    let csv_columns = csv_columns_with_time(data);
    write_csv_atomic(out, &header_refs, &csv_columns)?;
    let t_write_end = Instant::now();

    ui::saved(format!(
        "{} ({})",
        out.display(),
        ui::fmt_duration(t_write_end - t_write_start)
    ));
    Ok(())
}

fn fetch_raw_into_dir(cfg: &Config, dir: &Path) -> Result<()> {
    fetch_raw_into_dir_and_collect_csv(cfg, dir, false).map(|_| ())
}

fn fetch_raw_into_dir_and_collect_csv(
    cfg: &Config,
    dir: &Path,
    collect_csv: bool,
) -> Result<(Vec<u8>, WaveformData)> {
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    let depth = handler
        .query_memory_depth()
        .context("failed to query oscilloscope memory depth")?;
    if depth == 0 {
        bail!("oscilloscope returned zero memory depth");
    }
    let channels = build_channel_list(cfg)?;
    let horizontal = handler
        .query_horizontal_settings()
        .context("failed to query oscilloscope horizontal settings")?;
    let mut metadata = build_raw_metadata(cfg, &channels, depth, horizontal)?;
    let mut csv_channels: Vec<Vec<f64>> = if collect_csv {
        Vec::with_capacity(channels.len())
    } else {
        Vec::new()
    };
    let mut time_axis = None;

    let pb = ui::progress(
        format!(
            "fetching raw WORD {depth} samples from {} channels",
            channels.len()
        ),
        channels.len() as u64,
    );

    let t_fetch_start = Instant::now();
    for &ch in &channels {
        pb.set_message(format!("fetching ch{ch} raw WORD"));
        let (channel_metadata, csv_channel) = if collect_csv {
            let raw = handler
                .fetch_raw_word(ch, depth)
                .with_context(|| format!("failed to fetch channel {ch} raw WORD"))?;
            let csv_channel = convert_raw_word_to_voltages(&raw);
            let channel_metadata = write_raw_channel(dir, ch, depth, raw)?;
            (channel_metadata, Some(csv_channel))
        } else {
            (
                write_raw_channel_streamed(&mut handler, dir, ch, depth)?,
                None,
            )
        };
        update_metadata_time_axis(&mut time_axis, &channel_metadata, ch)?;
        if let Some(csv_channel) = csv_channel {
            csv_channels.push(csv_channel);
        }
        metadata
            .channels
            .insert(format!("ch{ch}"), channel_metadata);
        pb.inc(1);
    }
    let t_fetch_end = Instant::now();

    ui::finish_success(
        pb,
        format!(
            "fetched raw WORD {} samples from {} channels ({}, {:.2} samples/sec)",
            depth,
            channels.len(),
            ui::fmt_duration(t_fetch_end - t_fetch_start),
            (depth * channels.len()) as f64 / (t_fetch_end - t_fetch_start).as_secs_f64()
        ),
    );

    write_raw_metadata(dir, &metadata)?;
    let t = if collect_csv {
        time_axis
            .ok_or_else(|| anyhow!("no waveform time axis was collected"))?
            .build()
    } else {
        Vec::new()
    };
    Ok((
        channels,
        WaveformData {
            t,
            channels: csv_channels,
        },
    ))
}

fn build_raw_metadata(
    cfg: &Config,
    channels: &[u8],
    memory_depth: usize,
    horizontal: DhoHorizontalSettings,
) -> Result<RawFetchMetadata> {
    let osc_cfg = &cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("Instruments configuration is missing."))?
        .oscilloscope;

    let created_at_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_secs();

    Ok(RawFetchMetadata {
        version: 1,
        created_at_unix_seconds,
        config_version: cfg.version,
        oscilloscope: RawOscilloscopeMetadata {
            model: osc_cfg.model.clone(),
            connection: osc_cfg.connection.clone(),
            memory_depth,
            waveform_mode: "RAW",
            waveform_format: "WORD",
            byte_order: "little-endian",
            sample_count: memory_depth,
            channels: channels.to_vec(),
            horizontal_offset: horizontal.offset,
            horizontal_scale: horizontal.scale,
        },
        channels: BTreeMap::new(),
    })
}

fn write_raw_channel(
    dir: &Path,
    ch: u8,
    expected_depth: usize,
    raw: DhoRawWaveform,
) -> Result<RawChannelMetadata> {
    let sample_count = raw.data.len() / 2;
    if sample_count != expected_depth {
        bail!("channel {ch} returned {sample_count} raw WORD samples, expected {expected_depth}");
    }

    let fname = format!("ch{ch}.u16le");
    let final_path = dir.join(&fname);
    let tmp_path = dir.join(format!("{fname}.tmp"));

    let file = File::create(&tmp_path)
        .with_context(|| format!("failed to create raw channel file: {}", tmp_path.display()))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(&raw.data)
        .with_context(|| format!("failed to write raw channel file: {}", tmp_path.display()))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush raw channel file: {}", tmp_path.display()))?;
    drop(writer);

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    Ok(RawChannelMetadata {
        file: fname,
        sample_count,
        preamble_raw: raw.preamble.raw,
        x_increment: raw.preamble.x_increment,
        x_origin: raw.preamble.x_origin,
        x_reference: raw.preamble.x_reference,
        y_increment: raw.preamble.y_increment,
        y_origin: raw.preamble.y_origin,
        y_reference: raw.preamble.y_reference,
        vertical_offset: raw.preamble.vertical_offset,
        vertical_scale: raw.preamble.vertical_scale,
    })
}

fn write_raw_channel_streamed(
    handler: &mut OscilloscopeHandler,
    dir: &Path,
    ch: u8,
    expected_depth: usize,
) -> Result<RawChannelMetadata> {
    let fname = format!("ch{ch}.u16le");
    let final_path = dir.join(&fname);
    let tmp_path = dir.join(format!("{fname}.tmp"));

    let file = File::create(&tmp_path)
        .with_context(|| format!("failed to create raw channel file: {}", tmp_path.display()))?;
    let mut writer = BufWriter::with_capacity(RAW_WRITE_BUFFER_BYTES, file);
    let written = handler
        .fetch_raw_word_into(ch, expected_depth, &mut writer)
        .with_context(|| format!("failed to fetch channel {ch} raw WORD"))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush raw channel file: {}", tmp_path.display()))?;
    drop(writer);

    let sample_count = written.byte_count / 2;
    if sample_count != expected_depth {
        bail!("channel {ch} returned {sample_count} raw WORD samples, expected {expected_depth}");
    }

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    Ok(RawChannelMetadata {
        file: fname,
        sample_count,
        preamble_raw: written.preamble.raw,
        x_increment: written.preamble.x_increment,
        x_origin: written.preamble.x_origin,
        x_reference: written.preamble.x_reference,
        y_increment: written.preamble.y_increment,
        y_origin: written.preamble.y_origin,
        y_reference: written.preamble.y_reference,
        vertical_offset: written.preamble.vertical_offset,
        vertical_scale: written.preamble.vertical_scale,
    })
}

fn convert_raw_word_to_voltages(raw: &DhoRawWaveform) -> Vec<f64> {
    let y_inc = raw.preamble.y_increment;
    let y_ori = raw.preamble.y_origin;
    let y_ref = raw.preamble.y_reference;

    raw.data
        .chunks_exact(2)
        .map(|chunk| {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]) as f64;
            (v - y_ori - y_ref) * y_inc
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct FetchTimeAxis {
    sample_count: usize,
    x_increment: f64,
    x_origin: f64,
    x_reference: f64,
}

impl FetchTimeAxis {
    fn from_preamble(
        preamble: &instruments::rigol::dho5108::DhoWaveformPreamble,
        sample_count: usize,
    ) -> Self {
        Self {
            sample_count,
            x_increment: preamble.x_increment,
            x_origin: preamble.x_origin,
            x_reference: preamble.x_reference,
        }
    }

    fn from_metadata(metadata: &RawChannelMetadata) -> Self {
        Self {
            sample_count: metadata.sample_count,
            x_increment: metadata.x_increment,
            x_origin: metadata.x_origin,
            x_reference: metadata.x_reference,
        }
    }

    fn build(self) -> Vec<f64> {
        (0..self.sample_count)
            .map(|i| self.x_origin + (i as f64 - self.x_reference) * self.x_increment)
            .collect()
    }
}

fn update_metadata_time_axis(
    time_axis: &mut Option<FetchTimeAxis>,
    metadata: &RawChannelMetadata,
    ch: u8,
) -> Result<()> {
    let axis = FetchTimeAxis::from_metadata(metadata);
    match time_axis {
        Some(expected) => validate_fetch_time_axis(*expected, axis, ch),
        None => {
            *time_axis = Some(axis);
            Ok(())
        }
    }
}

fn update_time_axis(
    time_axis: &mut Option<FetchTimeAxis>,
    preamble: &instruments::rigol::dho5108::DhoWaveformPreamble,
    sample_count: usize,
    ch: u8,
) -> Result<()> {
    let axis = FetchTimeAxis::from_preamble(preamble, sample_count);
    match time_axis {
        Some(expected) => validate_fetch_time_axis(*expected, axis, ch),
        None => {
            *time_axis = Some(axis);
            Ok(())
        }
    }
}

fn validate_fetch_time_axis(expected: FetchTimeAxis, actual: FetchTimeAxis, ch: u8) -> Result<()> {
    if expected.sample_count != actual.sample_count {
        bail!(
            "channel {ch} timebase sample_count mismatch: {} != {}",
            actual.sample_count,
            expected.sample_count
        );
    }
    validate_close("x_increment", expected.x_increment, actual.x_increment, ch)?;
    validate_close("x_origin", expected.x_origin, actual.x_origin, ch)?;
    validate_close("x_reference", expected.x_reference, actual.x_reference, ch)?;
    Ok(())
}

fn validate_close(name: &str, expected: f64, actual: f64, ch: u8) -> Result<()> {
    let scale = expected.abs().max(actual.abs());
    let tolerance = (scale * 1.0e-12).max(1.0e-18);
    if (expected - actual).abs() > tolerance {
        bail!("channel {ch} timebase mismatch: {name} {actual} != {expected}");
    }
    Ok(())
}

fn csv_columns_with_time(data: &WaveformData) -> Vec<&[f64]> {
    std::iter::once(data.t.as_slice())
        .chain(data.channels.iter().map(Vec::as_slice))
        .collect()
}

fn write_raw_metadata(dir: &Path, metadata: &RawFetchMetadata) -> Result<()> {
    let final_path = dir.join(RAW_METADATA_FNAME);
    let tmp_path = dir.join(format!("{RAW_METADATA_FNAME}.tmp"));
    let encoded = toml::to_string_pretty(metadata).context("failed to encode raw metadata")?;

    let file = File::create(&tmp_path)
        .with_context(|| format!("failed to create metadata file: {}", tmp_path.display()))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(encoded.as_bytes())
        .with_context(|| format!("failed to write metadata file: {}", tmp_path.display()))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush metadata file: {}", tmp_path.display()))?;
    drop(writer);

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

fn ensure_path_not_exists(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("output already exists: {}", path.display());
    }
    Ok(())
}

fn ensure_output_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output parent: {}", parent.display()))?;
    }
    Ok(())
}

fn write_csv_atomic<C>(out: &Path, headers: &[&str], data: &[C]) -> Result<()>
where
    C: AsRef<[f64]>,
{
    let tmp_path = write_csv_temp(out, headers, data)?;
    finalize_temp_file(&tmp_path, out)
}

fn write_csv_temp<C>(out: &Path, headers: &[&str], data: &[C]) -> Result<PathBuf>
where
    C: AsRef<[f64]>,
{
    let tmp_path = output_temp_file(out)?;
    ensure_path_not_exists(&tmp_path)?;

    match write_csv(&tmp_path, headers, data) {
        Ok(()) => Ok(tmp_path),
        Err(error) => {
            let _ = fs::remove_file(&tmp_path);
            Err(error.context(format!("failed to write csv output: {}", out.display())))
        }
    }
}

fn finalize_temp_file(tmp_path: &Path, out: &Path) -> Result<()> {
    fs::rename(tmp_path, out).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            out.display()
        )
    })
}

fn output_temp_file(out: &Path) -> Result<PathBuf> {
    let file_name = out
        .file_name()
        .ok_or_else(|| anyhow!("output path must name a file"))?
        .to_string_lossy();
    let parent = out.parent().unwrap_or_else(|| Path::new(""));
    Ok(parent.join(format!(".{file_name}.tmp")))
}

fn raw_temp_dir(out: &Path) -> Result<PathBuf> {
    let file_name = out
        .file_name()
        .ok_or_else(|| anyhow!("raw output path must name a directory"))?
        .to_string_lossy();
    let parent = out.parent().unwrap_or_else(|| Path::new(""));
    Ok(parent.join(format!(".{file_name}.tmp")))
}

pub fn fetch_all_channels(
    handler: &mut OscilloscopeHandler,
    channels: &[u8],
    depth: usize,
    progress: &indicatif::ProgressBar,
) -> Result<WaveformData> {
    let mut data: Vec<Vec<f64>> = Vec::with_capacity(channels.len());
    let mut time_axis = None;

    for &ch in channels {
        progress.set_message(format!("fetching ch{ch}"));
        let raw = handler
            .fetch_raw_word(ch, depth)
            .with_context(|| format!("failed to fetch channel {ch}"))?;
        update_time_axis(&mut time_axis, &raw.preamble, depth, ch)?;
        let v = convert_raw_word_to_voltages(&raw);

        if v.len() != depth {
            bail!(
                "channel {ch} returned {} samples, expected {}",
                v.len(),
                depth
            );
        }

        data.push(v);
        progress.inc(1);
    }

    let t = time_axis
        .ok_or_else(|| anyhow!("no waveform time axis was collected"))?
        .build();
    Ok(WaveformData { t, channels: data })
}

#[cfg(test)]
mod tests {
    use super::*;
    use instruments::rigol::dho5108::DhoWaveformPreamble;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn write_raw_channel_preserves_original_word_bytes() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();

        let raw_bytes = vec![0x34, 0x12, 0x78, 0x56];
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
    fn raw_metadata_serializes_horizontal_settings() {
        let metadata = RawFetchMetadata {
            version: 1,
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

        let encoded = toml::to_string_pretty(&metadata).unwrap();

        assert!(encoded.contains("horizontal_offset = -0.03"));
        assert!(encoded.contains("horizontal_scale = 0.005"));
    }

    #[test]
    fn ensure_output_parent_creates_missing_parent_directories() {
        let dir = unique_test_dir();
        let output = dir.join("nested").join("raw.csv");

        ensure_output_parent(&output).unwrap();

        assert!(dir.join("nested").is_dir());
        fs::remove_dir_all(dir).unwrap();
    }

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pmoke_raw_fetch_test_{}_{}",
            std::process::id(),
            nanos
        ))
    }
}
