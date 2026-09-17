pub mod debug;
pub mod estimator_snapshot;
pub mod joint;
pub mod lockin_core;
pub mod lockin_params;
pub mod lockin_plot;
pub mod model_loading;
pub mod provenance;
pub mod reference;
pub mod resolve;
pub mod save;
pub mod stride;

use crate::config::Config;
use crate::constants::{HARMONICS, LI_HEADER};
use crate::lockin::lockin_params::LockinParams;
use crate::lockin::provenance::LockinProvenance;
use crate::lockin::reference::ref_analysis::RefFitParams;
use crate::lockin::reference::run_fit_ref_core;
use crate::lockin::save::{get_li_headers, write_li_results};
use crate::lockin::stride::{li_stride_2d, li_stride_time};
use crate::sensor::{SensorOutput, run_sensor};
use crate::utils::time_axis::TimeAxisRef;
use crate::utils::waveform::read_all_fetched_waveforms;
use crate::{plot, ui};
use anyhow::{Context, Result, anyhow, bail};
use rayon::prelude::*;

pub struct LockinProcessOutput {
    pub result: Vec<Vec<Vec<f64>>>,
    /// Per-channel quality rows for GLS executions; None for boxcar_legacy.
    pub quality: Option<Vec<Vec<crate::lockin::joint::QualityRow>>>,
    /// Per-channel, per-output XY covariance for GLS executions; None for
    /// boxcar_legacy.
    pub covariance: Option<Vec<crate::lockin::joint::XyCovariances>>,
    /// Per-channel frozen estimator snapshots; None for boxcar_legacy.
    pub estimator_snapshots: Option<Vec<crate::lockin::estimator_snapshot::EstimatorSnapshot>>,
    /// Per-channel retained calibration bindings (exact validated bytes);
    /// None for boxcar_legacy.
    pub bindings: Option<Vec<crate::lockin::joint::ModelBinding>>,
    pub base_index_range: (usize, usize),
    pub output_index_range: (usize, usize),
    pub provenance: LockinProvenance,
}

/// Frozen inputs shared by the signal readout and the lock-in demodulation:
/// the full-rate sensor series strided onto the lock-in grid, the fitted
/// reference, and the prepared output-grid geometry. Prepared once per
/// pipeline, after the sensor stage and before any consumer.
pub struct LockinPreparation {
    pub t_stride: Vec<f64>,
    pub sensor_rate_stride: Vec<Vec<f64>>,
    pub sensor_integral_stride: Vec<Vec<f64>>,
    pub reference: RefFitParams,
    /// Inclusive output-grid index range shared by every consumer.
    pub index_range: (usize, usize),
    signal_ch: Vec<u8>,
    signal_idx: Vec<usize>,
}

pub struct LockinStageOutput {
    pub result: Vec<Vec<Vec<f64>>>,
    pub provenance: LockinProvenance,
}

type LockinRunOutput = (
    Vec<f64>,
    Vec<Vec<f64>>,
    Vec<Vec<f64>>,
    Vec<Vec<Vec<f64>>>,
    RefFitParams,
    LockinProvenance,
);

pub fn run(cfg: &Config) -> Result<()> {
    let pb = ui::spinner("reading fetched waveform data");
    let t0 = std::time::Instant::now();
    let data = read_all_fetched_waveforms(cfg)?;
    let elapsed_read = t0.elapsed();

    ui::finish_read(
        pb,
        format!(
            "fetched data: {} channels, {} samples ({})",
            data.channels.len(),
            data.channels.first().map_or(0, Vec::len),
            ui::fmt_duration(elapsed_read)
        ),
    );

    if data.channels.is_empty() {
        bail!("Fetched data is empty, cannot extract channels.");
    }

    let _ = run_li(cfg, &data.t, &data.channels)?;

    Ok(())
}

/// Sensor stage over already-loaded waveform data: resolves the configured
/// sensor columns and runs the reference-free sensor analysis. Kept separate
/// from the reference/grid preparation so the analyze pipeline executes
/// sensor -> reference preparation -> signal -> lock-in while sharing one
/// sensor pass.
pub fn run_sensor_stage<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    data: &[Vec<f64>],
) -> Result<SensorOutput> {
    let (sensor_ch, sensor_idx) = resolve::sensor_column_indices(cfg)?;
    check_column_bounds(data, sensor_idx.iter().copied())?;
    let sensor_data: Vec<&[f64]> = sensor_idx.iter().map(|&idx| data[idx].as_slice()).collect();
    run_sensor(cfg, t, &sensor_data, &sensor_ch)
}

