use crate::config::Config;
use crate::constants::{KERR_NAME, LI_RESULTS_NAME, LI_ROTATED_NAME};
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct ExportMetadata {
    schema_version: u32,
    pmoke_version: &'static str,
    arrays: Vec<ArrayMetadata>,
}

#[derive(Serialize)]
struct ArrayMetadata {
    file: String,
    source_csv: String,
    rows: usize,
    columns: usize,
    column_names: Vec<String>,
    dtype: &'static str,
    order: &'static str,
}

#[derive(Debug)]
struct CsvTable {
    headers: Vec<String>,
    columns: Vec<Vec<f64>>,
    rows: usize,
}

pub fn export(cfg: &Config, output: &Path) -> Result<()> {
    if !cfg.force {
        ensure_missing(output, "NPY output directory")?;
    } else if output.exists() {
        ensure_regular_directory(output, "NPY output")?;
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create NPY output parent: {}", parent.display()))?;
    }
    let staging = staging_path(output);
    ensure_missing(&staging, "NPY staging directory")?;
    fs::create_dir(&staging).with_context(|| {
        format!(
            "failed to create NPY staging directory: {}",
            staging.display()
        )
    })?;

    let result = export_into(cfg, &staging).and_then(|metadata| {
        write_metadata(&staging, &metadata)?;
        sync_directory(&staging)?;
        crate::commands::run_dir::publish_staged_directory(&staging, output, cfg.force)?;
        Ok(metadata)
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let metadata = result?;
    ui::settings_table(
        "NumPy export",
        vec![
            ("output".to_string(), output.display().to_string()),
            ("arrays".to_string(), metadata.arrays.len().to_string()),
        ],
    );
    ui::success("analysis NumPy export completed");
    Ok(())
}

pub fn export_canonical(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "export_npy")?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    let result = export_canonical_locked(cfg);
    match &result {
        Ok(()) => crate::commands::run_dir::write_run_state(
            cfg,
            "published",
            "export_npy_complete",
            None,
        )?,
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "export_npy", Some(error))?
        }
    }
    result
}

fn export_canonical_locked(cfg: &Config) -> Result<()> {
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::ExportNpy,
    )?;
    crate::commands::run_dir::ensure_analysis_config_snapshots(&staging_cfg)?;
    export_canonical_inner(&staging_cfg)?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)?;
    ui::success("analysis NumPy export completed");
    Ok(())
}

fn export_canonical_inner(cfg: &Config) -> Result<()> {
    let paths = cfg.paths();
    crate::commands::run_dir::verify_analysis_diagnostic_snapshots(cfg, None)?;
    let manifest_path = paths.analysis_manifest();
    let manifest: toml::Value = toml::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let checksums = manifest_csv_checksums(&manifest)?;
    let pairs = canonical_csv_npy_pairs(&paths.analysis_dir(), &checksums)?;
    verify_canonical_csv_checksums(&paths.analysis_dir(), &pairs, &checksums)?;

    for (_, destination) in &pairs {
        if destination.exists() {
            ensure_regular_file(destination, "NPY output")?;
        }
    }
    for (source, destination) in &pairs {
        let table = read_csv_table(source)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        write_npy_table_replacing(destination, &table.columns, table.rows, true)?;
    }
    crate::lockin::provenance::refresh_analysis_manifest_outputs(cfg, "export_npy")?;
    Ok(())
}

fn verify_canonical_csv_checksums(
    analysis_dir: &Path,
    pairs: &[(PathBuf, PathBuf)],
    checksums: &BTreeMap<String, String>,
) -> Result<()> {
    for (source, _) in pairs {
        let relative = source
            .strip_prefix(analysis_dir)
            .context("canonical analysis CSV is outside the analysis directory")?
            .to_string_lossy()
            .replace('\\', "/");
        let expected = checksums
            .get(&relative)
            .ok_or_else(|| anyhow!("analysis CSV checksum plan is missing {relative}"))?;
        let actual = crate::utils::checksum::file_sha256(source)
            .with_context(|| format!("failed to checksum analysis CSV: {}", source.display()))?;
        if actual.as_str() != expected {
            bail!(
                "analysis CSV checksum mismatch for {relative}: expected {expected}, got {actual}; rerun pmoke analyze or the owning analysis stage"
            );
        }
    }
    Ok(())
}

