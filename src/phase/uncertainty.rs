//! Conditional rotated-XY covariance reconstruction (R2c).
//!
//! The phase stage rotates XY quadratures by fitted per-harmonic deltas.
//! When the joint GLS covariance artifact is present **and** the config
//! selects `joint_harmonic_gls`, this module reconstructs the rotated XY
//! covariance per output window via the shared-core dense rotation, so MOKE
//! conditional uncertainty can be derived downstream without the external
//! model files. Boxcar runs, missing artifacts, and
//! `covariance_output=none` all yield `None` (no silent zeros); malformed
//! artifacts fail closed.

use crate::config::ArtifactPaths;
use crate::utils::csv::read_csv;
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::joint::rotate_xy_covariance_dense;
use pmoke_analysis_core::moke_uncertainty::moke_standard_angle_variance;

pub const PHIM_DEFAULT: f64 = 0.92;

/// Reconstructs rotated XY covariances for one channel as row-major 12x12
/// matrices, aligned with the rotated XY rows. Returns `None` when
/// reconstruction does not apply (boxcar estimator, missing artifact).
pub fn reconstruct_rotated_covariances(
    paths: &ArtifactPaths,
    channel: u8,
    deltas_rad: &[f64; 6],
    is_gls: bool,
) -> Result<Option<Vec<Vec<f64>>>> {
    if !is_gls {
        return Ok(None);
    }
    let covariance_csv = paths.lockin_covariance_csv(channel);
    if !covariance_csv.is_file() {
        return Ok(None);
    }
    let columns = read_csv(&covariance_csv).with_context(|| {
        format!(
            "failed to read covariance artifact: {}",
            covariance_csv.display()
        )
    })?;
    // Two serializations exist (FR-041): 13 columns = time_s + 12 diagonal,
    // 79 columns = time_s + 78 upper-triangle entries in lexicographic
    // (row, column) order. Anything else is malformed and fails closed —
    // never a partial-zero reconstruction.
    let full = match columns.len() {
        13 => false,
        79 => true,
        other => bail!(
            "covariance artifact has {other} columns, need time_s plus 12 diagonal or 78 upper-triangle entries"
        ),
    };
    let rows = columns[0].len();
    if columns.iter().any(|column| column.len() != rows) {
        bail!("covariance artifact columns have unequal lengths");
    }
    let mut rotated = Vec::with_capacity(rows);
    for (row, time) in columns[0].iter().enumerate() {
        if !time.is_finite() {
            bail!("covariance artifact row {row} has non-finite time_s");
        }
        let entries = if full {
            let mut packed_index = 0;
            let mut entries = vec![0.0; 144];
            for upper_row in 0..12 {
                for upper_column in upper_row..12 {
                    let value = columns[1 + packed_index][row];
                    if !value.is_finite() {
                        bail!("covariance artifact row {row} has non-finite entries");
                    }
                    entries[upper_row * 12 + upper_column] = value;
                    entries[upper_column * 12 + upper_row] = value;
                    packed_index += 1;
                }
            }
            entries
        } else {
            let mut entries = vec![0.0; 144];
            for index in 0..12 {
                let value = columns[1 + index][row];
                if !value.is_finite() {
                    bail!("covariance artifact row {row} has non-finite entries");
                }
                entries[index * 12 + index] = value;
            }
            entries
        };
        let rotated_row = rotate_xy_covariance_dense(&entries, deltas_rad)
            .map_err(|error| anyhow::anyhow!("covariance rotation failed at row {row}: {error}"))?;
        rotated.push(rotated_row);
    }
    Ok(Some(rotated))
}

