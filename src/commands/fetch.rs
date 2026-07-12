use crate::cli::FetchFormat;
use crate::commands::screenshot::{
    capture_screenshot, prepare_screenshot, prepare_screenshot_path, report_saved_screenshot,
};
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, Connection, FetchOutput, render_normalized_config};
#[cfg(test)]
use crate::constants::RAW_METADATA_FNAME;
use crate::constants::{RAW_METADATA_VERSION, T_HEADER};
use crate::ui;
use crate::utils::channels::build_channel_list;
use crate::utils::checksum::{finalize_sha256_hex, sha256_hex};
use crate::utils::csv::write_csv;
use crate::utils::raw_csv::{RawCsvChannel, write_raw_csv};
use crate::utils::raw_data::{
    RawTimeAxis, RawVoltageScale, TimeAxisError, TimeAxisMismatch, VoltageScaleError,
};
use crate::utils::time_axis::WaveformTime;
use crate::utils::waveform::{WaveformData, read_raw_waveform_channels_from_dir};
use anyhow::{Context, Result, anyhow, bail};
use instruments::rigol::{DhoHorizontalSettings, DhoRawWaveform, DhoTriggerStatus};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const RAW_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn finish(self) -> (W, String) {
        (self.inner, finalize_sha256_hex(self.hasher.finalize()))
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Debug, Serialize, Clone)]
struct RawFetchMetadata {
    schema_version: u32,
    status: &'static str,
    pmoke_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_commit: Option<&'static str>,
    timestamp: String,
    created_at_unix_seconds: u64,
    config_version: u32,
    config_file: &'static str,
    sha256: String,
    resolved_config_file: &'static str,
    resolved_config_sha256: String,
    oscilloscope: RawOscilloscopeMetadata,
    channels: Vec<RawChannelMetadata>,
}

#[derive(Debug, Serialize, Clone)]
struct RawOscilloscopeMetadata {
    idn_raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    firmware: Option<String>,
    model: String,
    connection: Connection,
    memory_depth: usize,
    waveform_mode: &'static str,
    waveform_format: &'static str,
    byte_order: &'static str,
    byte_order_source: &'static str,
    acquisition_state: &'static str,
    sample_count: usize,
    channels: Vec<u8>,
    horizontal_offset: f64,
    horizontal_scale: f64,
}

#[derive(Debug, Serialize, Clone)]
struct RawChannelMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<u8>,
    file: String,
    bytes: usize,
    sha256: String,
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

#[derive(Serialize)]
struct CsvAcquisitionManifest {
    schema_version: u32,
    pmoke_version: &'static str,
    timestamp: String,
    waveform_format: &'static str,
    file: String,
    sha256: String,
    rows: usize,
    columns: usize,
}

pub fn fetch(cfg: &Config) -> Result<()> {
    fetch_with_options(cfg, None, None)
}

