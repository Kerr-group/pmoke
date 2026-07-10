use super::{Config, ConfigLoad, load_from_path, load_from_str, render_config_v4};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const LATEST_CONFIG_VERSION: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationLevel {
    Notice,
    Lossy,
}

impl MigrationLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Notice => "NOTICE",
            Self::Lossy => "LOSSY",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationIssue {
    pub level: MigrationLevel,
    pub message: String,
}

impl MigrationIssue {
    fn notice(message: impl Into<String>) -> Self {
        Self {
            level: MigrationLevel::Notice,
            message: message.into(),
        }
    }

    fn lossy(message: impl Into<String>) -> Self {
        Self {
            level: MigrationLevel::Lossy,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub source_version: u32,
    pub target_version: u32,
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub target_toml: String,
    pub issues: Vec<MigrationIssue>,
    pub changed: bool,
    pub(crate) original: Vec<u8>,
}

impl MigrationPlan {
    pub fn has_lossy_changes(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.level == MigrationLevel::Lossy)
    }

    pub fn compatibility_label(&self) -> &'static str {
        if !self.changed {
            "LATEST"
        } else if self.has_lossy_changes() {
            "LOSSY"
        } else {
            "EXACT"
        }
    }
}

pub fn plan_upgrade(
    source_path: impl AsRef<Path>,
    destination_path: Option<&Path>,
    target_version: u32,
) -> Result<MigrationPlan> {
    let source_path = source_path.as_ref();
    let original = fs::read(source_path)
        .with_context(|| format!("failed to read config: {}", source_path.display()))?;
    let source_text = std::str::from_utf8(&original)
        .with_context(|| format!("config is not UTF-8: {}", source_path.display()))?;
    let source_version = declared_version(source_text)?;

    if target_version != LATEST_CONFIG_VERSION {
        bail!(
            "unsupported migration target v{target_version}; this pmoke supports config upgrade to v{LATEST_CONFIG_VERSION}"
        );
    }
    if source_version > target_version {
        bail!(
            "config downgrade is not supported (source v{source_version}, target v{target_version})"
        );
    }

    let destination_path = destination_path.unwrap_or(source_path).to_path_buf();
    let (config, source_warnings) = ready_config(load_from_path(source_path), "source config")?;

    if source_version == target_version {
        return Ok(MigrationPlan {
            source_version,
            target_version,
            source_path: source_path.to_path_buf(),
            destination_path,
            target_toml: source_text.to_string(),
            issues: Vec::new(),
            changed: false,
            original,
        });
    }

    let mut issues = vec![MigrationIssue::notice(
        "comments, table order, whitespace, and numeric formatting are not preserved",
    )];
    issues.extend(
        source_warnings
            .into_iter()
            .map(|warning| MigrationIssue::notice(warning.message)),
    );

    if config.legacy_timebase.is_some() {
        issues.push(MigrationIssue::lossy(
            "legacy [timebase] is not representable in v4; CSV files without a recorded time column may no longer be readable",
        ));
    }
    if source_version == 1 && has_filter_length_samples(source_text)? {
        issues.push(MigrationIssue::lossy(
            "v1 lockin.filter_length_samples is interpreted as lockin.filter.half_window_cycles by the existing compatibility normalization",
        ));
    }
    if source_version == 1 {
        issues.push(MigrationIssue::lossy(
            "the permissive v1 schema may contain unrecognized legacy keys; only recognized v1 settings are migrated",
        ));
    }

    inspect_channel_losses(&config, &mut issues);
    inspect_artifact_base_change(source_path, &destination_path, &mut issues)?;

    let target_toml = render_config_v4(&config)
        .context("source config cannot be represented by the v4 output schema")?;
    let (target_config, target_warnings) =
        ready_config(load_from_str(&target_toml), "generated v4 config")?;
    issues.extend(
        target_warnings
            .into_iter()
            .map(|warning| MigrationIssue::notice(warning.message)),
    );

    verify_preserved_semantics(config, target_config)?;

    Ok(MigrationPlan {
        source_version,
        target_version,
        source_path: source_path.to_path_buf(),
        destination_path,
        target_toml,
        issues,
        changed: true,
        original,
    })
}

fn declared_version(source: &str) -> Result<u32> {
    let value = toml::from_str::<toml::Value>(source).context("failed to parse source TOML")?;
    let version = value
        .get("version")
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| anyhow!("missing required integer config version"))?;
    u32::try_from(version).map_err(|_| anyhow!("config version must be a non-negative integer"))
}

fn has_filter_length_samples(source: &str) -> Result<bool> {
    let value = toml::from_str::<toml::Value>(source).context("failed to inspect source TOML")?;
    Ok(value
        .get("lockin")
        .and_then(|lockin| lockin.get("filter_length_samples"))
        .is_some())
}

