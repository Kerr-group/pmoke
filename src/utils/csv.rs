use anyhow::{Context, Result};
use csv::{ReaderBuilder, StringRecord};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

const CSV_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

pub fn read_csv<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<f64>>> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
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
    let ncols = columns.len();
    if ncols == 0 {
        File::create(path.as_ref())?;
        return Ok(());
    }

    let column_refs: Vec<&[f64]> = columns.iter().map(|col| col.as_ref()).collect();
    let nrows = column_refs[0].len();
    for (i, col) in column_refs.iter().enumerate() {
        if col.len() != nrows {
            anyhow::bail!("column {i} has length {}, expected {nrows}", col.len());
        }
    }

    let file = File::create(path.as_ref()).context("failed to create csv file")?;
    // Large waveform CSVs contain hundreds of millions of rows. Keep the
    // formatting unchanged while avoiding frequent small writes to the OS.
    let mut w = BufWriter::with_capacity(CSV_WRITE_BUFFER_BYTES, file);

    if !headers.is_empty() {
        if headers.len() != ncols {
            anyhow::bail!(
                "header len ({}) and column len ({}) mismatch",
                headers.len(),
                ncols
            );
        }
        for (i, h) in headers.iter().enumerate() {
            if i + 1 == ncols {
                write!(w, "{h}")?;
            } else {
                write!(w, "{h},")?;
            }
        }
        writeln!(w)?;
    }

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