fn check_acquisition_exists(cfg: &Config) -> Result<()> {
    if cfg.force {
        return Ok(());
    }
    let paths = cfg.paths();
    if paths.acquisition_dir().exists() {
        bail!(
            "acquisition directory already exists: {} (use --force to overwrite)",
            paths.acquisition_dir().display()
        );
    }
    let legacy_raw = paths.run_dir.join("raw_waveform");
    if legacy_raw.exists() {
        bail!(
            "legacy raw_waveform directory already exists: {} (use --force to overwrite)",
            legacy_raw.display()
        );
    }
    let legacy_csv = paths.run_dir.join("raw.csv");
    if legacy_csv.exists() {
        bail!(
            "legacy raw.csv already exists: {} (use --force to overwrite)",
            legacy_csv.display()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupState {
    RollbackArmed,
    RecoveryFailed,
    Invalidated,
    Restored,
}

struct AnalysisBackup {
    backup_dir: PathBuf,
    moved_items: Vec<(PathBuf, PathBuf)>,
    state: BackupState,
}

impl AnalysisBackup {
    fn create(run_dir: &Path) -> Result<Self> {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let pid = std::process::id();
        let timestamp = jiff::Timestamp::now().to_string().replace([':', '.'], "-");
        let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let backup_dir = run_dir.join(format!(
            ".analysis.backup.{}.{}.{}",
            pid, timestamp, counter
        ));
        Self::create_internal(run_dir, backup_dir)
    }

    fn create_internal(run_dir: &Path, backup_dir: PathBuf) -> Result<Self> {
        if let Ok(entries) = std::fs::read_dir(run_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().into_string().unwrap_or_default();
                if name.starts_with(".analysis.backup.") && entry.path() != backup_dir {
                    crate::ui::warn(format!(
                        "Stale analysis backup directory detected: {}. Manual recovery of previous analysis files may be possible.",
                        entry.path().display()
                    ));
                }
            }
        }

        std::fs::create_dir_all(&backup_dir)?;

        let mut backup = Self {
            backup_dir,
            moved_items: Vec::new(),
            state: BackupState::RollbackArmed,
        };

        let mut candidates = Vec::new();

        // 1. Check canonical analysis/
        let canonical = run_dir.join("analysis");
        if canonical.exists() {
            candidates.push((canonical.clone(), PathBuf::from("analysis")));
        }

        // 2. Check legacy folders/files
        let legacy_npy = run_dir.join("analysis_npy");
        if legacy_npy.exists() {
            candidates.push((legacy_npy.clone(), PathBuf::from("analysis_npy")));
        }

        let legacy_meta = run_dir.join("analysis_metadata.toml");
        if legacy_meta.exists() {
            candidates.push((legacy_meta.clone(), PathBuf::from("analysis_metadata.toml")));
        }

        let legacy_kerr = run_dir.join("kerr_results.csv");
        if legacy_kerr.exists() {
            candidates.push((legacy_kerr.clone(), PathBuf::from("kerr_results.csv")));
        }

        // Wildcards for lockin results
        if let Ok(entries) = std::fs::read_dir(run_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let is_lockin = name.starts_with("lockin_results_ch")
                        || name.starts_with("lockin_rotated_ch");
                    if is_lockin && name.ends_with(".csv") {
                        candidates.push((path.clone(), PathBuf::from(name)));
                    }
                }
            }
        }

        // Move all items into backup_dir
        for (orig, rel) in candidates {
            let dest = backup.backup_dir.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Err(err) = rename_file(&orig, &dest) {
                let rollback_res = backup.restore_internal();
                return match rollback_res {
                    Ok(()) => Err(err).with_context(|| {
                        "failed to create analysis backup; previous analysis was restored".to_string()
                    }),
                    Err(rollback_err) => Err(err).with_context(|| {
                        format!(
                            "failed to create analysis backup and rollback failed: {rollback_err}; recovery data remains at {}",
                            backup.backup_dir.display()
                        )
                    }),
                };
            }
            backup.moved_items.push((orig, rel));
        }

        Ok(backup)
    }

    fn restore_internal(&mut self) -> Result<()> {
        if self.state == BackupState::Restored || self.state == BackupState::Invalidated {
            return Ok(());
        }

        let mut failed_items = Vec::new();
        let mut errors = Vec::new();

        let items = std::mem::take(&mut self.moved_items);

        for (original, relative) in items.into_iter().rev() {
            let backup_path = self.backup_dir.join(&relative);

            if !backup_path.exists() {
                if original.exists() {
                    continue;
                }
                errors.push(format!(
                    "backup item is missing and original was not restored: {}",
                    original.display()
                ));
                failed_items.push((original, relative));
                continue;
            }

            if let Some(parent) = original.parent() {
                let create_res = std::fs::create_dir_all(parent);
                if let Err(error) = create_res {
                    errors.push(format!(
                        "failed to create restore parent for {}: {}",
                        original.display(),
                        error
                    ));
                    failed_items.push((original, relative));
                    continue;
                }
            }

            if let Err(error) = rename_file(&backup_path, &original) {
                errors.push(format!(
                    "failed to restore {} from backup: {}",
                    original.display(),
                    error
                ));
                failed_items.push((original, relative));
            }
        }

        failed_items.reverse();
        self.moved_items = failed_items;

        if self.moved_items.is_empty() {
            let _ = std::fs::remove_dir_all(&self.backup_dir);
            self.state = BackupState::Restored;
        }

        if !errors.is_empty() {
            anyhow::bail!(
                "{}; remaining backup: {}",
                errors.join("; "),
                self.backup_dir.display()
            );
        }

        Ok(())
    }

    fn restore(mut self) -> Result<()> {
        match self.restore_internal() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.state = BackupState::RecoveryFailed;
                Err(error)
            }
        }
    }

    fn commit(mut self, run_dir: &Path) -> Result<()> {
        self.state = BackupState::Invalidated;

        if self.moved_items.is_empty() {
            let _ = std::fs::remove_dir_all(&self.backup_dir);
            return Ok(());
        }
        let timestamp = jiff::Timestamp::now().to_string().replace([':', '.'], "-");
        let dest_dir = run_dir.join(format!("legacy_analysis.invalidated.{}", timestamp));
        rename_file(&self.backup_dir, &dest_dir).with_context(|| {
            format!(
                "failed to move analysis backup to invalidated folder: {}",
                dest_dir.display()
            )
        })?;
        Ok(())
    }
}

