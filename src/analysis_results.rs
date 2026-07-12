use crate::config::Config;
use crate::constants::T_HEADER;
use crate::lockin::sensor::extract_sensor_metadata;
use crate::utils::csv::write_csv;
use anyhow::{Result, bail};
use std::path::Path;

#[derive(Debug, PartialEq)]
pub struct AnalysisResultData {
    pub time: Vec<f64>,
    pub sensor_rate: Vec<Vec<f64>>,
    pub sensor_integral: Vec<Vec<f64>>,
    pub results: Vec<Vec<Vec<f64>>>,
}

pub fn parse_analysis_result_files(
    files: &[Vec<Vec<f64>>],
    sensor_count: usize,
    result_column_count: usize,
    label: &str,
) -> Result<AnalysisResultData> {
    let context_column_count = 1 + sensor_count * 2;
    let expected_column_count = context_column_count + result_column_count;
    validate_column_count(files, expected_column_count, label)?;

    let Some(first) = files.first() else {
        bail!("no {label} files were loaded");
    };
    validate_context_columns(files, context_column_count, label)?;

    Ok(AnalysisResultData {
        time: first[0].clone(),
        sensor_rate: first[1..(1 + sensor_count)].to_vec(),
        sensor_integral: first[(1 + sensor_count)..context_column_count].to_vec(),
        results: files
            .iter()
            .map(|file| file[context_column_count..expected_column_count].to_vec())
            .collect(),
    })
}

pub fn build_analysis_headers<I, S>(cfg: &Config, result_headers: I) -> Result<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let sensor_metadata = extract_sensor_metadata(cfg)?;
    let mut headers = Vec::new();
    headers.push(T_HEADER.to_string());
    headers.extend(sensor_metadata.iter().map(|metadata| {
        let label = metadata.label.replace('$', "");
        format!("{} rate ({}/s)", label, metadata.unit)
    }));
    headers.extend(sensor_metadata.iter().map(|metadata| {
        let label = metadata.label.replace('$', "");
        format!("{} integral ({})", label, metadata.unit)
    }));
    headers.extend(result_headers.into_iter().map(Into::into));
    Ok(headers)
}

pub fn write_analysis_results<P: AsRef<Path>>(
    path: P,
    headers: &[String],
    time: &[f64],
    sensor_rate: &[Vec<f64>],
    sensor_integral: &[Vec<f64>],
    results: &[Vec<f64>],
    save_npy: bool,
) -> Result<()> {
    let columns = std::iter::once(time)
        .chain(sensor_rate.iter().map(Vec::as_slice))
        .chain(sensor_integral.iter().map(Vec::as_slice))
        .chain(results.iter().map(Vec::as_slice))
        .collect::<Vec<_>>();
    let header_refs = headers.iter().map(String::as_str).collect::<Vec<_>>();
    let path_ref = path.as_ref();
    if path_ref.exists() {
        bail!("analysis output already exists: {}", path_ref.display());
    }
    let npy_path = path_ref.with_extension("npy");
    if save_npy && npy_path.exists() {
        bail!("analysis output already exists: {}", npy_path.display());
    }
    write_csv(path_ref, &header_refs, &columns)?;

    if save_npy && let Err(error) = crate::utils::csv::write_npy(&npy_path, &columns) {
        let _ = std::fs::remove_file(path_ref);
        return Err(error);
    }

    Ok(())
}

fn validate_column_count(
    files: &[Vec<Vec<f64>>],
    expected_column_count: usize,
    label: &str,
) -> Result<()> {
    for (index, file) in files.iter().enumerate() {
        if file.len() != expected_column_count {
            bail!(
                "{label} file {index} has {} columns, expected {expected_column_count}; old CSV layouts are not supported",
                file.len()
            );
        }
    }
    Ok(())
}

fn validate_context_columns(
    files: &[Vec<Vec<f64>>],
    context_column_count: usize,
    label: &str,
) -> Result<()> {
    let Some(first) = files.first() else {
        return Ok(());
    };
    for (file_index, file) in files.iter().enumerate().skip(1) {
        for column_index in 0..context_column_count {
            if file[column_index] != first[column_index] {
                bail!(
                    "{label} file {file_index} context column {column_index} differs from file 0"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::LI_HEADER;
    use crate::test_support::test_config;

    #[test]
    fn parses_time_sensor_context_and_result_columns() {
        let first = columns(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let second = columns(&[0.0, 1.0, 2.0, 3.0, 4.0, 6.0]);

        let parsed = parse_analysis_result_files(&[first, second], 2, 1, "results").unwrap();

        assert_eq!(parsed.time, vec![0.0]);
        assert_eq!(parsed.sensor_rate, vec![vec![1.0], vec![2.0]]);
        assert_eq!(parsed.sensor_integral, vec![vec![3.0], vec![4.0]]);
        assert_eq!(parsed.results, vec![vec![vec![5.0]], vec![vec![6.0]]]);
    }

    #[test]
    fn rejects_empty_old_or_context_mismatched_results() {
        let empty = parse_analysis_result_files(&[], 1, 1, "lock-in results").unwrap_err();
        assert!(empty.to_string().contains("no lock-in results files"));

        let old_layout = vec![vec![vec![0.0]; 1 + 2 + 12]];
        let old = parse_analysis_result_files(&old_layout, 2, 12, "lock-in results").unwrap_err();
        assert!(
            old.to_string()
                .contains("old CSV layouts are not supported")
        );

        let first = columns(&[0.0, 1.0, 2.0, 3.0]);
        let second = columns(&[0.0, 1.5, 2.0, 4.0]);
        let mismatch =
            parse_analysis_result_files(&[first, second], 1, 1, "lock-in results").unwrap_err();
        assert!(mismatch.to_string().contains("context column 1"));
    }

    #[test]
    fn builds_headers_in_time_rate_integral_result_order() {
        let cfg = test_config(vec![1, 2], vec![3]);
        let headers = build_analysis_headers(&cfg, LI_HEADER).unwrap();

        let mut expected = vec![
            "time (s)".to_string(),
            "ch1 rate (T/s)".to_string(),
            "ch2 rate (T/s)".to_string(),
            "ch1 integral (T)".to_string(),
            "ch2 integral (T)".to_string(),
        ];
        expected.extend(LI_HEADER.iter().map(|header| header.to_string()));
        assert_eq!(headers, expected);
    }

    fn columns(values: &[f64]) -> Vec<Vec<f64>> {
        values.iter().map(|&value| vec![value]).collect()
    }
}
