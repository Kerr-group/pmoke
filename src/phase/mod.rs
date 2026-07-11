pub mod omega_t0_analysis;
pub mod phase_rotation_plot;
pub mod rotator;
pub mod save;

use crate::analysis_results::parse_analysis_result_files;
use crate::constants::{LI_HEADER, LI_ROTATED_HEADER};
use crate::phase::omega_t0_analysis::OT0Analyser;
use crate::phase::phase_rotation_plot::PhaseRotationPlotter;
use crate::phase::rotator::rotate_phase;
use crate::phase::save::{get_li_rotated_headers, write_li_rotated_results};
use crate::{config::Config, utils::csv::read_csv};
use crate::{plot, ui};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::f64::consts::PI;
use std::time::Instant;

#[derive(Debug)]
pub struct PhaseAnalysisOutput {
    pub rotated_result: Vec<Vec<f64>>,
    pub omega_t0: f64,
    pub deltas: [f64; 6],
}

pub fn run(cfg: &Config) -> Result<()> {
    let ch = cfg.phase_signal_ch();

    if ch.is_empty() {
        ui::skipped("phase analysis: no channels specified");
        return Ok(());
    }

    let t0 = Instant::now();
    let pb = ui::spinner(format!("reading lock-in results for channels {:?}", ch));

    let paths = cfg.paths();
    let all_data: Vec<Vec<Vec<f64>>> = ch
        .par_iter()
        .map(|channel| read_csv(paths.lockin_xy_csv(*channel)))
        .collect::<Result<Vec<_>, _>>()?;

    let elapsed_read = t0.elapsed();
    ui::finish_read(
        pb,
        format!(
            "lock-in results for channels {:?} ({})",
            ch,
            ui::fmt_duration(elapsed_read)
        ),
    );

    let data = parse_analysis_result_files(
        &all_data,
        cfg.roles.sensor_ch.len(),
        LI_HEADER.len(),
        "lock-in results",
    )?;

    let _ = run_phase_analysis(
        cfg,
        &data.time,
        &data.sensor_rate,
        &data.sensor_integral,
        &data.results,
    )?;
    Ok(())
}