fn manifest_csv_checksums(manifest: &toml::Value) -> Result<BTreeMap<String, String>> {
    let outputs = manifest
        .get("outputs")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            anyhow!(
                "analysis manifest has no output checksums; rerun pmoke analyze or pmoke li before exporting NPY"
            )
        })?;
    let mut checksums = BTreeMap::new();
    for output in outputs {
        let Some(file) = output.get("file").and_then(toml::Value::as_str) else {
            continue;
        };
        if !looks_like_canonical_analysis_csv(file) {
            continue;
        }
        validate_canonical_analysis_csv_path(file)?;
        let checksum = output
            .get("sha256")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow!("analysis CSV checksum is missing for {file}"))?;
        if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("analysis CSV checksum is malformed for {file}");
        }
        if checksums
            .insert(file.to_string(), checksum.to_string())
            .is_some()
        {
            bail!("analysis manifest contains duplicate output entries for {file}");
        }
    }

    if let Some(artifacts) = manifest.get("artifacts") {
        let artifacts = artifacts
            .as_array()
            .ok_or_else(|| anyhow!("analysis manifest artifacts must be an array"))?;
        let mut artifact_csvs = BTreeSet::new();
        for artifact in artifacts {
            let Some(csv) = artifact.get("csv") else {
                continue;
            };
            let csv = csv
                .as_str()
                .ok_or_else(|| anyhow!("analysis artifact csv must be a string"))?;
            validate_canonical_analysis_csv_path(csv)?;
            if !artifact_csvs.insert(csv.to_string()) {
                bail!("analysis manifest contains duplicate CSV artifacts for {csv}");
            }
        }
        let output_csvs = checksums.keys().cloned().collect::<BTreeSet<_>>();
        if artifact_csvs != output_csvs {
            bail!("analysis manifest CSV artifacts and output checksums do not match");
        }
    }
    if checksums.is_empty() {
        bail!(
            "analysis manifest contains no canonical CSV artifacts; run pmoke analyze or pmoke li first"
        );
    }
    Ok(checksums)
}

fn looks_like_canonical_analysis_csv(file: &str) -> bool {
    (file.starts_with("lockin/") || file.starts_with("kerr/")) && file.ends_with(".csv")
}

fn validate_canonical_analysis_csv_path(file: &str) -> Result<()> {
    if file.contains('\\') {
        bail!("analysis manifest paths must use forward slashes: {file}");
    }
    let path = Path::new(file);
    let components = path.components().collect::<Vec<_>>();
    let valid_root = matches!(
        components.first(),
        Some(std::path::Component::Normal(root)) if *root == "lockin" || *root == "kerr"
    );
    if path.is_absolute()
        || components.len() != 2
        || !valid_root
        || path.extension().and_then(|value| value.to_str()) != Some("csv")
    {
        bail!("invalid canonical analysis CSV path in manifest: {file}");
    }
    Ok(())
}

fn canonical_csv_npy_pairs(
    analysis_dir: &Path,
    checksums: &BTreeMap<String, String>,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut actual_csvs = BTreeSet::new();
    let mut existing_npys = BTreeSet::new();
    for directory in [analysis_dir.join("lockin"), analysis_dir.join("kerr")] {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect canonical analysis CSV directory: {}",
                        directory.display()
                    )
                });
            }
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let extension = path.extension().and_then(|extension| extension.to_str());
            if !matches!(extension, Some("csv" | "npy")) {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() {
                bail!(
                    "canonical analysis artifact is not a regular file: {}",
                    path.display()
                );
            }
            let relative = path
                .strip_prefix(analysis_dir)
                .context("canonical analysis artifact is outside the analysis directory")?
                .to_string_lossy()
                .replace('\\', "/");
            if extension == Some("csv") {
                actual_csvs.insert(relative);
            } else {
                existing_npys.insert(relative);
            }
        }
    }
    let expected_csvs = checksums.keys().cloned().collect::<BTreeSet<_>>();
    if let Some(missing) = expected_csvs.difference(&actual_csvs).next() {
        bail!("canonical analysis CSV recorded in the manifest is missing: {missing}");
    }
    if let Some(extra) = actual_csvs.difference(&expected_csvs).next() {
        bail!("canonical analysis contains an unrecorded CSV: {extra}");
    }
    for npy in existing_npys {
        let csv = Path::new(&npy).with_extension("csv");
        let csv = csv.to_string_lossy().replace('\\', "/");
        if !expected_csvs.contains(&csv) {
            bail!("canonical analysis contains an orphan NPY without a recorded CSV: {npy}");
        }
    }
    Ok(expected_csvs
        .into_iter()
        .map(|relative| {
            let source = analysis_dir.join(relative);
            let destination = source.with_extension("npy");
            (source, destination)
        })
        .collect())
}

fn write_npy_table_replacing(
    destination: &Path,
    columns: &[Vec<f64>],
    rows: usize,
    force: bool,
) -> Result<()> {
    if !force || !destination.exists() {
        return write_npy_table(destination, columns, rows);
    }
    let temporary = crate::commands::run_dir::unique_temporary_path(destination)?;
    let result = (|| {
        write_npy_table(&temporary, columns, rows)?;
        crate::commands::run_dir::replace_file_atomically(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn ensure_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label}: {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("{label} is not a regular file: {}", path.display());
    }
    Ok(())
}

fn ensure_regular_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label}: {}", path.display()))?;
    if !metadata.file_type().is_dir() {
        bail!("{label} is not a regular directory: {}", path.display());
    }
    Ok(())
}