/// Reference fit plus the immutable analysis-window/output-grid preparation
/// reused by the signal readout and the lock-in demodulation: the full-rate
/// sensor series strided onto the lock-in grid, the fitted reference
/// parameters, and the frozen output index range. The grid is validated
/// before any consumer runs; lock-in demodulation and signal averaging must
/// not run early or repeat the sensor/reference work.
pub fn prepare_lockin_grid<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    data: &[Vec<f64>],
    sensor: &SensorOutput,
) -> Result<LockinPreparation> {
    let t = t.into();
    let (_, ref_idx) = resolve::reference_column_index(cfg)?;
    let (signal_ch, signal_idx) = resolve::signal_column_indices(cfg)?;
    check_column_bounds(
        data,
        signal_idx.iter().copied().chain(std::iter::once(ref_idx)),
    )?;

    // Reference analysis (one fit per pipeline).
    let reference = run_fit_ref_core(cfg, t, data[ref_idx].as_slice())?;

    // Stride the full-rate sensor series onto the lock-in grid here, so the
    // signal and lock-in inputs are identical to the pre-decoupling layout.
    let t_stride = li_stride_time(cfg, t, reference.f_ref)?;
    let sensor_rate_stride = li_stride_2d(cfg, t, &sensor.rate, reference.f_ref)?;
    let sensor_integral_stride = li_stride_2d(cfg, t, &sensor.integral, reference.f_ref)?;

    // Freeze the output grid before any consumer runs: the lock-in result and
    // every downstream artifact must cover exactly this index range.
    let dt = t
        .dt()
        .ok_or_else(|| anyhow!("lock-in time axis must contain at least two samples"))?;
    let params = LockinParams::from_geometry(t.len(), dt, reference.f_ref, &cfg.lockin)?;
    if params.i_start > params.i_end {
        bail!(
            "lock-in output range is empty after edge trimming: index_range=({}, {}); reduce lpf_half_window_cycles or lockin.window.half_window_cycles, or use a longer trace",
            params.i_start,
            params.i_end
        );
    }
    let index_range = (params.i_start, params.i_end);
    let expected_len = index_range.1 - index_range.0 + 1;
    if t_stride.len() != expected_len {
        bail!(
            "time stride length ({}) does not match the prepared output grid {:?} length ({expected_len})",
            t_stride.len(),
            index_range
        );
    }
    for column in sensor_rate_stride
        .iter()
        .chain(sensor_integral_stride.iter())
    {
        if column.len() != t_stride.len() {
            bail!(
                "sensor stride length ({}) does not match time stride length ({})",
                column.len(),
                t_stride.len()
            );
        }
    }

    Ok(LockinPreparation {
        t_stride,
        sensor_rate_stride,
        sensor_integral_stride,
        reference,
        index_range,
        signal_ch,
        signal_idx,
    })
}

