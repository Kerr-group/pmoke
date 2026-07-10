use crate::{
    config::Config,
    constants::{KERR_HEADER, T_HEADER},
    lockin::sensor::extract_sensor_metadata,
    utils::csv::write_csv,
};
use anyhow::Result;
use std::path::Path;

pub fn get_kerr_headers(cfg: &Config) -> Result<Vec<String>> {
    let t_header = T_HEADER.to_string();

    let sensor_meta = extract_sensor_metadata(cfg)?;

    let sensor_rate_headers: Vec<String> = sensor_meta
        .iter()
        .map(|m| {
            let label = m.label.replace("$", "");
            format!("{} rate ({}/s)", label, m.unit)
        })
        .collect();
    let sensor_integral_headers: Vec<String> = sensor_meta
        .iter()
        .map(|m| {
            let label = m.label.replace("$", "");
            format!("{} integral ({})", label, m.unit)
        })
        .collect();

    let use_signal_ch = cfg.phase_signal_ch();
    let kerr_headers: Vec<String> = use_signal_ch
        .iter()
        .map(|ch| format!("Ch{} {}", ch, KERR_HEADER))
        .collect();

    let mut headers = Vec::new();
    headers.push(t_header);
    headers.extend(sensor_rate_headers);
    headers.extend(sensor_integral_headers);
    headers.extend(kerr_headers);
    Ok(headers)
}

pub fn write_kerr_results<P: AsRef<Path>>(
    fname: P,
    headers: &[String],
    t: &[f64],
    s_rate: &[Vec<f64>],
    s_integral: &[Vec<f64>],
    kerr_results: &[Vec<f64>],
) -> Result<()> {
    let mut export_data: Vec<&[f64]> =
        Vec::with_capacity(1 + s_rate.len() + s_integral.len() + kerr_results.len());

    export_data.push(t);
    export_data.extend(s_rate.iter().map(|col| col.as_slice()));
    export_data.extend(s_integral.iter().map(|col| col.as_slice()));
    export_data.extend(kerr_results.iter().map(|col| col.as_slice()));

    let headers_slice: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    write_csv(fname, &headers_slice, &export_data)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::get_kerr_headers;
    use crate::test_support::test_config;

    #[test]
    fn kerr_headers_use_rate_integral_result_order() {
        let cfg = test_config(vec![1, 2], vec![3, 4]);

        let headers = get_kerr_headers(&cfg).unwrap();

        assert_eq!(
            headers,
            vec![
                "time (s)".to_string(),
                "ch1 rate (T/s)".to_string(),
                "ch2 rate (T/s)".to_string(),
                "ch1 integral (T)".to_string(),
                "ch2 integral (T)".to_string(),
                "Ch3 Kerr angle (rad)".to_string(),
                "Ch4 Kerr angle (rad)".to_string(),
            ]
        );
    }
}
