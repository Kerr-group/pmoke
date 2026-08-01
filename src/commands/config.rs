use crate::cli::ConfigCommand;
use crate::commands::show;
use crate::config::{
    CONFIG_FIELD_DOCS, ConfigFieldDoc, ConfigLoad, MigrationPlan, load_from_path, load_from_str,
    plan_latest_executable_migration, plan_migration,
};
use crate::ui;
use anyhow::{Context, Result, bail, ensure};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigCommandOutcome {
    pub exit_code: u8,
}

pub fn run(config_path: &str, command: &ConfigCommand) -> Result<ConfigCommandOutcome> {
    match command {
        ConfigCommand::Init { output, force } => {
            run_init(Path::new(config_path), output.as_deref(), *force)
        }
        ConfigCommand::Validate => run_validate(Path::new(config_path)),
        ConfigCommand::Explain { path } => run_explain(path.as_deref()),
        ConfigCommand::Migrate {
            output,
            in_place,
            check,
            accept_lossy,
            to,
        } => run_migrate(
            Path::new(config_path),
            output.as_deref(),
            *in_place,
            *check,
            *accept_lossy,
            *to,
        ),
    }
}

const CONFIG_TEMPLATE_V4: &str = r#"version = 4

[scope]
model = "DHO5108"
connection = "tcp://192.168.10.100:55255"

[generator]
model = "WF1946B"
connection = "gpib://0/11"
# macOS Prologix alternatives:
# connection = "prologix-serial:///dev/cu.usbserial-XXXX?addr=11"
# connection = "prologix-tcp://192.168.1.50:1234?addr=11"

[data]
output = "raw"       # "csv", "raw", or "both"
input = "raw"        # "csv", "raw", or "auto"
screenshot = true

[[sensors]]
channel = 1
scale = { max_abs = 55.0, polarity = -1 }
# A TOML literal string passes one backslash to Matplotlib mathtext.
label = '$\mu_0H$'
unit = "T"

[[sensors]]
channel = 4
scale = { factor = 1.0 }
label = "sensor"
unit = "a.u."

[pulse]
background_before = { start = -5e-3, end = -0.1e-3 }
background_after  = { start = 43e-3, end = 46e-3 }

[reference]
channel = 2
fft_window = { start = 0e-3, end = 15e-3 }
stride_samples = 10_000
window_samples = 1_000

[lockin]
signal_channels = [3]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0, 0, 0, 0, 0, 0]

[kerr]
sensor = 1
method = "harmonics" # "standard" or "harmonics"
factor = -1.0

[plot]
mode = "both" # "off", "save", "interactive", or "both"
decimation = "min_max" # "none", "stride", or "min_max"
"#;

fn run_init(source: &Path, output: Option<&Path>, force: bool) -> Result<ConfigCommandOutcome> {
    let destination = output.unwrap_or(source);
    let stdout_output = destination == Path::new("-");

    ensure_template_is_valid()?;
    if stdout_output {
        print!("{CONFIG_TEMPLATE_V4}");
        io::stdout()
            .flush()
            .context("failed to flush config template to stdout")?;
    } else if force {
        replace_output(destination, CONFIG_TEMPLATE_V4.as_bytes())?;
        ui::saved(format!("initialized config at {}", destination.display()));
    } else {
        write_new_output(destination, CONFIG_TEMPLATE_V4.as_bytes())?;
        ui::saved(format!("initialized config at {}", destination.display()));
    }

    Ok(ConfigCommandOutcome { exit_code: 0 })
}

fn run_validate(source: &Path) -> Result<ConfigCommandOutcome> {
    match load_from_path(source) {
        ConfigLoad::Ready { warnings, .. } => {
            show::print_warnings(&warnings);
            if warnings.is_empty() {
                ui::success(format!("config is valid: {}", source.display()));
            } else {
                ui::success(format!(
                    "config is valid with {} warning(s): {}",
                    warnings.len(),
                    source.display()
                ));
            }
            Ok(ConfigCommandOutcome { exit_code: 0 })
        }
        ConfigLoad::Diagnostics(diagnostics) => {
            show::print_diagnostics(&diagnostics);
            Ok(ConfigCommandOutcome { exit_code: 1 })
        }
    }
}