/// Delta-method conditional variance of the standard MOKE angle at one
/// rotated `(x1, x2)` point with its marginal `(v11, v12, v22)` covariance
/// entries. Fail-closed: degenerate or non-finite inputs error, never zero.
pub fn conditional_moke_variance(x1: f64, x2: f64, v11: f64, v12: f64, v22: f64) -> Result<f64> {
    moke_standard_angle_variance(x1, x2, PHIM_DEFAULT, v11, v12, v22)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

#[cfg(test)]
mod tests {
    use super::reconstruct_rotated_covariances;
    use crate::config::ArtifactPaths;

    fn staging_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pmoke_phase_uncertainty_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("analysis/lockin")).unwrap();
        dir
    }

    fn write_diagonal_covariance(paths: &ArtifactPaths, channel: u8, rows: usize) {
        let mut text = String::from("time_s");
        for name in [
            "x1", "y1", "x2", "y2", "x3", "y3", "x4", "y4", "x5", "y5", "x6", "y6",
        ] {
            text.push_str(&format!(",cov_{name}_{name}_v2"));
        }
        text.push('\n');
        for row in 0..rows {
            text.push_str(&format!("{}", row as f64 * 0.001));
            for index in 0..12 {
                text.push_str(&format!(",{}", 0.001 * (index as f64 + 1.0)));
            }
            text.push('\n');
        }
        std::fs::write(paths.lockin_covariance_csv(channel), text).unwrap();
    }

    #[test]
    fn boxcar_returns_none_without_reading() {
        // Boxcar estimators never reconstruct: None without touching disk.
        let dir = staging_dir("boxcar");
        let paths = ArtifactPaths::new(&dir);
        let deltas = [0.0; 6];
        let result = reconstruct_rotated_covariances(&paths, 3, &deltas, false).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_artifact_returns_none_for_gls() {
        // covariance_output=none (or a pruned artifact) is explicit
        // unavailability, not zero covariance.
        let dir = staging_dir("missing");
        let paths = ArtifactPaths::new(&dir);
        let deltas = [0.0; 6];
        let result = reconstruct_rotated_covariances(&paths, 3, &deltas, true).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn diagonal_covariance_rotates_x1_x2_block() {
        // Identity deltas: rotation is the identity, so the (x1, x2)
        // marginal comes back as the written diagonal entries.
        let dir = staging_dir("diagonal");
        let paths = ArtifactPaths::new(&dir);
        write_diagonal_covariance(&paths, 3, 2);
        let deltas = [0.0; 6];
        let result = reconstruct_rotated_covariances(&paths, 3, &deltas, true)
            .unwrap()
            .expect("diagonal artifact must reconstruct");
        assert_eq!(result.len(), 2);
        for matrix in &result {
            assert_eq!(matrix.len(), 144);
            // (x1, x1) = 0.001, (x2, x2) = 0.003, off-diagonal ~ 0.
            assert!((matrix[0] - 0.001).abs() < 1e-12);
            assert!((matrix[2 * 12 + 2] - 0.003).abs() < 1e-12);
            assert!(matrix[2].abs() < 1e-12);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_artifact_fails_closed() {
        // A truncated covariance file (missing diagonal columns) must
        // error, never reconstruct partial zeros.
        let dir = staging_dir("malformed");
        let paths = ArtifactPaths::new(&dir);
        std::fs::write(
            paths.lockin_covariance_csv(3),
            "time_s,cov_x1_x1_v2\n0.0,0.001\n",
        )
        .unwrap();
        let deltas = [0.0; 6];
        assert!(reconstruct_rotated_covariances(&paths, 3, &deltas, true).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn conditional_variance_agrees_with_core() {
        // The pipeline wrapper is a thin phim=0.92 binding over the core
        // delta-method variance (validated against the numerical oracle in
        // the core test suite).
        let variance = super::conditional_moke_variance(1.7, -0.9, 0.004, 0.0007, 0.009).unwrap();
        let core = pmoke_analysis_core::moke_uncertainty::moke_standard_angle_variance(
            1.7, -0.9, 0.92, 0.004, 0.0007, 0.009,
        )
        .unwrap();
        assert!((variance - core).abs() < 1e-18);
        assert!(variance > 0.0);
    }
}

/// Writes the rotated covariance artifact (`ch{N}_rotated_covariance.csv`):
/// `time_s` plus 144 row-major rotated entries per output window. Never
/// overwrites; non-finite entries abort instead of publishing unknown rows.
pub fn write_rotated_covariance_csv(
    path: &std::path::Path,
    times: &[f64],
    rotated: &[Vec<f64>],
) -> anyhow::Result<()> {
    use anyhow::Context;
    if path.exists() {
        anyhow::bail!(
            "rotated covariance output already exists: {}",
            path.display()
        );
    }
    if times.len() != rotated.len() {
        anyhow::bail!(
            "rotated covariance times ({}) and matrices ({}) differ",
            times.len(),
            rotated.len()
        );
    }
    if times.iter().any(|time| !time.is_finite()) {
        anyhow::bail!("rotated covariance has non-finite time_s");
    }
    if rotated.iter().any(|row| row.len() != 144) {
        anyhow::bail!("rotated covariance rows must each carry 144 entries");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create rotated covariance dir: {}",
                parent.display()
            )
        })?;
    }
    if rotated.iter().flatten().any(|value| !value.is_finite()) {
        anyhow::bail!("rotated covariance has non-finite entries");
    }
    let file = std::fs::File::create(path).with_context(|| {
        format!(
            "failed to create rotated covariance output: {}",
            path.display()
        )
    })?;
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_writer(file);
    let mut header = vec!["time_s".to_string()];
    for row in 0..12 {
        for column in 0..12 {
            header.push(format!("rot_cov_r{row}_c{column}_v2"));
        }
    }
    writer
        .write_record(&header)
        .context("failed to write rotated covariance header")?;
    for (time, row) in times.iter().zip(rotated.iter()) {
        let mut record = Vec::with_capacity(145);
        record.push(time.to_string());
        record.extend(row.iter().map(ToString::to_string));
        writer
            .write_record(&record)
            .with_context(|| format!("failed to write rotated covariance row at t={time}"))?;
    }
    writer
        .flush()
        .context("failed to flush rotated covariance output")?;
    Ok(())
}

/// Reads one rotated covariance artifact into `(times, row-major matrices)`.
/// Missing file yields `None` (explicit unavailability); present-but-broken
/// files fail closed.
pub type RotatedCovarianceGrid = (Vec<f64>, Vec<Vec<f64>>);

pub fn read_rotated_covariance_csv(
    path: &std::path::Path,
) -> anyhow::Result<Option<RotatedCovarianceGrid>> {
    use anyhow::Context;
    if !path.is_file() {
        return Ok(None);
    }
    let columns = read_csv(path)
        .with_context(|| format!("failed to read rotated covariance: {}", path.display()))?;
    if columns.len() != 145 {
        anyhow::bail!(
            "rotated covariance has {} columns, need time_s plus 144 entries",
            columns.len()
        );
    }
    let rows = columns[0].len();
    if columns.iter().any(|column| column.len() != rows) {
        anyhow::bail!("rotated covariance columns have unequal lengths");
    }
    if columns.iter().flatten().any(|value| !value.is_finite()) {
        anyhow::bail!("rotated covariance has non-finite entries");
    }
    let times = columns[0].clone();
    let matrices = (0..rows)
        .map(|row| (1..145).map(|column| columns[column][row]).collect())
        .collect();
    Ok(Some((times, matrices)))
}

#[cfg(test)]
mod roundtrip_tests {
    use super::{read_rotated_covariance_csv, write_rotated_covariance_csv};

    #[test]
    fn rotated_covariance_roundtrips_times_and_entries() {
        let dir = std::env::temp_dir().join(format!(
            "pmoke_phase_uncertainty_roundtrip_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rot.csv");
        let times = vec![0.001, 0.002];
        let mut first = vec![0.0; 144];
        first[0] = 0.004;
        first[2] = 0.0007;
        first[2 * 12 + 2] = 0.009;
        let mut second = vec![0.0; 144];
        second[0] = 0.01;
        second[2 * 12 + 2] = 0.02;
        let rotated = vec![first.clone(), second.clone()];
        write_rotated_covariance_csv(&path, &times, &rotated).unwrap();
        let (read_times, read_matrices) = read_rotated_covariance_csv(&path)
            .unwrap()
            .expect("written artifact must read back");
        assert_eq!(read_times, times);
        assert_eq!(read_matrices.len(), 2);
        assert!((read_matrices[0][0] - 0.004).abs() < 1e-15);
        assert!((read_matrices[0][2] - 0.0007).abs() < 1e-15);
        assert!((read_matrices[0][2 * 12 + 2] - 0.009).abs() < 1e-15);
        assert!((read_matrices[1][2 * 12 + 2] - 0.02).abs() < 1e-15);
        // Conditional variance off the round-tripped marginal agrees with
        // the core delta-method value.
        let variance = super::conditional_moke_variance(
            1.7,
            -0.9,
            read_matrices[0][0],
            read_matrices[0][2],
            read_matrices[0][2 * 12 + 2],
        )
        .unwrap();
        let core = pmoke_analysis_core::moke_uncertainty::moke_standard_angle_variance(
            1.7, -0.9, 0.92, 0.004, 0.0007, 0.009,
        )
        .unwrap();
        assert!((variance - core).abs() < 1e-18);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotated_covariance_writer_rejects_non_finite() {
        // Fail-closed: a NaN entry aborts before creating the file, never
        // a partial artifact.
        let dir = std::env::temp_dir().join(format!(
            "pmoke_phase_uncertainty_reject_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rot.csv");
        let mut bad = vec![0.0; 144];
        bad[7] = f64::NAN;
        assert!(write_rotated_covariance_csv(&path, &[0.001], &[bad]).is_err());
        assert!(!path.is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod export_policy_tests {
    use super::reconstruct_rotated_covariances;
    use crate::config::ArtifactPaths;
    use crate::config::GlsCovarianceOutput;
    use crate::lockin::joint::{pack_covariance_row, write_covariance_csv};

    fn staging_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pmoke_phase_uncertainty_policy_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("analysis/lockin")).unwrap();
        dir
    }

    fn identity_12() -> Vec<Vec<f64>> {
        (0..12)
            .map(|row| (0..12).map(|column| f64::from(row == column)).collect())
            .collect()
    }

    #[test]
    fn uncertainty_agrees_across_diagonal_and_full_policies() {
        // ROADMAP R2 exit clause: uncertainty must agree across XY export
        // policies. Diagonal and full serialize the same design-model
        // covariance here (identity marginal), so the reconstructed
        // conditional variance agrees exactly; none-policy is handled by
        // the missing-artifact test (explicit None, never zeros).
        for mode in [GlsCovarianceOutput::Diagonal, GlsCovarianceOutput::Full] {
            let dir = staging_dir(if matches!(mode, GlsCovarianceOutput::Diagonal) {
                "diagonal"
            } else {
                "full"
            });
            let paths = ArtifactPaths::new(&dir);
            let matrix = identity_12();
            write_covariance_csv(
                &paths.lockin_covariance_csv(3),
                mode,
                &[0.001],
                std::slice::from_ref(&matrix),
            )
            .unwrap();
            let deltas = [0.0; 6];
            let rotated = reconstruct_rotated_covariances(&paths, 3, &deltas, true)
                .unwrap()
                .expect("artifact must reconstruct");
            assert_eq!(rotated.len(), 1);
            let packed = pack_covariance_row(
                &rotated[0]
                    .chunks(12)
                    .map(<[f64]>::to_vec)
                    .collect::<Vec<_>>(),
                GlsCovarianceOutput::Diagonal,
            )
            .unwrap();
            // Identity deltas preserve the identity marginal.
            for (index, value) in packed.iter().enumerate() {
                assert!((value - 1.0).abs() < 1e-12, "entry {index}: {value}");
            }
            let variance =
                super::conditional_moke_variance(1.0, 1.0, packed[0], 0.0, packed[2]).unwrap();
            let expected = pmoke_analysis_core::moke_uncertainty::moke_standard_angle_variance(
                1.0, 1.0, 0.92, 1.0, 0.0, 1.0,
            )
            .unwrap();
            assert!((variance - expected).abs() < 1e-18);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