/// Lock-in demodulation on the frozen preparation: computes the XY results
/// for every configured lock-in channel, verifies that the executed geometry
/// is the prepared one, then saves the lock-in artifacts and the combined
/// plot.
pub fn execute_lockin<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    data: &[Vec<f64>],
    preparation: &LockinPreparation,
) -> Result<LockinStageOutput> {
    let t = t.into();
    let paths = cfg.paths();
    let signal_data: Vec<&[f64]> = preparation
        .signal_idx
        .iter()
        .map(|&idx| data[idx].as_slice())
        .collect();

    // Lock-in processing
    let lockin_output = li_process(
        cfg,
        t,
        &preparation.signal_ch,
        &signal_data,
        preparation.reference,
    )?;
    verify_lockin_geometry(preparation, &lockin_output)?;

    let t_stride = &preparation.t_stride;
    let sensor_rate_stride = &preparation.sensor_rate_stride;
    let sensor_integral_stride = &preparation.sensor_integral_stride;
    let signal_ch = &preparation.signal_ch;

    // Save lock-in results
    let headers = get_li_headers(cfg)?;
    let t0 = std::time::Instant::now();
    for (sig_ch, li_result) in signal_ch.iter().zip(lockin_output.result.iter()) {
        let li_result_path = paths.lockin_xy_csv(*sig_ch);
        write_li_results(
            &li_result_path,
            &headers,
            t_stride,
            sensor_rate_stride,
            sensor_integral_stride,
            li_result,
            cfg.lockin.save_npy,
        )?;
    }
    let elapsed_save = t0.elapsed();
    ui::saved(format!(
        "lock-in results for signals {:?} ({})",
        signal_ch,
        ui::fmt_duration(elapsed_save)
    ));

    // GLS diagnostics: per-window quality (always) plus the selected XY
    // covariance serialization (none omits the artifact entirely). The
    // manifest refresh downstream registers both kinds through the real
    // consumer path; boxcar executions publish neither.
    if let (Some(quality), Some(covariance), Some(snapshots), Some(bindings)) = (
        &lockin_output.quality,
        &lockin_output.covariance,
        &lockin_output.estimator_snapshots,
        &lockin_output.bindings,
    ) {
        use crate::lockin::estimator_snapshot::write_estimator_json;
        use crate::lockin::joint::{write_covariance_csv, write_covariance_npy, write_quality_csv};
        let mode = match &cfg.lockin.estimator {
            crate::config::LockinEstimator::JointHarmonicGls(gls) => gls.covariance_output,
            crate::config::LockinEstimator::BoxcarLegacy => {
                crate::config::GlsCovarianceOutput::None
            }
        };
        for ((((sig_ch, rows), covariances), snapshot), binding) in signal_ch
            .iter()
            .zip(quality.iter())
            .zip(covariance.iter())
            .zip(snapshots.iter())
            .zip(bindings.iter())
        {
            write_quality_csv(&paths.lockin_quality_csv(*sig_ch), rows)?;
            write_estimator_json(&paths.lockin_estimator_json(*sig_ch), snapshot)?;
            // v4 retention: persist the exact validated calibration bytes
            // the loader hashed, so staged reruns survive external model
            // removal (synthetic sources carry no bytes: nothing to retain).
            if let Some(bytes) = binding.bytes.as_ref() {
                let retained = paths.lockin_calibration_json(*sig_ch);
                if !retained.exists() {
                    if let Some(parent) = retained.parent() {
                        std::fs::create_dir_all(parent).with_context(|| {
                            format!(
                                "failed to create calibration retention dir: {}",
                                parent.display()
                            )
                        })?;
                    }
                    std::fs::write(&retained, bytes).with_context(|| {
                        format!("failed to retain calibration bytes: {}", retained.display())
                    })?;
                }
            }
            if !matches!(mode, crate::config::GlsCovarianceOutput::None) {
                let times: Vec<f64> = rows.iter().map(|row| row.time_s).collect();
                write_covariance_csv(
                    &paths.lockin_covariance_csv(*sig_ch),
                    mode,
                    &times,
                    covariances,
                )?;
                write_covariance_npy(
                    &paths.lockin_covariance_npy(*sig_ch),
                    mode,
                    &times,
                    covariances,
                )?;
            }
        }
        ui::saved(format!(
            "joint GLS diagnostics for signals {signal_ch:?} (quality always, covariance {mode:?})"
        ));
    }

    let headers = LI_HEADER;
    let labels: Vec<String> = headers
        .iter()
        .map(|s| s.trim().replace("(V)", ""))
        .collect();

    plot::run_plot(
        &cfg.plot,
        &cfg.paths().lockin_xy_combined_plot(),
        "plotting lock-in results",
        "lock-in plot completed",
        |output| {
            lockin_plot::LIPlotter {}
                .plot(
                    &cfg.plot,
                    output,
                    t_stride,
                    &lockin_output.result,
                    signal_ch,
                    &labels,
                )
                .context("failed to plot lock-in results")
        },
    )?;

    Ok(LockinStageOutput {
        result: lockin_output.result,
        provenance: lockin_output.provenance,
    })
}

/// Sensor -> reference preparation -> lock-in composition, unchanged in
/// shape and numerical behavior for the standalone `li`/`signal` commands.
pub fn run_li<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    data: &[Vec<f64>],
) -> Result<LockinRunOutput> {
    let t = t.into();
    let sensor = run_sensor_stage(cfg, t, data)?;
    let preparation = prepare_lockin_grid(cfg, t, data, &sensor)?;
    let lockin = execute_lockin(cfg, t, data, &preparation)?;
    Ok((
        preparation.t_stride,
        preparation.sensor_rate_stride,
        preparation.sensor_integral_stride,
        lockin.result,
        preparation.reference,
        lockin.provenance,
    ))
}