fn run_explain(path: Option<&str>) -> Result<ConfigCommandOutcome> {
    match path.map(str::trim).filter(|value| !value.is_empty()) {
        None => print_field_docs("Config Fields", CONFIG_FIELD_DOCS),
        Some(path) => {
            let matches = explain_matches(path);
            ensure!(
                !matches.is_empty(),
                "unknown config field or section: {path}"
            );
            let title = if matches.len() == 1 && matches[0].path == path {
                format!("Config Field: {path}")
            } else {
                format!("Config Fields Matching: {path}")
            };
            print_field_docs(&title, &matches);
        }
    }
    Ok(ConfigCommandOutcome { exit_code: 0 })
}

fn explain_matches(path: &str) -> Vec<ConfigFieldDoc> {
    let exact = CONFIG_FIELD_DOCS
        .iter()
        .copied()
        .filter(|doc| config_doc_path_matches(doc.path, path))
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return exact;
    }
    CONFIG_FIELD_DOCS
        .iter()
        .copied()
        .filter(|doc| {
            let searchable = doc.path.replace("[]", "");
            searchable.starts_with(&format!("{path}.")) || searchable.contains(path)
        })
        .collect()
}

fn config_doc_path_matches(documented: &str, query: &str) -> bool {
    documented == query || documented.replace("[]", "") == query
}

fn print_field_docs(title: &str, docs: &[ConfigFieldDoc]) {
    ui::settings_table(
        title,
        docs.iter()
            .map(|doc| {
                (
                    doc.path.to_string(),
                    format!("{} {}", doc.summary_en, doc.details_en),
                )
            })
            .collect(),
    );
}

fn ensure_template_is_valid() -> Result<()> {
    match load_from_str(CONFIG_TEMPLATE_V4) {
        ConfigLoad::Ready { .. } => Ok(()),
        ConfigLoad::Diagnostics(diagnostics) => bail!(
            "bundled config template is invalid: {} diagnostic(s)",
            diagnostics.diagnostics.len()
        ),
    }
}

fn run_migrate(
    source: &Path,
    output: Option<&Path>,
    in_place: bool,
    check: bool,
    accept_lossy: bool,
    target_version: Option<u32>,
) -> Result<ConfigCommandOutcome> {
    let stdout_output = output == Some(Path::new("-"));
    let destination = match (in_place, output) {
        (true, _) => Some(source),
        (false, Some(path)) if path != Path::new("-") => Some(path),
        _ => Some(source),
    };
    let plan = match target_version {
        Some(target) => plan_migration(source, destination, target),
        None => plan_latest_executable_migration(source, destination),
    }
    .context("config migration blocked")?;

    if stdout_output {
        eprint!("{}", migration_report(&plan));
    } else {
        print!("{}", migration_report(&plan));
    }

    if check {
        let exit_code = check_exit_code(plan.changed, plan.has_lossy_changes(), accept_lossy);
        return Ok(ConfigCommandOutcome { exit_code });
    }

    if !plan.changed {
        return Ok(ConfigCommandOutcome { exit_code: 0 });
    }

    if output.is_none() && !in_place {
        println!("{}", migration_diff(&plan));
        if plan.has_lossy_changes() && !accept_lossy {
            println!(
                "Preview only: use --accept-lossy with --output or --in-place to accept the reported behavior changes."
            );
        }
        return Ok(ConfigCommandOutcome { exit_code: 0 });
    }

    require_lossy_acceptance(plan.has_lossy_changes(), accept_lossy)?;

    if stdout_output {
        print!("{}", plan.target_toml);
        io::stdout()
            .flush()
            .context("failed to flush migrated config to stdout")?;
    } else if in_place {
        replace_in_place(&plan)?;
        ui::saved(format!("migrated {} in place", source.display()));
    } else if let Some(path) = output {
        write_new_output(path, plan.target_toml.as_bytes())?;
        ui::saved(format!("migrated config to {}", path.display()));
    }

    Ok(ConfigCommandOutcome { exit_code: 0 })
}

