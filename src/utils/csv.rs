use anyhow::{Context, Result};
use csv::{ReaderBuilder, StringRecord};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

const CSV_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const CSV_READ_BUFFER_BYTES: usize = 8 * 1024 * 1024;

pub fn read_csv<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<f64>>> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .buffer_capacity(CSV_READ_BUFFER_BYTES)
        .from_path(&path)
        .with_context(|| format!("failed to open csv: {}", path.as_ref().display()))?;

    let mut columns: Vec<Vec<f64>> = Vec::new();

    for result in rdr.records() {
        let record: StringRecord = result?;
        if columns.is_empty() {
            columns.resize(record.len(), Vec::new());
        }
        for (col_idx, field) in record.iter().enumerate() {
            let val: f64 = field
                .parse()
                .with_context(|| format!("failed to parse '{}' as f64", field))?;
            columns[col_idx].push(val);
        }
    }

    Ok(columns)
}

pub fn read_selected_columns<P: AsRef<Path>>(path: P, cols: &[usize]) -> Result<Vec<Vec<f64>>> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .buffer_capacity(CSV_READ_BUFFER_BYTES)
        .from_path(&path)
        .with_context(|| format!("failed to open csv: {}", path.as_ref().display()))?;

    let mut columns: Vec<Vec<f64>> = cols.iter().map(|_| Vec::new()).collect();

    for result in rdr.records() {
        let record: StringRecord = result?;
        for (out_idx, &col_idx) in cols.iter().enumerate() {
            let field = record
                .get(col_idx)
                .ok_or_else(|| anyhow::anyhow!("column {} out of range", col_idx))?;
            let val: f64 = field
                .parse()
                .with_context(|| format!("failed to parse '{}' as f64", field))?;
            columns[out_idx].push(val);
        }
    }

    Ok(columns)
}

pub fn write_csv<P, C>(
    path: P,
    headers: &[&str],
    columns: &[C], // column-major: columns[i][row]
) -> Result<()>
where
    P: AsRef<Path>,
    C: AsRef<[f64]>,
{
    let path_ref = path.as_ref();
    let ncols = columns.len();
    if !headers.is_empty() && headers.len() != ncols {
        anyhow::bail!(
            "header len ({}) and column len ({}) mismatch",
            headers.len(),
            ncols
        );
    }

    let column_refs: Vec<&[f64]> = columns.iter().map(|col| col.as_ref()).collect();
    let nrows = column_refs[0].len();
    for (i, col) in column_refs.iter().enumerate() {
        if col.len() != nrows {
            anyhow::bail!("column {i} has length {}, expected {nrows}", col.len());
        }
    }

    let mut encoded_header = Vec::new();
    if !headers.is_empty() {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(&mut encoded_header);
        writer.write_record(headers.iter().copied())?;
        writer.flush()?;
    }

    if let Some(parent) = path_ref.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).context("failed to create directory for csv output")?;
    }

    if ncols == 0 {
        File::create(path_ref)?;
        return Ok(());
    }

    let file = File::create(path.as_ref()).context("failed to create csv file")?;
    // Large waveform CSVs contain hundreds of millions of rows. Keep the
    // formatting unchanged while avoiding frequent small writes to the OS.
    let mut w = BufWriter::with_capacity(CSV_WRITE_BUFFER_BYTES, file);

    w.write_all(&encoded_header)?;

    for row in 0..nrows {
        for (col_idx, col) in column_refs.iter().enumerate() {
            write!(w, "{}", col[row])?;
            if col_idx + 1 != ncols {
                write!(w, ",")?;
            }
        }
        writeln!(w)?;
    }

    w.flush()?;
    Ok(())
}

pub fn write_npy<P, C>(path: P, columns: &[C]) -> Result<()>
where
    P: AsRef<Path>,
    C: AsRef<[f64]>,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).context("failed to create directory for npy output")?;
    }

    let ncols = columns.len();
    if ncols == 0 {
        anyhow::bail!("cannot write an NPY table without columns");
    }

    let column_refs: Vec<&[f64]> = columns.iter().map(|col| col.as_ref()).collect();
    let nrows = column_refs[0].len();
    for (i, col) in column_refs.iter().enumerate() {
        if col.len() != nrows {
            anyhow::bail!("column {i} has length {}, expected {nrows}", col.len());
        }
    }

    let dictionary =
        format!("{{'descr': '<f8', 'fortran_order': False, 'shape': ({nrows}, {ncols}), }}");
    let prefix_len = 10usize;
    let padding = (64 - ((prefix_len + dictionary.len() + 1) % 64)) % 64;
    let header = format!("{dictionary}{}\n", " ".repeat(padding));
    let header_len =
        u16::try_from(header.len()).map_err(|_| anyhow::anyhow!("NPY header is too large"))?;

    let file = File::create(path)
        .with_context(|| format!("failed to create NPY file: {}", path.display()))?;
    let mut writer = BufWriter::new(file);

    writer.write_all(b"\x93NUMPY")?;
    writer.write_all(&[1, 0])?;
    writer.write_all(&header_len.to_le_bytes())?;
    writer.write_all(header.as_bytes())?;
    for row in 0..nrows {
        for column in &column_refs {
            writer.write_all(&column[row].to_le_bytes())?;
        }
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_csv;
    use csv::ReaderBuilder;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pmoke-csv-{label}-{}-{nonce}.csv",
            std::process::id()
        ))
    }

    #[test]
    fn write_csv_escapes_special_headers() {
        let path = unique_test_path("special-headers");
        let headers = ["comma,header", "quote\"header", "line\nbreak"];

        write_csv(&path, &headers, &[&[1.0], &[2.0], &[3.0]]).unwrap();

        let mut reader = ReaderBuilder::new().from_path(&path).unwrap();
        let actual = reader
            .headers()
            .unwrap()
            .iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(actual, headers);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn write_csv_header_mismatch_preserves_existing_file() {
        let path = unique_test_path("header-mismatch");
        let original = b"existing contents\n";
        fs::write(&path, original).unwrap();

        let error = write_csv(&path, &["one", "two"], &[&[1.0]]).unwrap_err();

        assert!(error.to_string().contains("header len"));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(path).unwrap();
    }
}
