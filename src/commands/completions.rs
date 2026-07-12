use crate::cli::Cli;
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const POWERSHELL_COMPLETION_FILENAME: &str = "pmoke_completion.ps1";
const POWERSHELL_BLOCK_START: &str = "# >>> pmoke completion >>>";
const POWERSHELL_BLOCK_END: &str = "# <<< pmoke completion <<<";
const LEGACY_POWERSHELL_MARKER: &str = "# pmoke completion";
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Clone, Copy)]
enum ProfileEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

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
            let profile_path = powershell_profile_path();
            if let Some(path) = profile_path {
                install_powershell_completion(&path, &buffer)?;
                ui::success(format!(
                    "installed PowerShell completion for profile {}",
                    path.display()
                ));
            } else {
                ui::warn("could not determine PowerShell profile path automatically");
                ui::info("set PROFILE to the profile path and run this command again");
            }
        }
        _ => {
            io::stdout()
                .write_all(&buffer)
                .context("failed to write completion script to standard output")?;
        }
    }
    Ok(())
}

fn powershell_profile_path() -> Option<PathBuf> {
    std::env::var_os("PROFILE").map(PathBuf::from).or_else(|| {
        dirs::document_dir().map(|mut path| {
            path.push("WindowsPowerShell");
            path.push("Microsoft.PowerShell_profile.ps1");
            path
        })
    })
}

fn install_powershell_completion(profile_path: &Path, generated: &[u8]) -> Result<()> {
    let parent = profile_path
        .parent()
        .context("PowerShell profile path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create PowerShell profile directory {}",
            parent.display()
        )
    })?;

    let completion_path = parent.join(POWERSHELL_COMPLETION_FILENAME);
    let script = trim_leading_line_endings(generated);
    let (existing, profile_encoding) = match fs::read(profile_path) {
        Ok(bytes) => decode_profile(&bytes).with_context(|| {
            format!(
                "failed to decode PowerShell profile {}",
                profile_path.display()
            )
        })?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            (String::new(), ProfileEncoding::Utf8Bom)
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read PowerShell profile {}",
                    profile_path.display()
                )
            });
        }
    };
    let without_legacy = remove_legacy_inline_completion(&existing)?;
    let source_path = completion_path.to_string_lossy().replace('\'', "''");
    let block = format!("{POWERSHELL_BLOCK_START}\n. '{source_path}'\n{POWERSHELL_BLOCK_END}");
    let updated = upsert_managed_block(&without_legacy, &block)?;

    let mut completion_bytes = Vec::with_capacity(UTF8_BOM.len() + script.len());
    completion_bytes.extend_from_slice(UTF8_BOM);
    completion_bytes.extend_from_slice(script);
    fs::write(&completion_path, completion_bytes).with_context(|| {
        format!(
            "failed to write PowerShell completion to {}",
            completion_path.display()
        )
    })?;
    if updated != existing {
        fs::write(profile_path, encode_profile(&updated, profile_encoding)).with_context(|| {
            format!(
                "failed to update PowerShell profile {}",
                profile_path.display()
            )
        })?;
    }
    Ok(())
}

fn decode_profile(bytes: &[u8]) -> Result<(String, ProfileEncoding)> {
    if let Some(content) = bytes.strip_prefix(UTF8_BOM) {
        return Ok((
            String::from_utf8(content.to_vec()).context("profile is not valid UTF-8")?,
            ProfileEncoding::Utf8Bom,
        ));
    }
    if let Some(content) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok((decode_utf16(content, true)?, ProfileEncoding::Utf16Le));
    }
    if let Some(content) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Ok((decode_utf16(content, false)?, ProfileEncoding::Utf16Be));
    }
    Ok((
        String::from_utf8(bytes.to_vec()).context("profile is not valid UTF-8")?,
        ProfileEncoding::Utf8,
    ))
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        bail!("UTF-16 profile has an incomplete code unit");
    }
    let units = bytes.chunks_exact(2).map(|chunk| {
        let pair = [chunk[0], chunk[1]];
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });
    char::decode_utf16(units)
        .map(|item| item.map_err(|error| anyhow!(error)))
        .collect()
}

