pub mod omega_t0_analysis;
pub mod phase_rotation_plot;
pub mod rotator;
pub mod save;

use crate::constants::{LI_ROTATED_HEADER, LI_ROTATED_NAME};
use crate::phase::omega_t0_analysis::OT0Analyser;
use crate::phase::phase_rotation_plot::PhaseRotationPlotter;
use crate::phase::rotator::rotate_phase;
use crate::phase::save::{get_li_rotated_headers, write_li_rotated_results};
use crate::{config::Config, constants::LI_RESULTS_NAME, utils::csv::read_csv};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::f64::consts::PI;
use std::time::Instant;

pub fn run(cfg: &Config) -> Result<()> {
    let ch = &cfg.phase.use_signal_ch;

    if ch.is_empty() {
        println!("⚠️ No channels specified for phase analysis. Skipping phase analysis.");
        return Ok(());
    }

    let num_sensor_ch = cfg.roles.sensor_ch.len();

    let t0 = Instant::now();

    let all_data: Vec<Vec<Vec<f64>>> = ch
        .par_iter()
        .map(|channel| {
            let fname = format!("{}_ch{}.csv", LI_RESULTS_NAME, channel);
            read_csv(fname)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let elapsed_read = t0.elapsed();
    println!(
        "📥 Read lock-in rotated results for ch {:?} in {:.2?}",
        ch, elapsed_read
    );

    let t = all_data[0][0].clone(); // time column
    let sensor_integral_ch = all_data[0][1..(1 + num_sensor_ch)].to_vec();

    let li_results: Vec<Vec<Vec<f64>>> = all_data
        .iter()
        .map(|read_data| read_data[1 + num_sensor_ch..].to_vec())
        .collect();

    let _ = run_phase_analysis(cfg, &t, &sensor_integral_ch, &li_results)?;
    Ok(())
}

pub fn run_phase_analysis(
    cfg: &Config,
    t: &[f64],
    sensor_integral_ch: &[Vec<f64>],
    li_results: &[Vec<Vec<f64>>],
) -> Result<Vec<Vec<Vec<f64>>>> {
    let headers = LI_ROTATED_HEADER;
    let labels: Vec<String> = headers
        .iter()
        .map(|s| s.trim().replace("(V)", ""))
        .collect();
    let ch = &cfg.phase.use_signal_ch;

    println!("🔄 Running phase analysis for channels {:?}...", ch);
    let mut rotated_results: Vec<Vec<Vec<f64>>> = Vec::new();
    for (ch_i, li_result) in ch.iter().zip(li_results.iter()) {
        let rotated_result = phase_analysis(li_result)?;
        let li_rotated_name = LI_ROTATED_NAME;
        let fname = format!("{}_ch{}.csv", li_rotated_name, ch_i);
        let headers = get_li_rotated_headers(cfg)?;
        write_li_rotated_results(&fname, &headers, t, sensor_integral_ch, &rotated_result)?;
        rotated_results.push(rotated_result);
    }
    println!("✅ Phase analysis completed.");

    PhaseRotationPlotter {}
        .plot(t, &rotated_results, ch, &labels)
        .context("failed to plot phase-rotated results")?;
    Ok(rotated_results)
}

pub fn phase_analysis(li_result: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
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
        panic!(
            "Expected at least 6 pairs (12 elements), but got {}",
            li_result.len()
        );
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

    let m_omega_t0_1: Vec<f64> = theta_1.iter().map(|&theta| theta - PI).collect();
    let m_omega_t0_2: Vec<f64> = theta_2.iter().map(|&theta| theta - PI / 2.0).collect();
    let m_omega_t0_3: Vec<f64> = theta_3.iter().map(|&theta| theta - PI).collect();
    let m_omega_t0_4: Vec<f64> = theta_4.iter().map(|&theta| theta - PI / 2.0).collect();
    let m_omega_t0_5: Vec<f64> = theta_5.iter().map(|&theta| theta - PI).collect();
    let m_omega_t0_6: Vec<f64> = theta_6.iter().map(|&theta| theta - PI / 2.0).collect();

    let omega_t0: f64 = OT0Analyser {}
        .analyse(
            &m_omega_t0_1,
            &m_omega_t0_2,
            &m_omega_t0_3,
            &m_omega_t0_4,
            &m_omega_t0_5,
            &m_omega_t0_6,
        )
        .context("failed to analyse omega_t0")?;

    let delta_1 = PI - 1.0 * omega_t0;
    let delta_2 = PI / 2.0 - 2.0 * omega_t0;
    let delta_3 = PI - 3.0 * omega_t0;
    let delta_4 = PI / 2.0 - 4.0 * omega_t0;
    let delta_5 = PI - 5.0 * omega_t0;
    let delta_6 = PI / 2.0 - 6.0 * omega_t0;

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

    Ok(rotated_result)
}