fn check_exit_code(changed: bool, has_lossy_changes: bool, accept_lossy: bool) -> u8 {
    if !changed {
        0
    } else if has_lossy_changes && !accept_lossy {
        2
    } else {
        1
    }
}

fn require_lossy_acceptance(has_lossy_changes: bool, accept_lossy: bool) -> Result<()> {
    if has_lossy_changes && !accept_lossy {
        bail!(
            "migration contains behavior-changing steps; review the report and rerun with --accept-lossy"
        );
    }
    Ok(())
}

fn migration_report(plan: &MigrationPlan) -> String {
    let mut report = format!(
        "Config migration: v{} -> v{}\nDestination: {}\nStatus: {}\n",
        plan.source_version,
        plan.target_version,
        plan.destination_path.display(),
        plan.compatibility_label()
    );
    if !plan.changed {
        for issue in &plan.issues {
            report.push_str(&format!("[{}] {}\n", issue.level.label(), issue.message));
        }
        if plan.limited {
            report.push_str(
                "No higher config version can preserve the currently executable analysis path; no files were changed.\n",
            );
        } else {
            report.push_str(
                "The config is already at the requested version; no files were changed.\n",
            );
        }
        return report;
    }
    for issue in &plan.issues {
        report.push_str(&format!("[{}] {}\n", issue.level.label(), issue.message));
    }
    report
}

fn migration_diff(plan: &MigrationPlan) -> String {
    let source = String::from_utf8_lossy(&plan.original);
    let before = source.lines().collect::<Vec<_>>();
    let after = plan.target_toml.lines().collect::<Vec<_>>();
    let operations = line_diff(&before, &after);
    let mut output = format!(
        "--- {} (v{})\n+++ preview (v{})\n",
        plan.source_path.display(),
        plan.source_version,
        plan.target_version
    );
    for (prefix, line) in operations {
        output.push(prefix);
        output.push(' ');
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn line_diff<'a>(before: &[&'a str], after: &[&'a str]) -> Vec<(char, &'a str)> {
    const MAX_LCS_CELLS: usize = 1_000_000;
    if before.len().saturating_mul(after.len()) > MAX_LCS_CELLS {
        return before
            .iter()
            .map(|line| ('-', *line))
            .chain(after.iter().map(|line| ('+', *line)))
            .collect();
    }

    let width = after.len() + 1;
    let mut lengths = vec![0usize; (before.len() + 1) * width];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            lengths[i * width + j] = if before[i] == after[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }

    let mut operations = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            operations.push((' ', before[i]));
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            operations.push(('-', before[i]));
            i += 1;
        } else {
            operations.push(('+', after[j]));
            j += 1;
        }
    }
    operations.extend(before[i..].iter().map(|line| ('-', *line)));
    operations.extend(after[j..].iter().map(|line| ('+', *line)));
    operations
}

fn write_new_output(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("refusing to overwrite output: {}", path.display()))?;
    if let Err(error) = write_and_sync(&mut file, contents) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error).with_context(|| format!("failed to write output: {}", path.display()));
    }
    Ok(())
}

