use crate::{
    config::Config,
    constants::{LI_ROTATED_HEADER, T_HEADER},
    lockin::sensor::extract_sensor_metadata,
    utils::csv::write_csv,
};
use anyhow::Result;

pub fn get_li_rotated_headers(cfg: &Config) -> Result<Vec<String>> {
    let t_header = T_HEADER.to_string();

    let sensor_meta = extract_sensor_metadata(cfg)?;

    let sensor_headers: Vec<String> = sensor_meta
        .iter()
        .map(|m| {
            let label = m.label.replace("$", "");
            format!("{} ({})", label, m.unit)
        })
        .collect();

    let li_headers: Vec<String> = LI_ROTATED_HEADER.iter().map(|s| s.to_string()).collect();

    let mut headers = Vec::new();
    headers.push(t_header);
    headers.extend(sensor_headers);
    headers.extend(li_headers);
    Ok(headers)
}

pub fn write_li_rotated_results(
    fname: &str,
    headers: &[String],
    t: &[f64],
    s_integral: &[Vec<f64>],
    li_rotated_result: &[Vec<f64>],
) -> Result<()> {
    let mut export_data: Vec<Vec<f64>> =
        Vec::with_capacity(1 + s_integral.len() + li_rotated_result.len());

    export_data.push(t.to_vec());
    export_data.extend_from_slice(s_integral);
    export_data.extend_from_slice(li_rotated_result);

    let headers_slice: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    write_csv(fname, &headers_slice, &export_data)?;

    Ok(())
}
