use super::{
    ConfigLoad, Connection, FetchAnalysisInput, FetchOutput, LockinLpfKind, PlotDecimation,
    ValidationTarget, load_from_path, load_from_str, render_normalized_config, validate_for_target,
    validate_sensor_metadata,
};
use std::fs;

mod general;
mod legacy;
mod v4;

fn v2_base_lockin(lockin: &str) -> String {
    format!(
        r#"
version = 2

[timebase]
t0 = 0.0
dt = 1.0

[roles]
sensor_ch = [1]
reference_ch = 2
signal_ch = [3]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[pulse]
bg_window_before = {{ start = -1.0, end = -0.5 }}
bg_window_after = {{ start = 0.5, end = 1.0 }}

[reference]
fft_window = {{ start = 0.0, end = 1.0 }}
stride_samples = 10
window_samples = 10

[lockin]
{lockin}

[phase]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#
    )
}

fn v3_base_lockin(lockin: &str) -> String {
    format!(
        r#"
version = 3

[roles]
sensor_ch = [1]
reference_ch = 2
signal_ch = [3]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[pulse]
bg_window_before = {{ start = -1.0, end = -0.5 }}
bg_window_after = {{ start = 0.5, end = 1.0 }}

[reference]
fft_window = {{ start = 0.0, end = 1.0 }}
stride_samples = 10
window_samples = 10

[lockin]
{lockin}

[phase]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#
    )
}
