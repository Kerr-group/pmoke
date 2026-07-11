use crate::config::Config;
use crate::constants::{KERR_NAME, LI_RESULTS_NAME, LI_ROTATED_NAME};
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
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
    ensure_missing(output, "NPY output directory")?;
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
        fs::rename(&staging, output).with_context(|| {
            format!(
                "failed to publish NPY directory {} as {}",
                staging.display(),
                output.display()
            )
        })?;
        sync_parent(output)?;
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

fn export_into(cfg: &Config, staging: &Path) -> Result<ExportMetadata> {
    let mut sources = cfg
        .phase_signal_ch()
        .iter()
        .flat_map(|channel| {
            [
                format!("{LI_RESULTS_NAME}_ch{channel}.csv"),
                format!("{LI_ROTATED_NAME}_ch{channel}.csv"),
            ]
        })
        .collect::<Vec<_>>();
    sources.push(format!("{KERR_NAME}_results.csv"));
    let mut arrays = Vec::with_capacity(sources.len());
    for source_name in sources {
        let source = cfg.artifact_path(&source_name);
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

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<()> {
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
}
