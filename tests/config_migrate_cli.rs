use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pmoke_config_migrate_cli_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pmoke(config: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmoke"))
        .arg("--config")
        .arg(config)
        .args(arguments)
        .output()
        .unwrap()
}

fn pmoke_in_dir(
    working_dir: &std::path::Path,
    config: &std::path::Path,
    arguments: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmoke"))
        .current_dir(working_dir)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .output()
        .unwrap()
}

fn v3_config() -> String {
    include_str!("fixtures/config_v3.toml").replace("\r\n", "\n")
}

#[test]
fn check_and_output_enforce_lossy_acceptance_end_to_end() {
    let dir = TempDir::new();
    let source = dir.0.join("config.toml");
    let target = dir.0.join("config.v4.toml");
    let original = v3_config();
    fs::write(&source, &original).unwrap();

    let blocked_check = pmoke(&source, &["config", "migrate", "--check"]);
    assert_eq!(blocked_check.status.code(), Some(2));

    let available_check = pmoke(&source, &["config", "migrate", "--check", "--accept-lossy"]);
    assert_eq!(available_check.status.code(), Some(1));

    let blocked_output = pmoke(
        &source,
        &["config", "migrate", "--output", target.to_str().unwrap()],
    );
    assert!(!blocked_output.status.success());
    assert!(!target.exists());
    assert_eq!(fs::read_to_string(&source).unwrap(), original);

    let written = pmoke(
        &source,
        &[
            "config",
            "migrate",
            "--output",
            target.to_str().unwrap(),
            "--accept-lossy",
        ],
    );
    assert!(
        written.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&written.stderr)
    );
    assert!(fs::read_to_string(&target).unwrap().contains("version = 4"));
    assert_eq!(fs::read_to_string(&source).unwrap(), original);

    let latest = pmoke(&target, &["config", "migrate", "--check"]);
    assert_eq!(latest.status.code(), Some(0));
}

#[test]
fn v2_csv_without_time_stays_executable_instead_of_advancing_to_v4() {
    let dir = TempDir::new();
    let source = dir.0.join("config.toml");
    let v2 = v3_config().replacen(
        "version = 3",
        "version = 2\n\n[timebase]\nt0 = -8.5e-3\ndt = 0.5e-9",
        1,
    );
    fs::write(&source, &v2).unwrap();
    fs::write(
        dir.0.join("raw.csv"),
        "Channel 1 (V),Channel 2 (V),Channel 3 (V)\n0.1,0.2,0.3\n",
    )
    .unwrap();

    let compatible = pmoke_in_dir(&dir.0, &source, &["config", "migrate", "--check"]);
    assert_eq!(compatible.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&compatible.stdout).contains("Status: LIMITED"));
    assert_eq!(fs::read_to_string(&source).unwrap(), v2);

    let forced_v4 = pmoke_in_dir(
        &dir.0,
        &source,
        &[
            "config",
            "migrate",
            "--check",
            "--to",
            "4",
            "--accept-lossy",
        ],
    );
    assert_eq!(forced_v4.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&forced_v4.stderr).contains("would not be executable"));
    assert_eq!(fs::read_to_string(&source).unwrap(), v2);
}

#[test]
fn v2_csv_with_recorded_time_can_advance_to_v4() {
    let dir = TempDir::new();
    let source = dir.0.join("config.toml");
    let target = dir.0.join("config.v4.toml");
    let v2 = v3_config().replacen(
        "version = 3",
        "version = 2\n\n[timebase]\nt0 = -8.5e-3\ndt = 0.5e-9",
        1,
    );
    fs::write(&source, &v2).unwrap();
    fs::write(
        dir.0.join("raw.csv"),
        "time (s),Channel 1 (V),Channel 2 (V),Channel 3 (V)\n0.0,0.1,0.2,0.3\n",
    )
    .unwrap();

    let available = pmoke_in_dir(&dir.0, &source, &["config", "migrate", "--check"]);
    assert_eq!(
        available.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&available.stdout),
        String::from_utf8_lossy(&available.stderr)
    );

    let written = pmoke_in_dir(
        &dir.0,
        &source,
        &["config", "migrate", "--output", target.to_str().unwrap()],
    );
    assert!(
        written.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&written.stderr)
    );
    let migrated = fs::read_to_string(&target).unwrap();
    assert!(migrated.contains("version = 4"));
    assert!(!migrated.contains("[timebase]"));
}
