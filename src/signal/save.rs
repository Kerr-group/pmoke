use crate::analysis_results::{build_analysis_headers, write_analysis_results};
use crate::config::Config;
use anyhow::Result;
use std::path::Path;

pub fn get_signal_headers(cfg: &Config) -> Result<Vec<String>> {
    let mean_headers = cfg
        .signals
        .iter()
        .map(|signal| {
            format!(
                "Ch{} {} mean ({})",
                signal.channel,
                signal.label.replace('$', ""),
                signal.unit
            )
        })
        .collect::<Vec<_>>();
    build_analysis_headers(cfg, mean_headers)
}

#[allow(clippy::too_many_arguments)]
pub fn write_signal_results<P: AsRef<Path>>(
    fname: P,
    headers: &[String],
    t: &[f64],
    s_rate: &[Vec<f64>],
    s_integral: &[Vec<f64>],
    means: &[Vec<f64>],
    save_npy: bool,
) -> Result<()> {
    write_analysis_results(
        fname,
        headers,
        t,
        s_rate,
        s_integral,
        &means.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        save_npy,
    )
}

#[cfg(test)]
mod tests {
    use super::get_signal_headers;
    use crate::config::Signal;
    use crate::test_support::test_config;

    #[test]
    fn signal_headers_use_rate_integral_mean_order() {
        let mut cfg = test_config(vec![1, 2], vec![3, 4]);
        cfg.signals = vec![
            Signal {
                channel: 5,
                label: "DC".to_string(),
                unit: "V".to_string(),
            },
            Signal {
                channel: 6,
                label: "$\\mu_0H$".to_string(),
                unit: "T".to_string(),
            },
        ];

        let headers = get_signal_headers(&cfg).unwrap();

        assert_eq!(
            headers,
            vec![
                "time (s)".to_string(),
                "ch1 rate (T/s)".to_string(),
                "ch2 rate (T/s)".to_string(),
                "ch1 integral (T)".to_string(),
                "ch2 integral (T)".to_string(),
                "Ch5 DC mean (V)".to_string(),
                "Ch6 \\mu_0H mean (T)".to_string(),
            ]
        );
    }
}