impl Drop for AnalysisBackup {
    fn drop(&mut self) {
        if matches!(self.state, BackupState::RollbackArmed) {
            let res = self.restore_internal();
            if let Err(err) = res {
                crate::ui::warn(format!(
                    "Failed to automatically restore analysis backup during drop rollback: {}",
                    err
                ));
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static MOCK_RENAME_ERROR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn rename_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if MOCK_RENAME_ERROR.with(|m| m.get()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "mock rename error",
        ));
    }
    std::fs::rename(from, to)
}

fn has_any_analysis_artifact(run_dir: &Path) -> Result<bool> {
    if run_dir.join("analysis").exists() {
        return Ok(true);
    }
    if run_dir.join("analysis_npy").exists() {
        return Ok(true);
    }
    if run_dir.join("analysis_metadata.toml").exists() {
        return Ok(true);
    }
    if run_dir.join("kerr_results.csv").exists() {
        return Ok(true);
    }
    if let Ok(entries) = std::fs::read_dir(run_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if (name_str.starts_with("lockin_results_ch")
                || name_str.starts_with("lockin_rotated_ch"))
                && name_str.ends_with(".csv")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn fetch_with_options(
    cfg: &Config,
    format: Option<FetchFormat>,
    out: Option<&Path>,
) -> Result<()> {
    if out.is_some() {
        bail!(
            "--out is not supported for fetch; \
             canonical output is acquisition/waveforms/waveform.csv"
        );
    }
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "fetch")?;

    // Preflight checks before modifying state or directory
    if !cfg.force {
        check_acquisition_exists(cfg)?;
        if has_any_analysis_artifact(&cfg.paths().run_dir)? {
            bail!(
                "analysis artifacts already exist without a valid acquisition; \
                 use --force to invalidate them or choose a new run directory"
            );
        }
    }

    crate::commands::run_dir::prepare(cfg)?;

    let backup = if cfg.force {
        Some(AnalysisBackup::create(&cfg.paths().run_dir)?)
    } else {
        None
    };

    crate::commands::run_dir::write_run_state(cfg, "acquiring", "fetch", None)?;
    let result = fetch_with_options_inner(cfg, format, out);
    if let Err(error) = result {
        let final_err = if let Some(b) = backup {
            let backup_path = b.backup_dir.clone();
            match b.restore() {
                Ok(()) => error,
                Err(restore_err) => {
                    error.context(format!(
                        "fetch failed and previous analysis could not be restored: {restore_err}; backup remains at {}",
                        backup_path.display()
                    ))
                }
            }
        } else {
            error
        };
        crate::commands::run_dir::write_run_state(cfg, "failed", "fetch", Some(&final_err))?;
        return Err(final_err);
    }

    if let Some(b) = backup {
        let backup_path = b.backup_dir.clone();
        if let Err(commit_err) = b.commit(&cfg.paths().run_dir) {
            crate::ui::warn(format!(
                "new acquisition was published, but invalidated analysis could not be archived: {commit_err}; backup remains at {}",
                backup_path.display()
            ));
        }
    }
    crate::commands::run_dir::write_run_state(cfg, "acquired", "fetch", None)?;
    Ok(())
}

fn fetch_with_options_inner(
    cfg: &Config,
    format: Option<FetchFormat>,
    out: Option<&Path>,
) -> Result<()> {
    check_acquisition_exists(cfg)?;

    let mut cfg_staging = cfg.clone();
    cfg_staging.staging_active = true;

    let staging_acquisition = cfg_staging.paths().acquisition_dir();
    if staging_acquisition.exists() {
        std::fs::remove_dir_all(&staging_acquisition)
            .context("failed to clean up previous incomplete staging directory")?;
    }

    let output = format
        .map(FetchOutput::from)
        .unwrap_or(cfg_staging.fetch.output);
    let paths = cfg_staging.paths();
    let default_csv = paths.waveform_csv();
    let default_raw = paths.acquisition_dir();
    let result = match format {
        Some(FetchFormat::Csv) => fetch_csv(&cfg_staging, out.unwrap_or(&default_csv)),
        Some(FetchFormat::Raw) => fetch_raw(&cfg_staging, out.unwrap_or(&default_raw)),
        Some(FetchFormat::CsvAndRaw) if out.is_some() => {
            bail!(
                "--out cannot be used with --format csv-and-raw; use the canonical acquisition layout"
            )
        }
        _ => match (output, out) {
            (FetchOutput::Csv, out) => fetch_csv(&cfg_staging, out.unwrap_or(&default_csv)),
            (FetchOutput::Raw, out) => fetch_raw(&cfg_staging, out.unwrap_or(&default_raw)),
            (FetchOutput::CsvAndRaw, Some(_)) => {
                let setting = if cfg_staging.version >= 4 {
                    "data.output = \"both\""
                } else {
                    "fetch.output = \"csv_and_raw\""
                };
                bail!("--out cannot be used with {setting}; use the canonical acquisition layout")
            }
            (FetchOutput::CsvAndRaw, None) => {
                fetch_csv_and_raw(&cfg_staging, &default_csv, &default_raw)
            }
        },
    };

    result?;

    if out.is_some() {
        return Ok(());
    }

    let canonical_acquisition = cfg.paths().acquisition_dir();
    crate::commands::run_dir::publish_staged_directory(
        &staging_acquisition,
        &canonical_acquisition,
        cfg.force,
    )?;

    Ok(())
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
    let screenshot_plan = cfg
        .screenshot
        .enabled
        .then(|| prepare_screenshot(cfg))
        .transpose()?;
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    if let Some(plan) = screenshot_plan {
        let saved = capture_screenshot(&mut handler, &plan, true)?;
        report_saved_screenshot(&saved);
    }

    Ok(handler)
}

fn initialize_staged_fetch_handler(cfg: &Config, tmp_dir: &Path) -> Result<OscilloscopeHandler> {
    let screenshot_plan = cfg
        .screenshot
        .enabled
        .then(|| prepare_screenshot_path(&tmp_dir.join("screenshots").join("oscilloscope.png")))
        .transpose()?;
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")
        .inspect_err(|_| {
            let _ = fs::remove_dir(tmp_dir);
        })?;
    if let Some(plan) = screenshot_plan {
        let saved = capture_screenshot(&mut handler, &plan, true)?;
        report_saved_screenshot(&saved);
    }
    Ok(handler)
}

fn fetch_csv(cfg: &Config, out: &Path) -> Result<()> {
    ensure_output_parent(out)?;
    let data = run_fetch_to_csv_path(cfg, out)?;
    write_fetched_csv(cfg, out, &data)?;
    write_csv_acquisition_manifest(cfg, out, &data)?;
    Ok(())
}

pub fn run_fetch_for_process_locked(cfg: &Config) -> Result<WaveformData> {
    crate::commands::run_dir::prepare(cfg)?;
    crate::commands::run_dir::write_run_state(cfg, "acquiring", "fetch", None)?;
    let result = run_fetch_for_process_inner(cfg);
    match &result {
        Ok(_) => crate::commands::run_dir::write_run_state(cfg, "acquired", "fetch", None)?,
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "fetch", Some(error))?
        }
    }
    result
}

