use crate::cli::Cli;
use anyhow::{Context, Result};
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use dirs;
use std::{fs, path::PathBuf};

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
            if let Some(profile) = std::env::var_os("PROFILE") {
                let profile_path = PathBuf::from(profile);
                let script = String::from_utf8_lossy(&buffer);
                fs::write(&profile_path, format!("\n# pmoke completion\n{}", script))
                    .with_context(|| {
                        format!("failed to append completion to {:?}", profile_path)
                    })?;
                println!(
                    "✅ Added pmoke completion to PowerShell profile {:?}",
                    profile_path
                );
            } else {
                println!("⚠️  Could not find PowerShell profile path. Try manually:");
                println!("pmoke completions powershell | Out-String | Invoke-Expression");
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