fn check_column_bounds(data: &[Vec<f64>], indices: impl IntoIterator<Item = usize>) -> Result<()> {
    for index in indices {
        if index >= data.len() {
            bail!(
                "Configuration error: required channel index {} is out of bounds. Fetched data only has {} channels.",
                index,
                data.len()
            );
        }
    }
    Ok(())
}

/// The executed lock-in geometry must be the frozen preparation: a mismatch
/// would silently desynchronize the signal grid, the saved lock-in results
/// and the downstream phase/MOKE grid.
fn verify_lockin_geometry(
    preparation: &LockinPreparation,
    output: &LockinProcessOutput,
) -> Result<()> {
    if output.base_index_range != preparation.index_range
        || output.output_index_range != preparation.index_range
    {
        bail!(
            "lock-in executed base/output index ranges {:?}/{:?} do not match the prepared grid {:?}",
            output.base_index_range,
            output.output_index_range,
            preparation.index_range
        );
    }
    let expected_len = preparation.t_stride.len();
    for (signal_index, signal) in output.result.iter().enumerate() {
        for (column_index, column) in signal.iter().enumerate() {
            if column.len() != expected_len {
                bail!(
                    "lock-in result length mismatch: signal {signal_index} column {column_index} has {}, expected {expected_len}",
                    column.len()
                );
            }
        }
    }
    Ok(())
}

pub fn li_process<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    signal_ch: &[u8],
    signal_data: &[&[f64]],
    ref_fit_params: RefFitParams,
) -> Result<LockinProcessOutput> {
    match &cfg.lockin.estimator {
        crate::config::LockinEstimator::BoxcarLegacy => {
            li_process_boxcar(cfg, t, signal_ch, signal_data, ref_fit_params)
        }
        crate::config::LockinEstimator::JointHarmonicGls(gls) => {
            li_process_joint(cfg, t, signal_ch, signal_data, ref_fit_params, gls)
        }
    }
}

/// Joint-harmonic GLS estimation over the shared legacy-trim grid.
/// Calibration-backed models only (FR-048): any loading failure aborts,
/// never falls back to boxcar_legacy.
fn li_process_joint<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    signal_ch: &[u8],
    signal_data: &[&[f64]],
    ref_fit_params: RefFitParams,
    gls: &crate::config::JointHarmonicGlsConfig,
) -> Result<LockinProcessOutput> {
    use crate::lockin::joint::{JointRunInputs, run_joint_li};
    use crate::lockin::model_loading::{FileNoiseModelSource, PIPELINE_VOLTAGE_UNIT};

    let t = t.into();
    let sample_interval_s = t
        .dt()
        .ok_or_else(|| anyhow::anyhow!("joint_harmonic_gls execution needs a uniform timebase"))?;
    // Calibration paths resolve against the declaring configuration file
    // directory, never an unspecified cwd (INTERFACES section 5).
    let base_dir = cfg
        .source_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let source = FileNoiseModelSource::new(
        base_dir,
        &gls.calibrations,
        PIPELINE_VOLTAGE_UNIT.to_string(),
        sample_interval_s,
        ref_fit_params.f_ref,
    )
    .context("joint_harmonic_gls model source is not usable")?;
    let inputs = JointRunInputs {
        lockin: &cfg.lockin,
        gls,
        t,
        f_ref: ref_fit_params.f_ref,
        omega_tref: ref_fit_params.omega_tref,
        sample_rate: sample_interval_s.recip(),
        tolerances: pmoke_analysis_core::joint::JointSolverTolerances::default(),
    };
    let output = run_joint_li(&inputs, signal_ch, signal_data, &source)?;
    Ok(LockinProcessOutput {
        result: output.result,
        quality: Some(output.quality),
        covariance: Some(output.covariance),
        estimator_snapshots: Some(output.snapshots),
        bindings: Some(output.bindings),
        base_index_range: output.base_index_range,
        output_index_range: output.output_index_range,
        provenance: output.provenance,
    })
}

