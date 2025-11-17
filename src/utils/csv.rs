use anyhow::{bail, Context, Result};
use csv::{ReaderBuilder, StringRecord};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

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

pub fn write_csv(
    path: &str,
    headers: &[&str],
    columns: &[Vec<f64>], // column-major: columns[i][row]
) -> Result<()> {
    let ncols = columns.len();
    if ncols == 0 {
        File::create(path)?;
        return Ok(());
    }

    let nrows = columns[0].len();
    for (i, col) in columns.iter().enumerate() {
        if col.len() != nrows {
            anyhow::bail!("column {i} has length {}, expected {nrows}", col.len());
        }
    }

    let file = File::create(path).context("failed to create csv file")?;
    let mut w = BufWriter::new(file);

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
        for (col_idx, col) in columns.iter().enumerate() {
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

#[cfg(feature = "hw")]
pub fn ensure_not_exists(path: &str) -> Result<()> {
    if Path::new(path).exists() {
        bail!("file {} already exists", path);
    }
    Ok(())
}