fn export_into(cfg: &Config, staging: &Path) -> Result<ExportMetadata> {
    let resolver = cfg.resolver();
    let mut sources = Vec::new();
    for &channel in cfg.phase_signal_ch() {
        sources.push((
            resolver.lockin_xy_csv(channel),
            format!("{LI_RESULTS_NAME}_ch{channel}.csv"),
        ));
        sources.push((
            resolver.lockin_rotated_csv(channel),
            format!("{LI_ROTATED_NAME}_ch{channel}.csv"),
        ));
    }
    sources.push((resolver.kerr_csv(), format!("{KERR_NAME}_results.csv")));

    let mut arrays = Vec::with_capacity(sources.len());
    for (source, source_name) in sources {
        let table = read_csv_table(&source)?;
        let output_name = source_name.strip_suffix(".csv").map_or_else(
            || format!("{source_name}.npy"),
            |stem| format!("{stem}.npy"),
        );
        write_npy_table(&staging.join(&output_name), &table.columns, table.rows)?;
        arrays.push(ArrayMetadata {
            file: output_name,
            source_csv: source_name,
            rows: table.rows,
            columns: table.columns.len(),
            column_names: table.headers,
            dtype: "<f8",
            order: "C",
        });
    }
    Ok(ExportMetadata {
        schema_version: 1,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        arrays,
    })
}

fn read_csv_table(path: &Path) -> Result<CsvTable> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .with_context(|| format!("failed to open analysis CSV: {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed to read CSV headers: {}", path.display()))?
        .iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if headers.is_empty() {
        bail!("analysis CSV has no columns: {}", path.display());
    }
    let mut columns = vec![Vec::new(); headers.len()];
    let mut rows = 0usize;
    for record in reader.records() {
        let record = record.with_context(|| format!("failed to read CSV: {}", path.display()))?;
        if record.len() != headers.len() {
            bail!(
                "analysis CSV row {rows} has {} columns, expected {}: {}",
                record.len(),
                headers.len(),
                path.display()
            );
        }
        for (column, field) in columns.iter_mut().zip(record.iter()) {
            column.push(field.parse::<f64>().with_context(|| {
                format!("failed to parse {field:?} in {} row {rows}", path.display())
            })?);
        }
        rows += 1;
    }
    if rows == 0 {
        bail!("analysis CSV has no data rows: {}", path.display());
    }
    Ok(CsvTable {
        headers,
        columns,
        rows,
    })
}

