use crate::cli::FetchFormat;
use crate::commands::image::{capture_image, prepare_image, report_saved_image};
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, Connection, FetchOutput};
use crate::constants::{
    FETCHED_FNAME, RAW_METADATA_FNAME, RAW_METADATA_VERSION, RAW_WAVEFORM_DIR, T_HEADER,
};
use crate::ui;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::write_csv;
use crate::utils::raw_csv::{RawCsvChannel, write_raw_csv};
use crate::utils::waveform::{WaveformData, read_raw_waveform_channels_from_dir};
use anyhow::{Context, Result, anyhow, bail};
use instruments::rigol::{DhoHorizontalSettings, DhoRawWaveform};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
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

fn initialize_fetch_handler(cfg: &Config) -> Result<OscilloscopeHandler> {
    let image_plan = cfg.image.enabled.then(|| prepare_image(cfg)).transpose()?;
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    if let Some(plan) = image_plan {
        let saved = capture_image(&mut handler, &plan, true)?;
        report_saved_image(&saved);
    }

    Ok(handler)
}

fn initialize_staged_fetch_handler(cfg: &Config, tmp_dir: &Path) -> Result<OscilloscopeHandler> {
    initialize_fetch_handler(cfg).inspect_err(|_| {
        let _ = fs::remove_dir(tmp_dir);
    })
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

    let mut handler = initialize_fetch_handler(cfg)?;

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
            (depth as f64) * (channels.len() as f64) / fetch_elapsed.as_secs_f64()
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

    let mut handler = initialize_staged_fetch_handler(cfg, &tmp_dir)?;

    match fetch_raw_into_dir(cfg, &tmp_dir, &mut handler) {
        Ok(_) => {
            finalize_temp_dir(&tmp_dir, out).with_context(|| {
                format!(
                    "raw waveform staging directory was preserved at {}",
                    tmp_dir.display()
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

    let mut handler = initialize_staged_fetch_handler(cfg, &tmp_dir)?;

    match fetch_raw_into_dir(cfg, &tmp_dir, &mut handler) {
        Ok((channels, _)) => {
            finalize_temp_dir(&tmp_dir, out).with_context(|| {
                format!(
                    "raw waveform staging directory was preserved at {}",
                    tmp_dir.display()
                )
            })?;
            ui::saved(out.display().to_string());
            let data = read_raw_waveform_channels_from_dir(out, &channels).with_context(|| {
                format!(
                    "failed to read collected raw waveform data from saved raw output: {}",
                    out.display()
                )
            })?;
            Ok(data)
        }
        Err(error) => Err(error.context(format!(
            "raw waveform output was left incomplete in {}",
            tmp_dir.display()
        ))),
    }
}

fn fetch_csv_and_raw(cfg: &Config, csv_out: &Path, raw_out: &Path) -> Result<()> {
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

    let mut handler = initialize_staged_fetch_handler(cfg, &tmp_dir)?;

    match fetch_raw_into_dir(cfg, &tmp_dir, &mut handler) {
        Ok((channels, metadata)) => {
            let t_write_start = Instant::now();
            write_raw_csv_and_finalize_outputs(csv_out, raw_out, &tmp_dir, &channels, &metadata)?;
            let t_write_end = Instant::now();

            ui::saved(format!(
                "{} and {} ({})",
                csv_out.display(),
                raw_out.display(),
                ui::fmt_duration(t_write_end - t_write_start)
            ));
            Ok(())
        }
        Err(error) => Err(error.context(format!(
            "raw waveform output was left incomplete in {}",
            tmp_dir.display()
        ))),
    }
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

    let mut handler = initialize_staged_fetch_handler(cfg, &tmp_dir)?;

    match fetch_raw_into_dir(cfg, &tmp_dir, &mut handler) {
        Ok((channels, metadata)) => {
            let t_write_start = Instant::now();
            write_raw_csv_and_finalize_outputs(csv_out, raw_out, &tmp_dir, &channels, &metadata)?;
            let t_write_end = Instant::now();

            ui::saved(format!(
                "{} and {} ({})",
                csv_out.display(),
                raw_out.display(),
                ui::fmt_duration(t_write_end - t_write_start)
            ));
            let data =
                read_raw_waveform_channels_from_dir(raw_out, &channels).with_context(|| {
                    format!(
                        "failed to read collected raw waveform data from saved raw output: {}",
                        raw_out.display()
                    )
                })?;
            Ok(data)
        }
        Err(error) => Err(error.context(format!(
            "raw waveform output was left incomplete in {}",
            tmp_dir.display()
        ))),
    }
}

fn write_raw_csv_and_finalize_outputs(
    csv_out: &Path,
    raw_out: &Path,
    tmp_dir: &Path,
    channels: &[u8],
    metadata: &RawFetchMetadata,
) -> Result<()> {
    let headers: Vec<String> = std::iter::once(T_HEADER.to_string())
        .chain(channels.iter().map(|ch| format!("ch{ch}")))
        .collect();
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let tmp_csv = write_raw_csv_temp(csv_out, &header_refs, tmp_dir, channels, metadata)
        .with_context(|| {
            format!(
                "raw waveform staging directory was preserved at {}",
                tmp_dir.display()
            )
        })?;

    if let Err(error) = finalize_temp_dir(tmp_dir, raw_out) {
        let _ = fs::remove_file(&tmp_csv);
        return Err(error.context(format!(
            "raw waveform staging directory was preserved at {}",
            tmp_dir.display()
        )));
    }
    if let Err(error) = finalize_temp_file(&tmp_csv, csv_out) {
        let _ = fs::remove_file(&tmp_csv);
        return match finalize_temp_dir(raw_out, tmp_dir) {
            Ok(()) => Err(error.context(format!(
                "failed to finalize csv output; raw waveform staging directory was restored at {}",
                tmp_dir.display()
            ))),
            Err(rollback_error) => Err(error.context(format!(
                "failed to finalize csv output and failed to restore raw waveform staging directory from {} to {}: {rollback_error}",
                raw_out.display(),
                tmp_dir.display()
            ))),
        };
    }
    Ok(())
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

fn fetch_raw_into_dir(
    cfg: &Config,
    dir: &Path,
    handler: &mut OscilloscopeHandler,
) -> Result<(Vec<u8>, RawFetchMetadata)> {
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
        let channel_metadata = write_raw_channel_streamed(handler, dir, ch, depth)?;
        validate_fetch_voltage_range(
            channel_metadata.y_increment,
            channel_metadata.y_origin,
            channel_metadata.y_reference,
            ch,
        )?;
        update_metadata_time_axis(&mut time_axis, &channel_metadata, ch)?;
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
            (depth as f64) * (channels.len() as f64) / (t_fetch_end - t_fetch_start).as_secs_f64()
        ),
    );

    write_raw_metadata(dir, &metadata)?;
    Ok((channels, metadata))
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
        version: RAW_METADATA_VERSION,
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

#[cfg(test)]
fn write_raw_channel(
    dir: &Path,
    ch: u8,
    expected_depth: usize,
    raw: DhoRawWaveform,
) -> Result<RawChannelMetadata> {
    validate_raw_word_byte_count(ch, raw.data.len(), expected_depth)?;
    let sample_count = expected_depth;

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

    validate_raw_word_byte_count(ch, written.byte_count, expected_depth)?;
    let sample_count = expected_depth;

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

fn validate_raw_word_byte_count(ch: u8, byte_count: usize, expected_depth: usize) -> Result<()> {
    let expected_bytes = expected_depth
        .checked_mul(2)
        .ok_or_else(|| anyhow!("channel {ch} expected raw WORD byte count overflows"))?;
    if byte_count != expected_bytes {
        bail!(
            "channel {ch} returned {byte_count} raw WORD bytes, expected {expected_bytes} bytes ({expected_depth} samples)"
        );
    }
    Ok(())
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
        (0..self.sample_count).map(|i| self.value_at(i)).collect()
    }

    fn value_at(self, index: usize) -> f64 {
        self.x_origin + (index as f64 - self.x_reference) * self.x_increment
    }

    fn validate(self, ch: u8) -> Result<()> {
        if self.sample_count == 0 {
            bail!("channel {ch} sample_count must be positive");
        }
        if !self.x_increment.is_finite()
            || !self.x_origin.is_finite()
            || !self.x_reference.is_finite()
        {
            bail!("channel {ch} timebase values must be finite");
        }
        if self.x_increment <= 0.0 {
            bail!(
                "channel {ch} x_increment must be positive: {}",
                self.x_increment
            );
        }
        for index in [0, self.sample_count - 1] {
            let value = self.value_at(index);
            if !value.is_finite() {
                bail!("channel {ch} timebase produces non-finite time at sample {index}: {value}");
            }
        }
        if self.sample_count > 1 {
            for (left, right) in [(0, 1), (self.sample_count - 2, self.sample_count - 1)] {
                if self.value_at(right) <= self.value_at(left) {
                    bail!(
                        "channel {ch} timebase does not advance between samples {left} and {right}"
                    );
                }
            }
        }
        Ok(())
    }
}

fn update_metadata_time_axis(
    time_axis: &mut Option<FetchTimeAxis>,
    metadata: &RawChannelMetadata,
    ch: u8,
) -> Result<()> {
    let axis = FetchTimeAxis::from_metadata(metadata);
    axis.validate(ch)?;
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
    axis.validate(ch)?;
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
    if !expected.is_finite() || !actual.is_finite() {
        bail!("channel {ch} timebase mismatch: {name} must be finite");
    }
    let scale = expected.abs().max(actual.abs());
    let tolerance = (scale * 1.0e-12).max(1.0e-18);
    if (expected - actual).abs() > tolerance {
        bail!("channel {ch} timebase mismatch: {name} {actual} != {expected}");
    }
    Ok(())
}

fn validate_fetch_voltage_range(
    y_increment: f64,
    y_origin: f64,
    y_reference: f64,
    ch: u8,
) -> Result<()> {
    if !y_increment.is_finite() || !y_origin.is_finite() || !y_reference.is_finite() {
        bail!("channel {ch} voltage scaling values must be finite");
    }
    if y_increment <= 0.0 {
        bail!("channel {ch} y_increment must be positive: {y_increment}");
    }
    let voltage_at = |word: u16| (word as f64 - y_origin - y_reference) * y_increment;
    for word in [u16::MIN, u16::MAX] {
        let voltage = voltage_at(word);
        if !voltage.is_finite() {
            bail!(
                "channel {ch} voltage scaling produces non-finite voltage at WORD value {word}: {voltage}"
            );
        }
    }
    for (left, right) in [(u16::MIN, 1), (u16::MAX - 1, u16::MAX)] {
        if voltage_at(right) <= voltage_at(left) {
            bail!(
                "channel {ch} voltage scaling does not distinguish adjacent WORD values {left} and {right}"
            );
        }
    }
    Ok(())
}

fn csv_columns_with_time(data: &WaveformData) -> Vec<&[f64]> {
    std::iter::once(data.t.as_slice())
        .chain(data.channels.iter().map(Vec::as_slice))
        .collect()
}

fn write_raw_csv_temp(
    out: &Path,
    headers: &[&str],
    raw_dir: &Path,
    channels: &[u8],
    metadata: &RawFetchMetadata,
) -> Result<PathBuf> {
    let tmp_path = output_temp_file(out)?;
    ensure_path_not_exists(&tmp_path)?;
    let raw_csv_channels = channels
        .iter()
        .map(|&ch| {
            let key = format!("ch{ch}");
            let channel = metadata
                .channels
                .get(&key)
                .ok_or_else(|| anyhow!("raw channel missing in metadata: {key}"))?;
            Ok(RawCsvChannel {
                file: &channel.file,
                sample_count: channel.sample_count,
                x_increment: channel.x_increment,
                x_origin: channel.x_origin,
                x_reference: channel.x_reference,
                y_increment: channel.y_increment,
                y_origin: channel.y_origin,
                y_reference: channel.y_reference,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    match write_raw_csv(&tmp_path, headers, raw_dir, &raw_csv_channels) {
        Ok(()) => Ok(tmp_path),
        Err(error) => {
            let _ = fs::remove_file(&tmp_path);
            Err(error.context(format!("failed to write csv output: {}", out.display())))
        }
    }
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
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("output already exists: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to inspect output path: {}", path.display()))
        }
    }
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
    match finalize_temp_file(&tmp_path, out) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&tmp_path);
            Err(error)
        }
    }
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
    ensure_path_not_exists(out)?;
    fs::rename(tmp_path, out).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            out.display()
        )
    })
}