fn replace_output(path: &Path, contents: &[u8]) -> Result<()> {
    let permissions = regular_file_metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let (temporary, mut temporary_file) = create_temporary(path)?;
    let prepare_result = (|| -> Result<()> {
        write_and_sync(&mut temporary_file, contents)?;
        if let Some(permissions) = permissions {
            fs::set_permissions(&temporary, permissions).with_context(|| {
                format!(
                    "failed to preserve output permissions: {}",
                    temporary.display()
                )
            })?;
            temporary_file
                .sync_all()
                .with_context(|| format!("failed to sync permissions: {}", temporary.display()))?;
        }
        Ok(())
    })();
    drop(temporary_file);
    if let Err(error) = prepare_result {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to write output: {}", path.display()));
    }

    if let Err(error) = atomic_replace(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| {
            format!(
                "failed to atomically replace {} with {}",
                path.display(),
                temporary.display()
            )
        });
    }
    sync_parent_directory(path)?;
    Ok(())
}

fn replace_in_place(plan: &MigrationPlan) -> Result<()> {
    let source = &plan.source_path;
    let metadata = regular_file_metadata(source)?;
    ensure_source_unchanged(source, &plan.original)?;

    let backup = backup_path(source, plan.source_version);
    create_backup(&backup, &plan.original, &metadata.permissions())?;

    let (temporary, mut temporary_file) = match create_temporary(source) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(&backup);
            return Err(error);
        }
    };
    let prepare_result = (|| -> Result<()> {
        write_and_sync(&mut temporary_file, plan.target_toml.as_bytes())?;
        fs::set_permissions(&temporary, metadata.permissions()).with_context(|| {
            format!(
                "failed to preserve config permissions: {}",
                temporary.display()
            )
        })?;
        temporary_file
            .sync_all()
            .with_context(|| format!("failed to sync permissions: {}", temporary.display()))?;
        regular_file_metadata(source)?;
        ensure_source_unchanged(source, &plan.original)?;
        Ok(())
    })();
    drop(temporary_file);
    if let Err(error) = prepare_result {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(&backup);
        return Err(error);
    }

    if let Err(error) = atomic_replace(&temporary, source) {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(&backup);
        return Err(error).with_context(|| {
            format!(
                "failed to atomically replace {} with {}",
                source.display(),
                temporary.display()
            )
        });
    }
    sync_parent_directory(source)?;
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn regular_file_metadata(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect config: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("refusing to replace a symlink config: {}", path.display());
    }
    if !metadata.file_type().is_file() {
        bail!("config is not a regular file: {}", path.display());
    }
    Ok(metadata)
}

fn create_backup(path: &Path, contents: &[u8], permissions: &fs::Permissions) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("refusing to overwrite backup: {}", path.display()))?;
    let result = (|| -> Result<()> {
        write_and_sync(&mut file, contents)?;
        fs::set_permissions(path, permissions.clone()).with_context(|| {
            format!("failed to preserve backup permissions: {}", path.display())
        })?;
        file.sync_all()
            .with_context(|| format!("failed to sync backup: {}", path.display()))?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error).context("failed to create config backup");
    }
    Ok(())
}

fn ensure_source_unchanged(path: &Path, expected: &[u8]) -> Result<()> {
    let current = fs::read(path)
        .with_context(|| format!("failed to re-read source config: {}", path.display()))?;
    if current != expected {
        bail!(
            "source config changed while the migration was being prepared; no replacement was performed"
        );
    }
    Ok(())
}

fn backup_path(source: &Path, version: u32) -> PathBuf {
    let mut value = source.as_os_str().to_os_string();
    value.push(format!(".v{version}.bak"));
    PathBuf::from(value)
}

fn create_temporary(source: &Path) -> Result<(PathBuf, File)> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let filename = source
        .file_name()
        .unwrap_or_else(|| OsStr::new("config.toml"));
    for attempt in 0..100u32 {
        let mut name = OsStr::new(".").to_os_string();
        name.push(filename);
        name.push(format!(".migrate.{}.{attempt}.tmp", std::process::id()));
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create temporary file: {}", path.display())
                });
            }
        }
    }
    bail!("failed to allocate a unique temporary config file")
}

