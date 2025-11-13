use crate::{
    config::Config,
    constants::{KERR_HEADER, LI_ROTATED_HEADER, T_HEADER},
    lockin::sensor::extract_sensor_metadata,
    utils::csv::write_csv,
};
use anyhow::Result;

pub fn get_kerr_headers(cfg: &Config) -> Result<Vec<String>> {
    let t_header = T_HEADER.to_string();

    let sensor_meta = extract_sensor_metadata(cfg)?;

    let sensor_headers: Vec<String> = sensor_meta
        .iter()
        .map(|m| {
            let label = m.label.replace("$", "");
            format!("{} ({})", label, m.unit)
        })
        .collect();

    let use_signal_ch = &cfg.phase.use_signal_ch;
    let kerr_headers: Vec<String> = use_signal_ch
        .iter()
        .map(|ch| format!("Ch{} {}", ch, KERR_HEADER))
        .collect();

    let mut headers = Vec::new();
    headers.push(t_header);
    headers.extend(sensor_headers);
    headers.extend(kerr_headers);
    Ok(headers)
}

pub fn write_kerr_results(
    fname: &str,
    headers: &[String],
    t: &[f64],
    s_integral: &[f64],
    kerr_results: &[Vec<f64>],
) -> Result<()> {
    let mut export_data: Vec<Vec<f64>> =
        Vec::with_capacity(1 + s_integral.len() + kerr_results.len());

    export_data.push(t.to_vec());
    export_data.push(s_integral.to_vec());
    export_data.extend_from_slice(kerr_results);

    let headers_slice: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    write_csv(fname, &headers_slice, &export_data)?;

    Ok(())
}