fn run_fetch_for_process_inner(cfg: &Config) -> Result<WaveformData> {
    check_acquisition_exists(cfg)?;

    let mut cfg_staging = cfg.clone();
    cfg_staging.staging_active = true;

    let staging_acquisition = cfg_staging.paths().acquisition_dir();
    if staging_acquisition.exists() {
        std::fs::remove_dir_all(&staging_acquisition)?;
    }

    let paths = cfg_staging.paths();
    let csv_out = paths.waveform_csv();
    let raw_out = paths.acquisition_dir();
    let data = match cfg_staging.fetch.output {
        FetchOutput::Csv => {
            ensure_output_parent(&csv_out)?;
            let data = run_fetch_to_csv_path(&cfg_staging, &csv_out)?;
            write_fetched_csv(&cfg_staging, &csv_out, &data)?;
            write_csv_acquisition_manifest(&cfg_staging, &csv_out, &data)?;
            data
        }
        FetchOutput::Raw => fetch_raw_collect(&cfg_staging, &raw_out)?,
        FetchOutput::CsvAndRaw => fetch_csv_and_raw_collect(&cfg_staging, &csv_out, &raw_out)?,
    };

    let canonical_acquisition = cfg.paths().acquisition_dir();
    crate::commands::run_dir::publish_staged_directory(
        &staging_acquisition,
        &canonical_acquisition,
        cfg.force,
    )?;

    Ok(data)
}