fn encode_profile(text: &str, encoding: ProfileEncoding) -> Vec<u8> {
    match encoding {
        ProfileEncoding::Utf8 => text.as_bytes().to_vec(),
        ProfileEncoding::Utf8Bom => {
            let mut bytes = Vec::with_capacity(UTF8_BOM.len() + text.len());
            bytes.extend_from_slice(UTF8_BOM);
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        ProfileEncoding::Utf16Le | ProfileEncoding::Utf16Be => {
            let mut bytes = Vec::with_capacity(2 + text.len() * 2);
            let little_endian = matches!(encoding, ProfileEncoding::Utf16Le);
            bytes.extend_from_slice(if little_endian {
                &[0xFF, 0xFE]
            } else {
                &[0xFE, 0xFF]
            });
            for unit in text.encode_utf16() {
                let encoded = if little_endian {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                bytes.extend_from_slice(&encoded);
            }
            bytes
        }
    }
}

fn trim_leading_line_endings(bytes: &[u8]) -> &[u8] {
    let first_content = bytes
        .iter()
        .position(|byte| !matches!(byte, b'\r' | b'\n'))
        .unwrap_or(bytes.len());
    &bytes[first_content..]
}

fn remove_legacy_inline_completion(profile: &str) -> Result<String> {
    let mut retained = profile;
    while let Some(marker) = retained.rfind(LEGACY_POWERSHELL_MARKER) {
        let suffix = &retained[marker..];
        let is_legacy_pmoke_block = suffix.contains("using namespace System.Management.Automation")
            && suffix.contains("Register-ArgumentCompleter -Native -CommandName 'pmoke'");
        if !is_legacy_pmoke_block {
            break;
        }
        let Some(sort_position) = suffix.rfind("Sort-Object -Property ListItemText") else {
            bail!(
                "legacy inline pmoke completion was found in the PowerShell profile but could not be migrated safely; remove the block beginning with '{LEGACY_POWERSHELL_MARKER}' and retry"
            );
        };
        let after_sort = &suffix[sort_position + "Sort-Object -Property ListItemText".len()..];
        if after_sort.trim() != "}" {
            bail!(
                "legacy inline pmoke completion is followed by other PowerShell profile statements; remove only the old pmoke block manually and retry"
            );
        }
        retained = retained[..marker].trim_end();
    }
    Ok(retained.to_string())
}

fn upsert_managed_block(profile: &str, block: &str) -> Result<String> {
    match (
        profile.find(POWERSHELL_BLOCK_START),
        profile.find(POWERSHELL_BLOCK_END),
    ) {
        (Some(start), Some(end)) if start <= end => {
            let end = end + POWERSHELL_BLOCK_END.len();
            let mut updated = String::with_capacity(profile.len() - (end - start) + block.len());
            updated.push_str(&profile[..start]);
            updated.push_str(block);
            updated.push_str(&profile[end..]);
            Ok(updated)
        }
        (None, None) => {
            let prefix = profile.trim_end();
            if prefix.is_empty() {
                Ok(format!("{block}\n"))
            } else {
                Ok(format!("{prefix}\n\n{block}\n"))
            }
        }
        _ => bail!(
            "PowerShell profile contains an incomplete pmoke completion block; remove it and retry"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_inline_completion_is_removed_without_touching_profile_prefix() {
        let legacy_block = r#"# pmoke completion
using namespace System.Management.Automation
using namespace System.Management.Automation.Language
Register-ArgumentCompleter -Native -CommandName 'pmoke' -ScriptBlock {
    $completions | Sort-Object -Property ListItemText
}
"#;
        let profile = format!("Set-Alias ll Get-ChildItem\n\n{legacy_block}\n{legacy_block}");

        assert_eq!(
            remove_legacy_inline_completion(&profile).unwrap(),
            "Set-Alias ll Get-ChildItem"
        );
    }

    #[test]
    fn unrelated_legacy_marker_is_not_removed() {
        let profile = "# pmoke completion\nWrite-Output 'custom'\n";
        assert_eq!(remove_legacy_inline_completion(profile).unwrap(), profile);
    }

    #[test]
    fn legacy_completion_followed_by_user_code_is_not_removed() {
        let profile = r#"# pmoke completion
using namespace System.Management.Automation
Register-ArgumentCompleter -Native -CommandName 'pmoke' -ScriptBlock {
    $completions | Sort-Object -Property ListItemText
}
function Keep-Me {
}
"#;
        let error = remove_legacy_inline_completion(profile).unwrap_err();
        assert!(error.to_string().contains("followed by other"));
    }

    #[test]
    fn managed_profile_block_is_idempotent_and_preserves_surrounding_content() {
        let first = upsert_managed_block("before\n", "managed-v1").unwrap();
        let profile = first.replace(
            "managed-v1",
            &format!("{POWERSHELL_BLOCK_START}\nold\n{POWERSHELL_BLOCK_END}"),
        ) + "after\n";
        let replacement = format!("{POWERSHELL_BLOCK_START}\nnew\n{POWERSHELL_BLOCK_END}");
        let updated = upsert_managed_block(&profile, &replacement).unwrap();

        assert!(updated.contains("before"));
        assert!(updated.contains("new"));
        assert!(!updated.contains("old"));
        assert!(updated.ends_with("after\n"));
        assert_eq!(
            upsert_managed_block(&updated, &replacement).unwrap(),
            updated
        );
    }

    #[test]
    fn powershell_profile_contains_only_a_dot_source_statement() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pmoke_completion_test_{}_{}",
            std::process::id(),
            nonce
        ));
        let _ = fs::remove_dir_all(&root);
        let profile = root.join("Microsoft.PowerShell_profile.ps1");
        let generated = b"\r\nusing namespace System.Management.Automation\r\ncompletion-body\r\n";

        install_powershell_completion(&profile, generated).unwrap();
        install_powershell_completion(&profile, generated).unwrap();

        let profile_bytes = fs::read(&profile).unwrap();
        let (profile_text, _) = decode_profile(&profile_bytes).unwrap();
        let completion = fs::read(root.join(POWERSHELL_COMPLETION_FILENAME)).unwrap();
        assert!(!profile_text.contains("using namespace"));
        assert_eq!(profile_text.matches(POWERSHELL_BLOCK_START).count(), 1);
        assert!(profile_text.contains("pmoke_completion.ps1'"));
        assert!(completion.starts_with(UTF8_BOM));
        assert!(completion[UTF8_BOM.len()..].starts_with(b"using namespace"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_encoding_round_trips_utf8_and_utf16() {
        let text = "Write-Output '測定'\n";
        for encoding in [
            ProfileEncoding::Utf8,
            ProfileEncoding::Utf8Bom,
            ProfileEncoding::Utf16Le,
            ProfileEncoding::Utf16Be,
        ] {
            let bytes = encode_profile(text, encoding);
            let (decoded, _) = decode_profile(&bytes).unwrap();
            assert_eq!(decoded, text);
        }
    }
}
