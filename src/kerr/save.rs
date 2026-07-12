use crate::analysis_results::{build_analysis_headers, write_analysis_results};
use crate::config::Config;
use crate::constants::KERR_HEADER;
use anyhow::Result;
use std::path::Path;

pub fn get_kerr_headers(cfg: &Config) -> Result<Vec<String>> {
    let use_signal_ch = cfg.phase_signal_ch();
    let kerr_headers = use_signal_ch
        .iter()
        .map(|ch| format!("Ch{} {}", ch, KERR_HEADER))
        .collect::<Vec<_>>();
    build_analysis_headers(cfg, kerr_headers)
}

pub fn write_kerr_results<P: AsRef<Path>>(
    fname: P,
    headers: &[String],
    t: &[f64],
    s_rate: &[Vec<f64>],
    s_integral: &[Vec<f64>],
    kerr_results: &[Vec<f64>],
    save_npy: bool,
) -> Result<()> {
    write_analysis_results(fname, headers, t, s_rate, s_integral, kerr_results, save_npy)
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