fn write_csv_acquisition_manifest(cfg: &Config, csv: &Path, data: &WaveformData) -> Result<()> {
    let acquisition = cfg.paths().acquisition_dir();
    let relative = csv.strip_prefix(&acquisition).with_context(|| {
        format!(
            "waveform CSV {} is outside acquisition directory {}",
            csv.display(),
            acquisition.display()
        )
    })?;
    let manifest = CsvAcquisitionManifest {
        schema_version: 1,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        timestamp: jiff::Timestamp::now().to_string(),
        waveform_format: "csv",
        file: relative.to_string_lossy().replace('\\', "/"),
        sha256: crate::utils::checksum::file_sha256(csv)?,
        rows: data.t.len(),
        columns: data.channels.len() + 1,
    };
    let encoded = toml::to_string_pretty(&manifest)?;
    write_synced_file(&cfg.paths().acquisition_manifest(), encoded.as_bytes())
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
            write_waveform_csv_into_staging(csv_out, raw_out, &tmp_dir, &channels, &metadata)?;
            finalize_temp_dir(&tmp_dir, raw_out)?;
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
            write_waveform_csv_into_staging(csv_out, raw_out, &tmp_dir, &channels, &metadata)?;
            finalize_temp_dir(&tmp_dir, raw_out)?;
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

fn write_waveform_csv_into_staging(
    csv_out: &Path,
    raw_out: &Path,
    tmp_dir: &Path,
    channels: &[u8],
    metadata: &RawFetchMetadata,
) -> Result<()> {
    let relative_csv = csv_out.strip_prefix(raw_out).with_context(|| {
        format!(
            "waveform CSV {} must be inside acquisition directory {}",
            csv_out.display(),
            raw_out.display()
        )
    })?;
    let staged_csv = tmp_dir.join(relative_csv);
    ensure_output_parent(&staged_csv)?;
    let headers: Vec<String> = std::iter::once(T_HEADER.to_string())
        .chain(channels.iter().map(|ch| format!("ch{ch}")))
        .collect();
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let tmp_csv = write_raw_csv_temp(&staged_csv, &header_refs, tmp_dir, channels, metadata)
        .with_context(|| {
            format!(
                "raw waveform staging directory was preserved at {}",
                tmp_dir.display()
            )
        })?;

    if let Err(error) = finalize_temp_file(&tmp_csv, &staged_csv) {
        let _ = fs::remove_file(&tmp_csv);
        return Err(error.context("failed to finalize staged waveform CSV"));
    }
    Ok(())
}

#[cfg(test)]
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
            Err(rollback) => Err(error.context(format!(
                "failed to finalize csv output and restore staging: {rollback}"
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
    let time = data.t.to_vec();
    let csv_columns = csv_columns_with_time(&time, data);
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
    let config_snapshots = snapshot_configs(cfg, &cfg.paths().run_dir)?;
    let waveform_dir = dir.join("waveforms");
    fs::create_dir_all(&waveform_dir)
        .with_context(|| format!("failed to create {}", waveform_dir.display()))?;
    let idn_raw = handler
        .identify()
        .context("failed to identify oscilloscope before raw fetch")?;
    handler
        .stop()
        .context("failed to stop oscilloscope before raw fetch")?;
    ensure_scope_stopped(handler, "before raw fetch")?;
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
    let mut metadata =
        build_raw_metadata(cfg, &channels, depth, horizontal, idn_raw, config_snapshots)?;
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
        let mut channel_metadata = write_raw_channel_streamed(handler, &waveform_dir, ch, depth)?;
        channel_metadata.index = Some(ch);
        channel_metadata.file = format!("waveforms/{}", channel_metadata.file);
        validate_fetch_voltage_range(
            channel_metadata.y_increment,
            channel_metadata.y_origin,
            channel_metadata.y_reference,
            ch,
        )?;
        update_metadata_time_axis(&mut time_axis, &channel_metadata, ch)?;
        metadata.channels.push(channel_metadata);
        pb.inc(1);
    }
    ensure_scope_stopped(handler, "after all channel transfers")?;
    let final_depth = handler
        .query_memory_depth()
        .context("failed to re-query oscilloscope memory depth after raw fetch")?;
    if final_depth != depth {
        bail!("oscilloscope memory depth changed during raw fetch: {depth} -> {final_depth}");
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
    idn_raw: String,
    config_snapshots: ConfigSnapshotHashes,
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
        schema_version: RAW_METADATA_VERSION,
        status: "complete",
        pmoke_version: env!("CARGO_PKG_VERSION"),
        git_commit: option_env!("PMOKE_GIT_COMMIT"),
        timestamp: jiff::Timestamp::now().to_string(),
        created_at_unix_seconds,
        config_version: cfg.version,
        config_file: "../config.source.toml",
        sha256: config_snapshots.source,
        resolved_config_file: "../config.resolved.toml",
        resolved_config_sha256: config_snapshots.resolved,
        oscilloscope: RawOscilloscopeMetadata {
            firmware: idn_firmware(&idn_raw),
            idn_raw,
            model: osc_cfg.model.clone(),
            connection: osc_cfg.connection.clone(),
            memory_depth,
            waveform_mode: "RAW",
            waveform_format: "WORD",
            // The DHO/MHO5000 programming guide exposes WORD format but no
            // waveform byte-order command. The DHO driver decodes its fixed
            // WORD payload as least-significant byte first.
            byte_order: "little-endian",
            byte_order_source: "DHO5000 WORD protocol",
            acquisition_state: "STOP",
            sample_count: memory_depth,
            channels: channels.to_vec(),
            horizontal_offset: horizontal.offset,
            horizontal_scale: horizontal.scale,
        },
        channels: Vec::new(),
    })
}

fn ensure_scope_stopped(handler: &mut OscilloscopeHandler, context: &str) -> Result<()> {
    let status = handler
        .query_trigger_status()
        .with_context(|| format!("failed to query oscilloscope trigger status {context}"))?;
    if status != DhoTriggerStatus::Stop {
        bail!("oscilloscope must be STOP {context}, got {status:?}");
    }
    Ok(())
}

struct ConfigSnapshotHashes {
    source: String,
    resolved: String,
}

fn snapshot_configs(cfg: &Config, dir: &Path) -> Result<ConfigSnapshotHashes> {
    let contents = match &cfg.source_text {
        Some(source) => source.as_bytes().to_vec(),
        None => fs::read(&cfg.source_path).with_context(|| {
            format!(
                "failed to read source config: {}",
                cfg.source_path.display()
            )
        })?,
    };
    write_snapshot(dir, "config.source.toml", &contents)?;
    let resolved = render_normalized_config(cfg)
        .context("failed to render resolved acquisition config")?
        .into_bytes();
    write_snapshot(dir, "config.resolved.toml", &resolved)?;
    Ok(ConfigSnapshotHashes {
        source: sha256_hex(&contents),
        resolved: sha256_hex(&resolved),
    })
}

fn write_snapshot(dir: &Path, name: &str, contents: &[u8]) -> Result<()> {
    let final_path = dir.join(name);
    if final_path.exists() {
        let existing = fs::read(&final_path)
            .with_context(|| format!("failed to read {}", final_path.display()))?;
        if existing != contents {
            bail!(
                "run config snapshot differs from current config: {}",
                final_path.display()
            );
        }
        return Ok(());
    }
    let tmp_path = dir.join(format!("{name}.tmp"));
    write_synced_file(&tmp_path, contents)?;
    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            final_path.display()
        )
    })
}