fn finalize_temp_dir(tmp_dir: &Path, out: &Path) -> Result<()> {
    ensure_path_not_exists(out)?;
    fs::rename(tmp_dir, out).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_dir.display(),
            out.display()
        )
    })
}

fn output_temp_file(out: &Path) -> Result<PathBuf> {
    let file_name = out
        .file_name()
        .ok_or_else(|| anyhow!("output path must name a file"))?;
    let parent = out.parent().unwrap_or_else(|| Path::new(""));
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(".tmp");
    Ok(parent.join(temp_name))
}

fn raw_temp_dir(out: &Path) -> Result<PathBuf> {
    let file_name = out
        .file_name()
        .ok_or_else(|| anyhow!("raw output path must name a directory"))?;
    let parent = out.parent().unwrap_or_else(|| Path::new(""));
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(".tmp");
    Ok(parent.join(temp_name))
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
        validate_fetch_voltage_range(
            raw.preamble.y_increment,
            raw.preamble.y_origin,
            raw.preamble.y_reference,
            ch,
        )?;
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
        let zero_samples = FetchTimeAxis {
            sample_count: 0,
            x_increment: 0.5,
            x_origin: 0.0,
            x_reference: 0.0,
        }
        .validate(1)
        .unwrap_err();
        assert!(
            zero_samples
                .to_string()
                .contains("sample_count must be positive")
        );

        let zero_increment = FetchTimeAxis {
            sample_count: 1,
            x_increment: 0.0,
            x_origin: 0.0,
            x_reference: 0.0,
        }
        .validate(1)
        .unwrap_err();
        assert!(
            zero_increment
                .to_string()
                .contains("x_increment must be positive")
        );

        let time_overflow = FetchTimeAxis {
            sample_count: 2,
            x_increment: f64::MAX,
            x_origin: f64::MAX,
            x_reference: 0.0,
        }
        .validate(1)
        .unwrap_err();
        assert!(time_overflow.to_string().contains("non-finite time"));

        let rounded_to_zero = FetchTimeAxis {
            sample_count: 2,
            x_increment: 1.0,
            x_origin: f64::MAX,
            x_reference: 0.0,
        }
        .validate(1)
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

        let rounded_voltage =
            validate_fetch_voltage_range(1.0, f64::MAX / 4.0, 0.0, 1).unwrap_err();
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
        write_raw_csv_and_finalize_outputs(&csv_out, &raw_out, &staging_dir, &[1], &metadata)
            .unwrap();

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
}
