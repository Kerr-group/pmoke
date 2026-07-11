use crate::analysis_results::{build_analysis_headers, write_analysis_results};
use crate::config::Config;
use crate::constants::LI_ROTATED_HEADER;
use anyhow::Result;
use std::path::Path;

pub fn get_li_rotated_headers(cfg: &Config) -> Result<Vec<String>> {
    build_analysis_headers(cfg, LI_ROTATED_HEADER)
}

pub fn write_li_rotated_results<P: AsRef<Path>>(
    fname: P,
    headers: &[String],
    t: &[f64],
    s_rate: &[Vec<f64>],
    s_integral: &[Vec<f64>],
    li_rotated_result: &[Vec<f64>],
) -> Result<()> {
    write_analysis_results(fname, headers, t, s_rate, s_integral, li_rotated_result)
}

#[cfg(test)]
mod tests {
    use super::get_li_rotated_headers;
    use crate::{constants::LI_ROTATED_HEADER, test_support::test_config};

    #[test]
    fn rotated_headers_use_rate_integral_result_order() {
        let cfg = test_config(vec![1, 2], vec![3]);

        let headers = get_li_rotated_headers(&cfg).unwrap();

        let mut expected = vec![
            "time (s)".to_string(),
            "ch1 rate (T/s)".to_string(),
            "ch2 rate (T/s)".to_string(),
            "ch1 integral (T)".to_string(),
            "ch2 integral (T)".to_string(),
        ];
        expected.extend(LI_ROTATED_HEADER.iter().map(|header| header.to_string()));
        assert_eq!(headers, expected);
    }
}