fn idn_firmware(idn: &str) -> Option<String> {
    idn.split(',')
        .nth(3)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn write_synced_file(path: &Path, contents: &[u8]) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("failed to create file: {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(contents)
        .with_context(|| format!("failed to write file: {}", path.display()))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush file: {}", path.display()))?;
    let file = writer
        .into_inner()
        .map_err(|error| error.into_error())
        .with_context(|| format!("failed to finalize file: {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync file: {}", path.display()))
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
    let file = writer
        .into_inner()
        .map_err(|error| error.into_error())
        .with_context(|| {
            format!(
                "failed to finalize raw channel file: {}",
                tmp_path.display()
            )
        })?;
    file.sync_all()
        .with_context(|| format!("failed to sync raw channel file: {}", tmp_path.display()))?;

    let sha256 = sha256_hex(&raw.data);

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    Ok(RawChannelMetadata {
        index: None,
        file: fname,
        bytes: raw.data.len(),
        sha256,
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
    let buffered = BufWriter::with_capacity(RAW_WRITE_BUFFER_BYTES, file);
    let mut writer = HashingWriter::new(buffered);
    let written = handler
        .fetch_raw_word_into(ch, expected_depth, &mut writer)
        .with_context(|| format!("failed to fetch channel {ch} raw WORD"))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush raw channel file: {}", tmp_path.display()))?;
    let (buffered, sha256) = writer.finish();
    let file = buffered
        .into_inner()
        .map_err(|error| error.into_error())
        .with_context(|| {
            format!(
                "failed to finalize raw channel file: {}",
                tmp_path.display()
            )
        })?;
    file.sync_all()
        .with_context(|| format!("failed to sync raw channel file: {}", tmp_path.display()))?;

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
        index: None,
        file: fname,
        bytes: written.byte_count,
        sha256,
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
    let scale = RawVoltageScale {
        y_increment: raw.preamble.y_increment,
        y_origin: raw.preamble.y_origin,
        y_reference: raw.preamble.y_reference,
    };

    raw.data
        .chunks_exact(2)
        .map(|chunk| {
            let word = u16::from_le_bytes([chunk[0], chunk[1]]);
            scale.value_at(word)
        })
        .collect()
}

fn update_metadata_time_axis(
    time_axis: &mut Option<RawTimeAxis>,
    metadata: &RawChannelMetadata,
    ch: u8,
) -> Result<()> {
    let axis = RawTimeAxis {
        sample_count: metadata.sample_count,
        x_increment: metadata.x_increment,
        x_origin: metadata.x_origin,
        x_reference: metadata.x_reference,
    };
    validate_fetch_time_axis_geometry(axis, ch)?;
    match time_axis {
        Some(expected) => validate_fetch_time_axis(*expected, axis, ch),
        None => {
            *time_axis = Some(axis);
            Ok(())
        }
    }
}

fn update_time_axis(
    time_axis: &mut Option<RawTimeAxis>,
    preamble: &instruments::rigol::dho5108::DhoWaveformPreamble,
    sample_count: usize,
    ch: u8,
) -> Result<()> {
    let axis = RawTimeAxis {
        sample_count,
        x_increment: preamble.x_increment,
        x_origin: preamble.x_origin,
        x_reference: preamble.x_reference,
    };
    validate_fetch_time_axis_geometry(axis, ch)?;
    match time_axis {
        Some(expected) => validate_fetch_time_axis(*expected, axis, ch),
        None => {
            *time_axis = Some(axis);
            Ok(())
        }
    }
}

fn validate_fetch_time_axis_geometry(axis: RawTimeAxis, ch: u8) -> Result<()> {
    if !axis.x_increment.is_finite() || !axis.x_origin.is_finite() || !axis.x_reference.is_finite()
    {
        bail!("channel {ch} timebase values must be finite");
    }
    match axis.validate_geometry() {
        Ok(()) => Ok(()),
        Err(TimeAxisError::Empty) => bail!("channel {ch} sample_count must be positive"),
        Err(TimeAxisError::NonPositiveIncrement(value)) => {
            bail!("channel {ch} x_increment must be positive: {value}")
        }
        Err(TimeAxisError::NonFiniteTime { index, value }) => {
            bail!("channel {ch} timebase produces non-finite time at sample {index}: {value}")
        }
        Err(TimeAxisError::NonIncreasing { left, right }) => {
            bail!("channel {ch} timebase does not advance between samples {left} and {right}")
        }
    }
}

fn validate_fetch_time_axis(expected: RawTimeAxis, actual: RawTimeAxis, ch: u8) -> Result<()> {
    match expected.compare(actual) {
        Ok(()) => Ok(()),
        Err(TimeAxisMismatch::SampleCount { expected, actual }) => {
            bail!("channel {ch} timebase sample_count mismatch: {actual} != {expected}")
        }
        Err(TimeAxisMismatch::NonFinite { name }) => {
            bail!("channel {ch} timebase mismatch: {name} must be finite")
        }
        Err(TimeAxisMismatch::Value {
            name,
            expected,
            actual,
        }) => bail!("channel {ch} timebase mismatch: {name} {actual} != {expected}"),
    }
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
    let scale = RawVoltageScale {
        y_increment,
        y_origin,
        y_reference,
    };
    match scale.validate_geometry() {
        Ok(()) => Ok(()),
        Err(VoltageScaleError::InvalidIncrement(value)) => {
            bail!("channel {ch} y_increment must be positive: {value}")
        }
        Err(VoltageScaleError::NonFinite { word, value }) => {
            bail!(
                "channel {ch} voltage scaling produces non-finite voltage at WORD value {word}: {value}"
            )
        }
        Err(VoltageScaleError::Indistinguishable { left, right }) => {
            bail!(
                "channel {ch} voltage scaling does not distinguish adjacent WORD values {left} and {right}"
            )
        }
    }
}

fn csv_columns_with_time<'a>(time: &'a [f64], data: &'a WaveformData) -> Vec<&'a [f64]> {
    std::iter::once(time)
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
                .iter()
                .find(|c| c.index == Some(ch))
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
        Ok(()) => {
            if let Err(error) = sync_file(&tmp_path) {
                let _ = fs::remove_file(&tmp_path);
                return Err(
                    error.context(format!("failed to persist csv output: {}", out.display()))
                );
            }
            Ok(tmp_path)
        }
        Err(error) => {
            let _ = fs::remove_file(&tmp_path);
            Err(error.context(format!("failed to write csv output: {}", out.display())))
        }
    }
}

fn write_raw_metadata(dir: &Path, metadata: &RawFetchMetadata) -> Result<()> {
    let final_path = dir.join("manifest.toml");
    let tmp_path = dir.join("manifest.toml.tmp");
    let encoded = toml::to_string_pretty(metadata).context("failed to encode raw metadata")?;

    write_synced_file(&tmp_path, encoded.as_bytes())
        .with_context(|| format!("failed to write metadata file: {}", tmp_path.display()))?;

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
        Ok(()) => {
            if let Err(error) = sync_file(&tmp_path) {
                let _ = fs::remove_file(&tmp_path);
                return Err(
                    error.context(format!("failed to persist csv output: {}", out.display()))
                );
            }
            Ok(tmp_path)
        }
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
    })?;
    sync_parent_directory(out)
}

fn sync_file(path: &Path) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open file for sync: {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync file: {}", path.display()))
}

fn finalize_temp_dir(tmp_dir: &Path, out: &Path) -> Result<()> {
    ensure_path_not_exists(out)?;
    sync_directory(tmp_dir)?;
    fs::rename(tmp_dir, out).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_dir.display(),
            out.display()
        )
    })?;
    sync_parent_directory(out)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .with_context(|| format!("failed to open directory for sync: {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync directory: {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    sync_directory(parent)
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

    let t = WaveformTime::Uniform(
        time_axis.ok_or_else(|| anyhow!("no waveform time axis was collected"))?,
    );
    Ok(WaveformData { t, channels: data })
}

#[cfg(test)]
#[path = "fetch/tests.rs"]
mod tests;
