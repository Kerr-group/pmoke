use crate::analysis_results::{build_analysis_headers, write_analysis_results};
use crate::config::Config;
use crate::constants::LI_HEADER;
use anyhow::Result;
use std::path::Path;

pub fn get_li_headers(cfg: &Config) -> Result<Vec<String>> {
    build_analysis_headers(cfg, LI_HEADER)
}

pub fn write_li_results<P: AsRef<Path>>(
    fname: P,
    headers: &[String],
    t: &[f64],
    s_rate: &[Vec<f64>],
    s_integral: &[Vec<f64>],
    li_result: &[Vec<f64>],
) -> Result<()> {
    write_analysis_results(fname, headers, t, s_rate, s_integral, li_result)
}

#[cfg(test)]
mod tests {
    use super::get_li_headers;
    use crate::{constants::LI_HEADER, test_support::test_config};

    #[test]
    fn lockin_headers_use_rate_integral_result_order() {
        let cfg = test_config(vec![1, 2], vec![3]);

        let headers = get_li_headers(&cfg).unwrap();

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
}
