use crate::cli::Cli;
use crate::ui;
use anyhow::{Context, Result};
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use dirs;
use std::{fs, io::Write, path::PathBuf};

pub fn install_completion(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    let mut buffer = Vec::new();
    generate(shell, &mut cmd, "pmoke", &mut buffer);

    match shell {
        Shell::Fish => {
            let home = dirs::home_dir().context("could not determine home directory")?;
            let dest = home.join(".config/fish/completions/pmoke.fish");
            let parent = dest
                .parent()
                .context("fish completion destination has no parent directory")?;
            fs::create_dir_all(parent)?;
            fs::write(&dest, buffer)
                .with_context(|| format!("failed to write completion to {:?}", dest))?;
            ui::success(format!("installed fish completion at {}", dest.display()));
        }
        Shell::PowerShell => {
            let profile_path = std::env::var_os("PROFILE").map(PathBuf::from).or_else(|| {
                dirs::document_dir().map(|mut path| {
                    path.push("WindowsPowerShell");
                    path.push("Microsoft.PowerShell_profile.ps1");
                    path
                })
            });

            if let Some(path) = profile_path {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }

                let script = String::from_utf8_lossy(&buffer);

                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .with_context(|| format!("failed to open profile at {:?}", path))?;

                writeln!(file, "\n# pmoke completion\n{}", script)?;

                ui::success(format!(
                    "appended pmoke completion to PowerShell profile {:?}",
                    path
                ));
            } else {
                ui::warn("could not determine PowerShell profile path automatically");
                ui::info(
                    "try manually: pmoke completions powershell | Out-String | Invoke-Expression",
                );
            }
        }
        other => {
            ui::skipped(format!(
                "{other} is not yet supported for automatic installation"
            ));
            ui::info(format!(
                "manual install: pmoke completions {other} > <completion-path>"
            ));
        }
    }
    Ok(())
}
