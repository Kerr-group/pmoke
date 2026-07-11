use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactPaths {
    pub run_dir: PathBuf,
}

impl ArtifactPaths {
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            run_dir: run_dir.into(),
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

    pub fn acquisition_dir(&self) -> PathBuf {
        self.run_dir.join("raw_waveform")
    }

    pub fn acquisition_manifest(&self) -> PathBuf {
        self.acquisition_dir().join("metadata.toml")
    }

    pub fn waveform_dir(&self) -> PathBuf {
        self.acquisition_dir()
    }

    pub fn waveform_binary(&self, channel: u8) -> PathBuf {
        self.waveform_dir().join(format!("ch{channel}.u16le"))
    }

    pub fn waveform_csv(&self) -> PathBuf {
        self.run_dir.join("raw.csv")
    }

    pub fn oscilloscope_screenshot(&self) -> PathBuf {
        self.run_dir.join("oscilloscope.png")
    }

    pub fn analysis_dir(&self) -> PathBuf {
        self.run_dir.clone()
    }

    pub fn analysis_manifest(&self) -> PathBuf {
        self.run_dir.join("analysis_metadata.toml")
    }

    pub fn lockin_xy_csv(&self, channel: u8) -> PathBuf {
        self.run_dir.join(format!("lockin_results_ch{channel}.csv"))
    }

    pub fn lockin_xy_npy(&self, channel: u8) -> PathBuf {
        self.run_dir
            .join("analysis_npy")
            .join(format!("lockin_results_ch{channel}.npy"))
    }

    pub fn lockin_rotated_csv(&self, channel: u8) -> PathBuf {
        self.run_dir.join(format!("lockin_rotated_ch{channel}.csv"))
    }

    pub fn lockin_rotated_npy(&self, channel: u8) -> PathBuf {
        self.run_dir
            .join("analysis_npy")
            .join(format!("lockin_rotated_ch{channel}.npy"))
    }

    pub fn kerr_csv(&self) -> PathBuf {
        self.run_dir.join("kerr_results.csv")
    }

    pub fn kerr_npy(&self) -> PathBuf {
        self.run_dir.join("analysis_npy").join("kerr_results.npy")
    }
}
