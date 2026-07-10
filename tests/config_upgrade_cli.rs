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
            "pmoke_config_upgrade_cli_{}_{}_{}",
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

#[test]
fn check_and_output_enforce_lossy_acceptance_end_to_end() {
    let dir = TempDir::new();
    let source = dir.0.join("config.toml");
    let target = dir.0.join("config.v4.toml");
    let original = include_str!("fixtures/config_v3.toml");
    fs::write(&source, original).unwrap();

    let blocked_check = pmoke(&source, &["config", "upgrade", "--check"]);
    assert_eq!(blocked_check.status.code(), Some(2));

    let available_check = pmoke(&source, &["config", "upgrade", "--check", "--accept-lossy"]);
    assert_eq!(available_check.status.code(), Some(1));

    let blocked_output = pmoke(
        &source,
        &["config", "upgrade", "--output", target.to_str().unwrap()],
    );
    assert!(!blocked_output.status.success());
    assert!(!target.exists());
    assert_eq!(fs::read_to_string(&source).unwrap(), original);

    let written = pmoke(
        &source,
        &[
            "config",
            "upgrade",
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

    let latest = pmoke(&target, &["config", "upgrade", "--check"]);
    assert_eq!(latest.status.code(), Some(0));
}
