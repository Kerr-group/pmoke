use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempConfig {
    dir: PathBuf,
    path: PathBuf,
}

impl TempConfig {
    fn new(contents: &str) -> Self {
        let dir = env::temp_dir().join(format!(
            "pmoke_config_migration_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                + u128::from(TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed))
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, contents).unwrap();
        Self { dir, path }
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn v3_config() -> String {
    include_str!("../../../tests/fixtures/config_v3.toml").to_string()
}

#[test]
fn v3_plan_generates_readable_v4_and_preserves_sensor_scale() {
    let fixture = TempConfig::new(&v3_config());
    let plan = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap();
    assert_eq!(plan.source_version, 3);
    assert_eq!(plan.target_version, 4);
    assert!(plan.changed);
    assert!(plan.target_toml.contains("version = 4"));
    assert!(plan.target_toml.contains("max_abs = 55.0"));
    assert!(plan.target_toml.contains("polarity = -1"));
    assert!(matches!(
        load_from_str(&plan.target_toml),
        ConfigLoad::Ready { .. }
    ));
}

#[test]
fn legacy_timebase_requires_lossy_acceptance() {
    let v2 = v3_config().replacen(
        "version = 3",
        "version = 2\n\n[timebase]\nt0 = -8.5e-3\ndt = 0.5e-9",
        1,
    );
    let fixture = TempConfig::new(&v2);
    let plan = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap();
    assert!(plan.has_lossy_changes());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.message.contains("[timebase]"))
    );
}

#[test]
fn v1_filter_length_is_migrated_with_explicit_lossy_warning() {
    let v1 = v3_config()
        .replacen(
            "version = 3",
            "version = 1\n\n[timebase]\nt0 = -8.5e-3\ndt = 0.5e-9",
            1,
        )
        .replace(
            "lpf_kind = \"boxcar_legacy\"\nlpf_half_window_cycles = 1.0",
            "filter_length_samples = 1",
        );
    let fixture = TempConfig::new(&v1);
    let plan = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap();
    assert!(plan.target_toml.contains("kind = \"boxcar_legacy\""));
    assert!(plan.target_toml.contains("half_window_cycles = 1.0"));
    assert!(plan.issues.iter().any(|issue| {
        issue.level == MigrationLevel::Lossy && issue.message.contains("filter_length_samples")
    }));
}

#[test]
fn all_legacy_lockin_filter_variants_round_trip_to_v4() {
    for replacement in [
        "lpf_kind = \"boxcar_legacy\"\nlpf_half_window_cycles = 1.0",
        "lpf_kind = \"fir_boxcar_enbw\"\nlpf_half_window_cycles = 1.0",
        "lpf_kind = \"fir_zero_phase\"\nlpf_half_window_cycles = 1.0\nlpf_cutoff_ref_ratio = 0.25\nlpf_stopband_atten_db = 80.0",
        "lpf_kind = \"sync_iir_zero_phase\"\nlpf_half_window_cycles = 1.0\nlpf_cutoff_hz = 100.0\nlpf_sync_average_cycles = 2.0\nlpf_iir_order = 4",
    ] {
        let config = v3_config().replace(
            "lpf_kind = \"boxcar_legacy\"\nlpf_half_window_cycles = 1.0",
            replacement,
        );
        let fixture = TempConfig::new(&config);
        let plan = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap();
        assert!(matches!(
            load_from_str(&plan.target_toml),
            ConfigLoad::Ready { .. }
        ));
    }
}

#[test]
fn factor_scale_and_phase_expressions_are_preserved() {
    let config = v3_config()
        .replace("scale_to_abs_max = -55.0", "factor = -39364.5")
        .replace(
            "m_omega_t0_offset = [0, 0, 0, 0, 0, 0]",
            "m_omega_t0_offset = [\"pi / 2\", 0, 0, 0, 0, 0]",
        );
    let fixture = TempConfig::new(&config);
    let plan = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap();
    assert!(plan.target_toml.contains("factor = -39364.5"));
    let ConfigLoad::Ready { config, .. } = load_from_str(&plan.target_toml) else {
        panic!("generated v4 config must load");
    };
    assert_eq!(
        config.phase.m_omega_t0_offset[0],
        std::f64::consts::PI / 2.0
    );
}

#[test]
fn unused_channels_are_reported_as_lossy() {
    let config = v3_config().replace(
        "[pulse]",
        "[[channels]]\nindex = 4\nlabel = \"unused\"\n\n[pulse]",
    );
    let fixture = TempConfig::new(&config);
    let plan = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap();
    assert!(
        plan.issues
            .iter()
            .any(|issue| { issue.level == MigrationLevel::Lossy && issue.message.contains("ch4") })
    );
}

#[test]
fn v4_upgrade_is_a_no_op_instead_of_a_formatter() {
    let legacy = TempConfig::new(&v3_config());
    let upgraded = plan_upgrade(&legacy.path, Some(&legacy.path), 4)
        .unwrap()
        .target_toml;
    let current = TempConfig::new(&upgraded);
    let plan = plan_upgrade(&current.path, Some(&current.path), 4).unwrap();
    assert!(!plan.changed);
    assert_eq!(plan.target_toml, upgraded);
}

#[test]
fn missing_scope_blocks_v4_migration() {
    let config = v3_config().replace(
        "[instruments.oscilloscope]\nconnection = { protocol = \"tcpip\", ip = \"192.168.10.100\", port = 55255 }\nmodel = \"DHO5108\"\n\n",
        "",
    );
    let fixture = TempConfig::new(&config);
    let error = plan_upgrade(&fixture.path, Some(&fixture.path), 4).unwrap_err();
    assert!(format!("{error:#}").contains("no oscilloscope"));
}