pub fn run_phase_analysis(
    cfg: &Config,
    t: &[f64],
    sensor_rate_ch: &[Vec<f64>],
    sensor_integral_ch: &[Vec<f64>],
    li_results: &[Vec<Vec<f64>>],
) -> Result<Vec<Vec<Vec<f64>>>> {
    let headers = LI_ROTATED_HEADER;
    let labels: Vec<String> = headers
        .iter()
        .map(|s| s.trim().replace("(V)", ""))
        .collect();
    let paths = cfg.paths();
    let ch = cfg.phase_signal_ch();

    let pb = ui::progress(
        format!("phase analysis for channels {:?}", ch),
        ch.len() as u64,
    );
    let mut rotated_results: Vec<Vec<Vec<f64>>> = Vec::new();
    for (ch_i, li_result) in ch.iter().zip(li_results.iter()) {
        pb.set_message(format!("phase analysis ch{ch_i}"));
        let phase_output = phase_analysis(cfg, li_result)?;
        ui::suspend_progress(&pb, || {
            ui::summary_table(
                format!("Phase rotation ch{ch_i}"),
                &["Metric", "Value"],
                vec![
                    vec![
                        "omega_t0".to_string(),
                        format!("{:.8} rad", phase_output.omega_t0),
                    ],
                    vec![
                        "delta[1..6]".to_string(),
                        phase_output
                            .deltas
                            .iter()
                            .map(|delta| format!("{delta:.4}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ],
                ],
            );
        });
        let path = paths.lockin_rotated_csv(*ch_i);
        let headers = get_li_rotated_headers(cfg)?;
        write_li_rotated_results(
            &path,
            &headers,
            t,
            sensor_rate_ch,
            sensor_integral_ch,
            &phase_output.rotated_result,
        )?;
        rotated_results.push(phase_output.rotated_result);
        pb.inc(1);
    }
    ui::finish_saved(pb, format!("phase-rotated results for channels {:?}", ch));
    ui::success("phase analysis completed");

    plot::run_plot(
        &cfg.plot,
        "plotting phase-rotated results",
        "phase plot completed",
        || {
            PhaseRotationPlotter {}
                .plot(&cfg.plot, t, &rotated_results, ch, &labels)
                .context("failed to plot phase-rotated results")
        },
    )?;
    Ok(rotated_results)
}

pub fn phase_analysis(cfg: &Config, li_result: &[Vec<f64>]) -> Result<PhaseAnalysisOutput> {
    if li_result.len() < 12 {
        bail!(
            "phase analysis requires 12 lock-in columns (six x/y pairs), got {}",
            li_result.len()
        );
    }
    let pairs: Vec<_> = li_result.chunks_exact(2).collect();

    let [
        [li1x, li1y],
        [li2x, li2y],
        [li3x, li3y],
        [li4x, li4y],
        [li5x, li5y],
        [li6x, li6y],
        ..,
    ] = pairs.as_slice()
    else {
        unreachable!("lock-in column count was validated above");
    };
    let theta_1: Vec<f64> = li1y
        .iter()
        .zip(li1x.iter())
        .map(|(y, x)| y.atan2(*x))
        .collect();
    let theta_2: Vec<f64> = li2y
        .iter()
        .zip(li2x.iter())
        .map(|(y, x)| y.atan2(*x))
        .collect();
    let theta_3: Vec<f64> = li3y
        .iter()
        .zip(li3x.iter())
        .map(|(y, x)| y.atan2(*x))
        .collect();
    let theta_4: Vec<f64> = li4y
        .iter()
        .zip(li4x.iter())
        .map(|(y, x)| y.atan2(*x))
        .collect();
    let theta_5: Vec<f64> = li5y
        .iter()
        .zip(li5x.iter())
        .map(|(y, x)| y.atan2(*x))
        .collect();
    let theta_6: Vec<f64> = li6y
        .iter()
        .zip(li6x.iter())
        .map(|(y, x)| y.atan2(*x))
        .collect();

    let offset_phases = &cfg.phase.m_omega_t0_offset;

    let m_omega_t0_1: Vec<f64> = theta_1
        .iter()
        .map(|&theta| theta - PI + offset_phases[0])
        .collect();
    let m_omega_t0_2: Vec<f64> = theta_2
        .iter()
        .map(|&theta| theta - PI / 2.0 + offset_phases[1])
        .collect();
    let m_omega_t0_3: Vec<f64> = theta_3
        .iter()
        .map(|&theta| theta - PI + offset_phases[2])
        .collect();
    let m_omega_t0_4: Vec<f64> = theta_4
        .iter()
        .map(|&theta| theta - PI / 2.0 + offset_phases[3])
        .collect();
    let m_omega_t0_5: Vec<f64> = theta_5
        .iter()
        .map(|&theta| theta - PI + offset_phases[4])
        .collect();
    let m_omega_t0_6: Vec<f64> = theta_6
        .iter()
        .map(|&theta| theta - PI / 2.0 + offset_phases[5])
        .collect();

    let omega_t0: f64 = OT0Analyser {}
        .analyse(
            &cfg.plot,
            [
                &m_omega_t0_1,
                &m_omega_t0_2,
                &m_omega_t0_3,
                &m_omega_t0_4,
                &m_omega_t0_5,
                &m_omega_t0_6,
            ],
        )
        .context("failed to analyse omega_t0")?;

    let delta_1 = PI - 1.0 * omega_t0;
    let delta_2 = PI / 2.0 - 2.0 * omega_t0;
    let delta_3 = PI - 3.0 * omega_t0;
    let delta_4 = PI / 2.0 - 4.0 * omega_t0;
    let delta_5 = PI - 5.0 * omega_t0;
    let delta_6 = PI / 2.0 - 6.0 * omega_t0;
    let deltas = [delta_1, delta_2, delta_3, delta_4, delta_5, delta_6];

    let (li1_in, li1_out) = rotate_phase(li1x, li1y, delta_1);
    let (li2_in, li2_out) = rotate_phase(li2x, li2y, delta_2);
    let (li3_in, li3_out) = rotate_phase(li3x, li3y, delta_3);
    let (li4_in, li4_out) = rotate_phase(li4x, li4y, delta_4);
    let (li5_in, li5_out) = rotate_phase(li5x, li5y, delta_5);
    let (li6_in, li6_out) = rotate_phase(li6x, li6y, delta_6);

    let rotated_result: Vec<Vec<f64>> = vec![
        li1_in, li1_out, li2_in, li2_out, li3_in, li3_out, li4_in, li4_out, li5_in, li5_out,
        li6_in, li6_out,
    ];

    Ok(PhaseAnalysisOutput {
        rotated_result,
        omega_t0,
        deltas,
    })
}

#[cfg(test)]
mod tests {
    use super::phase_analysis;

    #[test]
    fn phase_analysis_rejects_incomplete_harmonic_columns() {
        let cfg = crate::test_support::test_config(vec![1], vec![2]);
        let error = phase_analysis(&cfg, &vec![vec![0.0]; 10]).unwrap_err();
        assert!(error.to_string().contains("requires 12 lock-in columns"));
    }
}
