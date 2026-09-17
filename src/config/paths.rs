use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactPaths {
    pub run_dir: PathBuf,
    pub is_staging: bool,
}

impl ArtifactPaths {
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            run_dir: run_dir.into(),
            is_staging: false,
        }
    }

    pub fn run_manifest(&self) -> PathBuf {
        self.run_dir.join("run.toml")
    }

    pub fn source_config(&self) -> PathBuf {
        self.run_dir.join("config.source.toml")
    }

    pub fn resolved_config(&self) -> PathBuf {
        self.run_dir.join("config.resolved.toml")
    }

    pub fn to_staging(&self) -> Self {
        Self {
            run_dir: self.run_dir.clone(),
            is_staging: true,
        }
    }

    pub fn acquisition_dir(&self) -> PathBuf {
        if self.is_staging {
            self.run_dir.join("acquisition.incomplete")
        } else {
            self.run_dir.join("acquisition")
        }
    }

    pub fn acquisition_manifest(&self) -> PathBuf {
        self.acquisition_dir().join("manifest.toml")
    }

    pub fn waveform_dir(&self) -> PathBuf {
        self.acquisition_dir().join("waveforms")
    }

    pub fn waveform_binary(&self, channel: u8) -> PathBuf {
        self.waveform_dir().join(format!("ch{channel}.u16le"))
    }

    pub fn waveform_csv(&self) -> PathBuf {
        self.waveform_dir().join("waveform.csv")
    }

    pub fn oscilloscope_screenshot(&self) -> PathBuf {
        self.acquisition_dir()
            .join("screenshots")
            .join("oscilloscope.png")
    }

    pub fn analysis_dir(&self) -> PathBuf {
        if self.is_staging {
            self.run_dir.join("analysis.incomplete")
        } else {
            self.run_dir.join("analysis")
        }
    }

    pub fn analysis_manifest(&self) -> PathBuf {
        self.analysis_dir().join("manifest.toml")
    }

    pub fn analysis_source_config(&self) -> PathBuf {
        self.analysis_dir().join("config.source.toml")
    }

    pub fn analysis_resolved_config(&self) -> PathBuf {
        self.analysis_dir().join("config.resolved.toml")
    }

    pub fn diagnostic_config_dir(&self, stage: &str) -> PathBuf {
        self.diagnostics_dir().join(stage)
    }

    pub fn diagnostics_dir(&self) -> PathBuf {
        self.analysis_dir().join("diagnostics")
    }

    pub fn diagnostic_source_config(&self, stage: &str) -> PathBuf {
        self.diagnostic_config_dir(stage).join("config.source.toml")
    }

    pub fn diagnostic_resolved_config(&self, stage: &str) -> PathBuf {
        self.diagnostic_config_dir(stage)
            .join("config.resolved.toml")
    }

    pub fn lockin_xy_csv(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_xy.csv"))
    }

    /// Per-window GLS quality diagnostics (CSV only, never NPY).
    pub fn lockin_quality_csv(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_quality.csv"))
    }

    /// Frozen per-channel estimator snapshot (JSON only, never NPY).
    pub fn lockin_estimator_json(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_estimator.json"))
    }

    /// Retained exact calibration model bytes per channel (v4 manifests):
    /// the validated artifact bytes the loader hashed, so staged reruns
    /// survive external model removal.
    pub fn lockin_calibration_json(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_calibration.json"))
    }

    /// Conditional XY covariance artifact (CSV plus NPY mirror).
    pub fn lockin_covariance_csv(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_covariance.csv"))
    }

    /// NPY mirror of [`ArtifactPaths::lockin_covariance_csv`] with identical
    /// time-plus-covariance columns.
    pub fn lockin_covariance_npy(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_covariance.npy"))
    }

    pub fn lockin_xy_npy(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_xy.npy"))
    }

    pub fn lockin_rotated_csv(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_rotated.csv"))
    }

    pub fn lockin_rotated_covariance_csv(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_rotated_covariance.csv"))
    }

    pub fn lockin_rotated_npy(&self, channel: u8) -> PathBuf {
        self.analysis_dir()
            .join("lockin")
            .join(format!("ch{channel}_rotated.npy"))
    }

    pub fn moke_csv(&self) -> PathBuf {
        self.analysis_dir().join("moke").join("moke.csv")
    }

    pub fn moke_npy(&self) -> PathBuf {
        self.analysis_dir().join("moke").join("moke.npy")
    }

    /// Conditional MOKE angle variance per output window (R2c, CSV-only):
    /// one combined file with one column per channel, beside moke.csv.
    pub fn moke_variance_csv(&self) -> PathBuf {
        self.analysis_dir().join("moke").join("moke_variance.csv")
    }

    pub fn signal_dir(&self) -> PathBuf {
        self.analysis_dir().join("signal")
    }

    pub fn signal_csv(&self) -> PathBuf {
        self.signal_dir().join("signal.csv")
    }

    pub fn signal_npy(&self) -> PathBuf {
        self.signal_dir().join("signal.npy")
    }

    pub fn signal_plot_dir(&self) -> PathBuf {
        self.plot_dir().join("signal")
    }

    /// One combined plot holding every configured signal as its own panel.
    /// Per-channel signal figures are not part of the artifact contract.
    pub fn signal_combined_plot(&self) -> PathBuf {
        self.signal_plot_dir().join("mean.png")
    }

    pub fn sensor_dir(&self) -> PathBuf {
        self.analysis_dir().join("sensor")
    }

    pub fn sensor_csv(&self) -> PathBuf {
        self.sensor_dir().join("sensor.csv")
    }

    pub fn sensor_npy(&self) -> PathBuf {
        self.sensor_dir().join("sensor.npy")
    }

    pub fn plot_dir(&self) -> PathBuf {
        self.analysis_dir().join("plots")
    }

    pub fn reference_plot_dir(&self) -> PathBuf {
        self.plot_dir().join("reference")
    }

    pub fn sensor_plot_dir(&self) -> PathBuf {
        self.plot_dir().join("sensor")
    }

    pub fn lockin_plot_dir(&self) -> PathBuf {
        self.plot_dir().join("lockin")
    }

    pub fn phase_plot_dir(&self) -> PathBuf {
        self.plot_dir().join("phase")
    }

    pub fn moke_plot_dir(&self) -> PathBuf {
        self.plot_dir().join("moke")
    }

    pub fn reference_fit_plot(&self) -> PathBuf {
        self.reference_plot_dir().join("reference_fit.png")
    }

    pub fn reference_fft_plot(&self) -> PathBuf {
        self.reference_plot_dir().join("reference_fft.png")
    }

    pub fn sensor_raw_plot(&self, channel: u8) -> PathBuf {
        self.sensor_plot_dir().join(format!("ch{channel}_raw.png"))
    }

    pub fn sensor_integral_plot(&self, channel: u8) -> PathBuf {
        self.sensor_plot_dir()
            .join(format!("ch{channel}_integral.png"))
    }

    pub fn sensor_raw_combined_plot(&self) -> PathBuf {
        self.sensor_plot_dir().join("raw.png")
    }

    pub fn sensor_integral_combined_plot(&self) -> PathBuf {
        self.sensor_plot_dir().join("integral.png")
    }

    pub fn lockin_xy_plot(&self, channel: u8) -> PathBuf {
        self.lockin_plot_dir().join(format!("ch{channel}_xy.png"))
    }

    pub fn lockin_xy_combined_plot(&self) -> PathBuf {
        self.lockin_plot_dir().join("xy.png")
    }

    pub fn phase_rotated_plot(&self, channel: u8) -> PathBuf {
        self.phase_plot_dir()
            .join(format!("ch{channel}_rotated.png"))
    }

    pub fn phase_rotated_combined_plot(&self) -> PathBuf {
        self.phase_plot_dir().join("rotated.png")
    }

    pub fn phase_offset_plot(&self) -> PathBuf {
        self.phase_plot_dir().join("omega_t0.png")
    }

    pub fn moke_plot(&self) -> PathBuf {
        self.moke_plot_dir().join("angle.png")
    }

    pub fn moke_channel_plot(&self, channel: u8) -> PathBuf {
        self.moke_plot_dir().join(format!("ch{channel}_angle.png"))
    }

    pub fn moke_vm_plot(&self) -> PathBuf {
        self.moke_plot_dir().join("vm.png")
    }

    pub fn moke_vm_channel_plot(&self, channel: u8) -> PathBuf {
        self.moke_plot_dir().join(format!("ch{channel}_vm.png"))
    }

    pub fn debug_dir(&self) -> PathBuf {
        self.analysis_dir().join("debug")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactResolver {
    pub paths: ArtifactPaths,
}

impl ArtifactResolver {
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            paths: ArtifactPaths::new(run_dir),
        }
    }

    pub fn run_manifest(&self) -> PathBuf {
        self.paths.run_manifest()
    }

    pub fn source_config(&self) -> PathBuf {
        self.paths.source_config()
    }

    pub fn resolved_config(&self) -> PathBuf {
        self.paths.resolved_config()
    }

    pub fn acquisition_manifest(&self) -> PathBuf {
        let new_path = self.paths.acquisition_manifest();
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join("raw_waveform")
            .join("metadata.toml");
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn waveform_csv(&self) -> PathBuf {
        let new_path = self.paths.waveform_csv();
        if new_path.exists() {
            return new_path;
        }
        let legacy_path1 = self.paths.run_dir.join("raw.csv");
        if legacy_path1.exists() {
            return legacy_path1;
        }
        let legacy_path2 = self.paths.run_dir.join("raw_waveform").join("raw.csv");
        if legacy_path2.exists() {
            return legacy_path2;
        }
        new_path
    }

    pub fn raw_channel(&self, channel: u8) -> PathBuf {
        let new_path = self.paths.waveform_binary(channel);
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join("raw_waveform")
            .join(format!("ch{channel}.u16le"));
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn lockin_xy_csv(&self, channel: u8) -> PathBuf {
        let new_path = self.paths.lockin_xy_csv(channel);
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join(format!("lockin_results_ch{channel}.csv"));
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn lockin_quality_csv(&self, channel: u8) -> PathBuf {
        self.paths.lockin_quality_csv(channel)
    }

    pub fn lockin_estimator_json(&self, channel: u8) -> PathBuf {
        self.paths.lockin_estimator_json(channel)
    }

    pub fn lockin_calibration_json(&self, channel: u8) -> PathBuf {
        self.paths.lockin_calibration_json(channel)
    }

    pub fn lockin_covariance_csv(&self, channel: u8) -> PathBuf {
        self.paths.lockin_covariance_csv(channel)
    }

    pub fn lockin_covariance_npy(&self, channel: u8) -> PathBuf {
        self.paths.lockin_covariance_npy(channel)
    }

    pub fn lockin_rotated_covariance_csv(&self, channel: u8) -> PathBuf {
        self.paths.lockin_rotated_covariance_csv(channel)
    }

    pub fn lockin_rotated_csv(&self, channel: u8) -> PathBuf {
        let new_path = self.paths.lockin_rotated_csv(channel);
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join(format!("lockin_rotated_ch{channel}.csv"));
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn moke_csv(&self) -> PathBuf {
        let new_path = self.paths.moke_csv();
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self.paths.run_dir.join("kerr_results.csv");
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn signal_csv(&self) -> PathBuf {
        self.paths.signal_csv()
    }

    pub fn sensor_csv(&self) -> PathBuf {
        self.paths.sensor_csv()
    }

    pub fn lockin_xy_npy(&self, channel: u8) -> PathBuf {
        let new_path = self.paths.lockin_xy_npy(channel);
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join("analysis_npy")
            .join(format!("lockin_results_ch{channel}.npy"));
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn lockin_rotated_npy(&self, channel: u8) -> PathBuf {
        let new_path = self.paths.lockin_rotated_npy(channel);
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join("analysis_npy")
            .join(format!("lockin_rotated_ch{channel}.npy"));
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn moke_variance_csv(&self) -> PathBuf {
        self.paths.moke_variance_csv()
    }

    pub fn moke_npy(&self) -> PathBuf {
        let new_path = self.paths.moke_npy();
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self
            .paths
            .run_dir
            .join("analysis_npy")
            .join("kerr_results.npy");
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }

    pub fn analysis_manifest(&self) -> PathBuf {
        let new_path = self.paths.analysis_manifest();
        if new_path.exists() {
            return new_path;
        }
        let legacy_path = self.paths.run_dir.join("analysis_metadata.toml");
        if legacy_path.exists() {
            return legacy_path;
        }
        new_path
    }
}

#[cfg(test)]
mod tests {
    use super::ArtifactPaths;
    use std::path::PathBuf;

    #[test]
    fn canonical_plot_paths_are_stage_scoped_and_staging_aware() {
        let paths = ArtifactPaths::new(PathBuf::from("shot with space measurement"));
        assert_eq!(
            paths.reference_fit_plot(),
            PathBuf::from("shot with space measurement/analysis/plots/reference/reference_fit.png")
        );
        assert_eq!(
            paths.lockin_xy_plot(3),
            PathBuf::from("shot with space measurement/analysis/plots/lockin/ch3_xy.png")
        );
        assert_eq!(
            paths.phase_rotated_plot(3),
            PathBuf::from("shot with space measurement/analysis/plots/phase/ch3_rotated.png")
        );
        assert_eq!(
            paths.moke_plot(),
            PathBuf::from("shot with space measurement/analysis/plots/moke/angle.png")
        );
        assert_eq!(
            paths.to_staging().moke_plot(),
            PathBuf::from("shot with space measurement/analysis.incomplete/plots/moke/angle.png")
        );
    }
}
