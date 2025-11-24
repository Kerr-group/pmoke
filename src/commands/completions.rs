use crate::cli::Cli;
use anyhow::{Context, Result};
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use dirs;
use std::{fs, path::PathBuf, io::Write}; 

pub fn install_completion(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    let mut buffer = Vec::new();
    generate(shell, &mut cmd, "pmoke", &mut buffer);

    match shell {
        Shell::Fish => {
            let dest = dirs::home_dir()
                .unwrap()
                .join(".config/fish/completions/pmoke.fish");
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::write(&dest, buffer)
                .with_context(|| format!("failed to write completion to {:?}", dest))?;
            println!("✅ Installed fish completion at {}", dest.display());
        }
        Shell::PowerShell => {
            let profile_path = std::env::var_os("PROFILE")
                .map(PathBuf::from)
                .or_else(|| {
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
                    .write(true)
                    .append(true) 
                    .open(&path)
                    .with_context(|| format!("failed to open profile at {:?}", path))?;

                writeln!(file, "\n# pmoke completion\n{}", script)?;

                println!(
                    "✅ Appended pmoke completion to PowerShell profile {:?}",
                    path
                );
            } else {
                println!("⚠️  Could not determine PowerShell profile path automatically.");
                println!("Try manually: pmoke completions powershell | Out-String | Invoke-Expression");
            }
        }
        other => {
            println!("{} is not yet supported for automatic installation.", other);
            println!("You can manually install with:");
            println!("pmoke completions {} > <completion-path>", other);
        }
    }
    Ok(())
}