fn ready_config(load: ConfigLoad, label: &str) -> Result<(Config, Vec<super::ConfigWarning>)> {
    match load {
        ConfigLoad::Ready { config, warnings } => Ok((config, warnings)),
        ConfigLoad::Diagnostics(diagnostics) => {
            let details = diagnostics
                .diagnostics
                .iter()
                .map(|diagnostic| match diagnostic.path.as_deref() {
                    Some(path) => format!("{path}: {}", diagnostic.message),
                    None => diagnostic.message.clone(),
                })
                .collect::<Vec<_>>()
                .join("; ");
            bail!("{label} is not migratable: {details}")
        }
    }
}

fn inspect_channel_losses(config: &Config, issues: &mut Vec<MigrationIssue>) {
    let mut used = BTreeSet::new();
    used.extend(config.roles.sensor_ch.iter().copied());
    used.extend(config.roles.signal_ch.iter().copied());
    used.insert(config.roles.reference_ch);

    let unused = config
        .channels
        .iter()
        .filter(|channel| !used.contains(&channel.index))
        .map(|channel| format!("ch{}", channel.index))
        .collect::<Vec<_>>();
    if !unused.is_empty() {
        issues.push(MigrationIssue::lossy(format!(
            "unused channel definitions are not representable in v4 and will be removed: {}",
            unused.join(", ")
        )));
    }

    let metadata = config
        .channels
        .iter()
        .filter(|channel| {
            !config.roles.sensor_ch.contains(&channel.index)
                && (channel.factor.is_some()
                    || channel.scale_to_abs_max.is_some()
                    || channel.label.is_some()
                    || channel.unit_out.is_some())
        })
        .map(|channel| format!("ch{}", channel.index))
        .collect::<Vec<_>>();
    if !metadata.is_empty() {
        issues.push(MigrationIssue::lossy(format!(
            "metadata on non-sensor channels is not representable in v4 and will be removed: {}",
            metadata.join(", ")
        )));
    }
}

fn inspect_artifact_base_change(
    source_path: &Path,
    destination_path: &Path,
    issues: &mut Vec<MigrationIssue>,
) -> Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let source_parent = absolute_parent(source_path, &cwd);
    let destination_parent = absolute_parent(destination_path, &cwd);
    if destination_parent != cwd {
        issues.push(MigrationIssue::lossy(format!(
            "v4 resolves data artifacts from the config directory ({}), while legacy configs resolve them from the process directory ({})",
            destination_parent.display(),
            cwd.display()
        )));
    } else {
        issues.push(MigrationIssue::notice(
            "v4 resolves data artifacts from the config directory instead of the process current directory",
        ));
    }
    if source_parent != destination_parent {
        issues.push(MigrationIssue::lossy(format!(
            "the upgraded config is being relocated from {} to {}; relative plot and artifact paths will use the new directory",
            source_parent.display(),
            destination_parent.display()
        )));
    }
    Ok(())
}

fn absolute_parent(path: &Path, cwd: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let parent = absolute
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(cwd)
        .to_path_buf();
    fs::canonicalize(&parent).unwrap_or(parent)
}

fn verify_preserved_semantics(mut source: Config, target: Config) -> Result<()> {
    canonicalize_for_v4(&mut source);
    let source = toml::Value::try_from(&source).context("failed to compare source semantics")?;
    let target = toml::Value::try_from(&target).context("failed to compare target semantics")?;
    if source != target {
        bail!(
            "generated v4 config does not preserve the normalized source semantics; migration was blocked"
        );
    }
    Ok(())
}

fn canonicalize_for_v4(config: &mut Config) {
    config.version = LATEST_CONFIG_VERSION;
    config.legacy_timebase = None;

    let mut used = BTreeSet::new();
    used.extend(config.roles.sensor_ch.iter().copied());
    used.extend(config.roles.signal_ch.iter().copied());
    used.insert(config.roles.reference_ch);
    config
        .channels
        .retain(|channel| used.contains(&channel.index));
    for channel in &mut config.channels {
        if !config.roles.sensor_ch.contains(&channel.index) {
            channel.factor = None;
            channel.scale_to_abs_max = None;
            channel.label = None;
            channel.unit_out = None;
        }
    }

    let (enabled, save, interactive) = match (
        config.plot.enabled,
        config.plot.save,
        config.plot.interactive,
    ) {
        (false, _, _) | (true, false, false) => (false, false, false),
        (true, true, true) => (true, true, true),
        (true, false, true) => (true, false, true),
        (true, true, false) => (true, true, false),
    };
    config.plot.enabled = enabled;
    config.plot.save = save;
    config.plot.interactive = interactive;
}

#[cfg(test)]
mod tests;