fn write_and_sync(file: &mut File, contents: &[u8]) -> Result<()> {
    file.write_all(contents)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("failed to sync config directory: {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pmoke_config_command_{}_{}_{}",
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

    fn replacement_plan(source: &Path, before: &[u8], after: &str) -> MigrationPlan {
        MigrationPlan {
            source_version: 3,
            target_version: 4,
            source_path: source.to_path_buf(),
            destination_path: source.to_path_buf(),
            target_toml: after.to_string(),
            issues: Vec::new(),
            changed: true,
            limited: false,
            original: before.to_vec(),
        }
    }

    #[test]
    fn line_diff_reconstructs_both_inputs() {
        let before = ["a", "b", "c"];
        let after = ["a", "x", "c"];
        let diff = line_diff(&before, &after);
        let reconstructed_before = diff
            .iter()
            .filter(|(kind, _)| *kind != '+')
            .map(|(_, line)| *line)
            .collect::<Vec<_>>();
        let reconstructed_after = diff
            .iter()
            .filter(|(kind, _)| *kind != '-')
            .map(|(_, line)| *line)
            .collect::<Vec<_>>();
        assert_eq!(reconstructed_before, before);
        assert_eq!(reconstructed_after, after);
    }

    #[test]
    fn check_exit_codes_distinguish_latest_migration_and_lossy_block() {
        assert_eq!(check_exit_code(false, false, false), 0);
        assert_eq!(check_exit_code(true, false, false), 1);
        assert_eq!(check_exit_code(true, true, false), 2);
        assert_eq!(check_exit_code(true, true, true), 1);
    }

    #[test]
    fn lossy_output_requires_explicit_acceptance() {
        assert!(require_lossy_acceptance(true, false).is_err());
        assert!(require_lossy_acceptance(true, true).is_ok());
        assert!(require_lossy_acceptance(false, false).is_ok());
    }

    #[test]
    fn in_place_migration_creates_versioned_backup() {
        let dir = TempDir::new();
        let source = dir.0.join("config.toml");
        let before = b"version = 3\n";
        let after = "version = 4\n";
        fs::write(&source, before).unwrap();

        replace_in_place(&replacement_plan(&source, before, after)).unwrap();

        assert_eq!(fs::read_to_string(&source).unwrap(), after);
        assert_eq!(
            fs::read(backup_path(&source, 3)).unwrap(),
            before.as_slice()
        );
    }

    #[test]
    fn existing_backup_blocks_in_place_migration_without_modifying_source() {
        let dir = TempDir::new();
        let source = dir.0.join("config.toml");
        let before = b"version = 3\n";
        fs::write(&source, before).unwrap();
        fs::write(backup_path(&source, 3), b"existing backup").unwrap();

        let error =
            replace_in_place(&replacement_plan(&source, before, "version = 4\n")).unwrap_err();

        assert!(error.to_string().contains("refusing to overwrite backup"));
        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(
            fs::read(backup_path(&source, 3)).unwrap(),
            b"existing backup"
        );
    }

    #[test]
    fn init_writes_a_valid_v4_template() {
        let dir = TempDir::new();
        let source = dir.0.join("config.toml");

        let outcome = run_init(&source, None, false).unwrap();

        assert_eq!(outcome.exit_code, 0);
        let text = fs::read_to_string(&source).unwrap();
        assert!(text.contains("version = 4"));
        assert!(matches!(load_from_str(&text), ConfigLoad::Ready { .. }));
    }

    #[test]
    fn init_template_matches_readme_example_config() {
        let readme = include_str!("../../README.md").replace("\r\n", "\n");
        let marker = "```toml\nversion = 4\n";
        let start = readme.find(marker).unwrap() + "```toml\n".len();
        let rest = &readme[start..];
        let end = rest.find("\n```").unwrap();

        assert_eq!(CONFIG_TEMPLATE_V4.trim(), rest[..end].trim());
    }

    #[test]
    fn init_refuses_to_overwrite_without_force() {
        let dir = TempDir::new();
        let source = dir.0.join("config.toml");
        fs::write(&source, "existing").unwrap();

        let error = run_init(&source, None, false).unwrap_err();

        assert!(error.to_string().contains("refusing to overwrite output"));
        assert_eq!(fs::read_to_string(&source).unwrap(), "existing");
    }

    #[test]
    fn init_force_overwrites_existing_config() {
        let dir = TempDir::new();
        let source = dir.0.join("config.toml");
        fs::write(&source, "existing").unwrap();

        run_init(&source, None, true).unwrap();

        let text = fs::read_to_string(&source).unwrap();
        assert!(text.contains(r#"filter = { kind = "boxcar_legacy""#));
        assert!(matches!(load_from_str(&text), ConfigLoad::Ready { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn init_force_preserves_regular_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let source = dir.0.join("config.toml");
        fs::write(&source, "existing").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();

        run_init(&source, None, true).unwrap();

        let mode = fs::metadata(&source).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn init_force_replaces_symlink_without_truncating_target() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new();
        let target = dir.0.join("target.toml");
        let source = dir.0.join("config.toml");
        fs::write(&target, "target contents").unwrap();
        symlink(&target, &source).unwrap();

        run_init(&source, None, true).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "target contents");
        let text = fs::read_to_string(&source).unwrap();
        assert!(text.contains("version = 4"));
        assert!(
            !fs::symlink_metadata(&source)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn validate_returns_zero_for_valid_config_and_one_for_invalid_config() {
        let dir = TempDir::new();
        let valid = dir.0.join("valid.toml");
        fs::write(&valid, CONFIG_TEMPLATE_V4).unwrap();
        let invalid = dir.0.join("invalid.toml");
        fs::write(&invalid, "version = 4\nunknown = true\n").unwrap();

        assert_eq!(run_validate(&valid).unwrap().exit_code, 0);
        assert_eq!(run_validate(&invalid).unwrap().exit_code, 1);
    }

    #[test]
    fn explain_accepts_sections_and_rejects_unknown_paths() {
        assert_eq!(run_explain(Some("lockin.filter")).unwrap().exit_code, 0);
        assert_eq!(run_explain(Some("sensors.channel")).unwrap().exit_code, 0);
        assert!(run_explain(Some("does.not.exist")).is_err());
    }

    #[test]
    fn changed_source_blocks_in_place_migration_without_creating_backup() {
        let dir = TempDir::new();
        let source = dir.0.join("config.toml");
        let planned = b"version = 3\n";
        let changed = b"version = 3\n# edited concurrently\n";
        fs::write(&source, changed).unwrap();

        let error =
            replace_in_place(&replacement_plan(&source, planned, "version = 4\n")).unwrap_err();

        assert!(error.to_string().contains("changed"));
        assert_eq!(fs::read(&source).unwrap(), changed);
        assert!(!backup_path(&source, 3).exists());
    }

    #[test]
    fn output_writer_refuses_to_overwrite_existing_file() {
        let dir = TempDir::new();
        let output = dir.0.join("config.v4.toml");
        fs::write(&output, b"keep me").unwrap();

        assert!(write_new_output(&output, b"replacement").is_err());
        assert_eq!(fs::read(&output).unwrap(), b"keep me");
    }

    #[cfg(unix)]
    #[test]
    fn in_place_migration_refuses_symlink_source() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new();
        let real = dir.0.join("real.toml");
        let source = dir.0.join("config.toml");
        let before = b"version = 3\n";
        fs::write(&real, before).unwrap();
        symlink(&real, &source).unwrap();

        let error =
            replace_in_place(&replacement_plan(&source, before, "version = 4\n")).unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert_eq!(fs::read(&real).unwrap(), before);
        assert!(!backup_path(&source, 3).exists());
    }
}
