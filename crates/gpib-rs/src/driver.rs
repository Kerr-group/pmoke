//! Driver initialization helpers (platform-aware).

use crate::error::{Result, sys_err};
use std::{fs, process::Command};

#[cfg(not(target_os = "windows"))]
fn have_gpib_dev_nodes() -> bool {
    if let Ok(rd) = fs::read_dir("/dev") {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n.starts_with("gpib") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(target_os = "windows")]
pub(crate) fn ensure_driver_configured() -> Result<()> {
    // Windows uses NI MAX; nothing to do here.
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn ensure_driver_configured() -> Result<()> {
    if have_gpib_dev_nodes() {
        return Ok(());
    }
    let status = Command::new("gpib_config").arg("-v").status();
    match status {
        Ok(st) if st.success() => {
            if have_gpib_dev_nodes() {
                eprintln!("(info) gpib_config applied and /dev/gpib* appeared");
                Ok(())
            } else {
                Err(sys_err("gpib_config", "ran ok but /dev/gpib* not found"))
            }
        }
        Ok(st) => Err(sys_err("gpib_config", format!("exit status {}", st))),
        Err(e) => Err(sys_err("gpib_config", format!("spawn error: {e}"))),
    }
}