pub fn li_process_boxcar<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    signal_ch: &[u8],
    signal_data: &[&[f64]],
    ref_fit_params: RefFitParams,
) -> Result<LockinProcessOutput> {
    let t = t.into();
    if signal_ch.len() != signal_data.len() {
        bail!(
            "signal channel count ({}) and signal data column count ({}) differ",
            signal_ch.len(),
            signal_data.len()
        );
    }
    if signal_data.is_empty() {
        bail!("no signal channels were available for lock-in processing");
    }
    let f_ref: f64 = ref_fit_params.f_ref;
    let omega_tref: f64 = ref_fit_params.omega_tref;
    let workers: usize = cfg.lockin.workers;

    let harmonics = HARMONICS;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .context("Failed to build rayon thread pool")?;

    let pb = ui::progress(
        format!("lock-in processing with {workers} workers"),
        (signal_data.len() * harmonics.len()) as u64,
    );
    let t0 = std::time::Instant::now();

    let mut all_signals_results: Vec<Vec<Vec<f64>>> = Vec::with_capacity(signal_data.len());
    let mut printed_lockin_summary = false;
    let mut base_index_range = None;
    let mut output_index_range = None;
    let mut provenance = None;
    let debug_time = cfg.lockin.lpf_debug_output.then(|| t.values(0..t.len()));

    for (&sig_ch, signal) in signal_ch.iter().zip(signal_data.iter()) {
        pb.set_message(format!("lock-in ch{sig_ch}"));
        let li_processor =
            lockin_core::LockinProcessor::new(t, signal, f_ref, omega_tref, &cfg.lockin)?;
        let processor_base_range = li_processor.base_index_range();
        let processor_output_range = li_processor.output_index_range();
        if provenance.is_none() {
            provenance = Some(LockinProvenance::from_processor(&li_processor));
        }
        if let Some(expected) = base_index_range {
            if processor_base_range != expected {
                bail!(
                    "lock-in base index range mismatch for signal ch{sig_ch}: {:?}, expected {:?}",
                    processor_base_range,
                    expected
                );
            }
        } else {
            base_index_range = Some(processor_base_range);
        }
        if let Some(expected) = output_index_range {
            if processor_output_range != expected {
                bail!(
                    "lock-in output index range mismatch for signal ch{sig_ch}: {:?}, expected {:?}",
                    processor_output_range,
                    expected
                );
            }
        } else {
            output_index_range = Some(processor_output_range);
        }
        if !printed_lockin_summary {
            ui::suspend_progress(&pb, || {
                ui::settings_table(
                    "Lock-in settings",
                    li_processor
                        .summary_lines()
                        .into_iter()
                        .map(|line| {
                            let (setting, value) = line.split_once('=').unwrap_or((&line, ""));
                            (setting.trim().to_string(), value.trim().to_string())
                        })
                        .collect(),
                );
            });
            printed_lockin_summary = true;
        }
        let include_debug = cfg.lockin.lpf_debug_output;

        let harmonic_results: Vec<lockin_core::HarmonicLockinResult> = if include_debug {
            let t_output = li_processor.output_times();
            let params = li_processor.params();
            let mut results = Vec::with_capacity(harmonics.len());
            for &harmonic in &harmonics {
                pb.set_message(format!("lock-in ch{sig_ch} h{harmonic}"));
                let result = li_processor.compute_harmonic_detailed(harmonic, include_debug);
                if include_debug {
                    debug::write_harmonic_debug(
                        cfg,
                        sig_ch,
                        harmonic,
                        params,
                        debug_time
                            .as_deref()
                            .expect("debug time is available when debug output is enabled"),
                        &t_output,
                        &result,
                    )
                    .with_context(|| {
                        format!("failed to write lock-in debug output for ch{sig_ch} h{harmonic}")
                    })?;
                    results.push(result.without_debug_data());
                } else {
                    results.push(result);
                }
                pb.inc(1);
            }
            results
        } else {
            let progress = pb.clone();
            pool.install(|| {
                harmonics
                    .par_iter()
                    .map(|&harmonic| {
                        progress.set_message(format!("lock-in ch{sig_ch} h{harmonic}"));
                        let result = li_processor.compute_harmonic_detailed(harmonic, false);
                        progress.inc(1);
                        result
                    })
                    .collect()
            })
        };

        let mut results_list = Vec::with_capacity(harmonic_results.len() * 2);
        for result in harmonic_results {
            results_list.push(result.li_x);
            results_list.push(result.li_y);
        }

        all_signals_results.push(results_list);
    }

    let elapsed_li = t0.elapsed();
    ui::finish_success(
        pb,
        format!(
            "lock-in processing completed ({})",
            ui::fmt_duration(elapsed_li)
        ),
    );

    Ok(LockinProcessOutput {
        result: all_signals_results,
        quality: None,
        covariance: None,
        estimator_snapshots: None,
        bindings: None,
        base_index_range: base_index_range.unwrap_or((0, 0)),
        output_index_range: output_index_range.unwrap_or((0, 0)),
        provenance: provenance
            .context("no signal channels were available for lock-in processing")?,
    })
}