fn write_npy_table(path: &Path, columns: &[Vec<f64>], rows: usize) -> Result<()> {
    if columns.is_empty() {
        bail!("cannot write an NPY table without columns");
    }
    for (index, column) in columns.iter().enumerate() {
        if column.len() != rows {
            bail!(
                "NPY column {index} has {} rows, expected {rows}",
                column.len()
            );
        }
    }
    let dictionary = format!(
        "{{'descr': '<f8', 'fortran_order': False, 'shape': ({rows}, {}), }}",
        columns.len()
    );
    let prefix_len = 10usize;
    let padding = (64 - ((prefix_len + dictionary.len() + 1) % 64)) % 64;
    let header = format!("{dictionary}{}\n", " ".repeat(padding));
    let header_len = u16::try_from(header.len()).map_err(|_| anyhow!("NPY header is too large"))?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create NPY file: {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"\x93NUMPY")?;
    writer.write_all(&[1, 0])?;
    writer.write_all(&header_len.to_le_bytes())?;
    writer.write_all(header.as_bytes())?;
    for row in 0..rows {
        for column in columns {
            writer.write_all(&column[row].to_le_bytes())?;
        }
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn write_metadata(staging: &Path, metadata: &ExportMetadata) -> Result<()> {
    let encoded = toml::to_string_pretty(metadata).context("failed to encode NPY metadata")?;
    let path = staging.join("metadata.toml");
    let mut file = File::create(&path)
        .with_context(|| format!("failed to create NPY metadata: {}", path.display()))?;
    file.write_all(encoded.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn ensure_missing(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("{label} already exists: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn staging_path(output: &Path) -> PathBuf {
    let parent = output.parent().unwrap_or_else(|| Path::new(""));
    let mut name = OsString::from(".");
    name.push(output.file_name().unwrap_or_default());
    name.push(".tmp");
    parent.join(name)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_file() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pmoke-npy-{}-{nonce}.npy", std::process::id()))
    }

    #[test]
    fn npy_table_is_c_order_little_endian_f64() {
        let path = temporary_file();
        write_npy_table(&path, &[vec![1.0, 2.0], vec![3.0, 4.0]], 2).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[..6], b"\x93NUMPY");
        let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        let header = std::str::from_utf8(&bytes[10..10 + header_len]).unwrap();
        assert!(header.contains("'shape': (2, 2)"));
        let payload = &bytes[10 + header_len..];
        let values = payload
            .chunks_exact(8)
            .map(|chunk| f64::from_le_bytes(chunk.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(values, vec![1.0, 3.0, 2.0, 4.0]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn csv_table_rejects_header_only_analysis_output() {
        let path = temporary_file().with_extension("csv");
        fs::write(&path, "time (s),Kerr angle (rad)\n").unwrap();

        let error = read_csv_table(&path).unwrap_err();

        assert!(error.to_string().contains("no data rows"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn canonical_export_places_npy_beside_each_analysis_csv() {
        let root = temporary_file().with_extension("run");
        fs::create_dir(&root).unwrap();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(root.clone());
        let paths = cfg.paths();
        for csv in [
            paths.lockin_xy_csv(2),
            paths.lockin_xy_csv(3),
            paths.lockin_rotated_csv(2),
            paths.kerr_csv(),
        ] {
            fs::create_dir_all(csv.parent().unwrap()).unwrap();
            fs::write(csv, "time (s),value\n0,1\n1,2\n").unwrap();
        }
        crate::commands::run_dir::write_analysis_config_snapshots(&cfg).unwrap();
        fs::write(
            paths.analysis_manifest(),
            "schema_version = 1\noutputs = []\n",
        )
        .unwrap();
        crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "export_npy").unwrap();

        export_canonical(&cfg).unwrap();

        assert!(paths.lockin_xy_npy(2).is_file());
        assert!(paths.lockin_xy_npy(3).is_file());
        assert!(paths.lockin_rotated_npy(2).is_file());
        assert!(paths.kerr_npy().is_file());
        let manifest = fs::read_to_string(paths.analysis_manifest()).unwrap();
        assert!(manifest.contains("lockin/ch2_xy.npy"));
        assert!(manifest.contains("kerr/kerr.npy"));
        let analysis_source = fs::read(paths.analysis_source_config()).unwrap();

        let missing_csv = paths.lockin_xy_csv(3);
        let missing_csv_contents = fs::read(&missing_csv).unwrap();
        fs::remove_file(&missing_csv).unwrap();
        let error = export_canonical(&cfg).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("recorded in the manifest is missing")
        );
        fs::write(&missing_csv, missing_csv_contents).unwrap();

        let extra_csv = paths.lockin_xy_csv(9);
        fs::write(&extra_csv, "time (s),value\n0,1\n").unwrap();
        let error = export_canonical(&cfg).unwrap_err();
        assert!(error.to_string().contains("unrecorded CSV"));
        fs::remove_file(extra_csv).unwrap();

        let orphan_npy = paths.lockin_xy_npy(9);
        fs::write(&orphan_npy, b"orphan").unwrap();
        let error = export_canonical(&cfg).unwrap_err();
        assert!(error.to_string().contains("orphan NPY"));
        fs::remove_file(orphan_npy).unwrap();

        cfg.source_text = Some("version = 3\n# export-only change\n".to_string());
        export_canonical(&cfg).unwrap();
        assert_eq!(
            fs::read(paths.analysis_source_config()).unwrap(),
            analysis_source
        );
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(paths.analysis_manifest()).unwrap()).unwrap();
        let run: toml::Value =
            toml::from_str(&fs::read_to_string(paths.run_manifest()).unwrap()).unwrap();
        assert_eq!(
            run["analysis"]["published_generation"].as_integer(),
            manifest["generation"].as_integer()
        );
        assert_eq!(
            run["analysis"]["last_attempt"]["command"].as_str(),
            Some("export_npy")
        );
        assert_eq!(run["status"].as_str(), Some("complete"));

        fs::write(paths.analysis_source_config(), b"tampered\n").unwrap();
        let error = export_canonical(&cfg).unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"));
        fs::write(paths.analysis_source_config(), &analysis_source).unwrap();

        cfg.force = true;
        let npy_before = fs::read(paths.lockin_xy_npy(2)).unwrap();
        let manifest_before = fs::read(paths.analysis_manifest()).unwrap();
        fs::write(paths.lockin_xy_csv(2), "time (s),value\n0,3\n1,4\n").unwrap();
        let error = export_canonical(&cfg).unwrap_err();
        assert!(error.to_string().contains("CSV checksum mismatch"));
        assert_eq!(fs::read(paths.lockin_xy_npy(2)).unwrap(), npy_before);
        assert_eq!(
            fs::read(paths.analysis_manifest()).unwrap(),
            manifest_before
        );
        fs::remove_dir_all(root).unwrap();
    }
}
