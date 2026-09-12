use crate::analysis_results::{build_analysis_headers, write_analysis_results};
use crate::config::Config;
use crate::constants::{ANGLE_HEADER, VM_HEADER};
use anyhow::Result;
use std::path::Path;

pub fn get_moke_headers(cfg: &Config) -> Result<Vec<String>> {
    let use_signal_ch = cfg.phase_signal_ch();
    let angle_headers = use_signal_ch
        .iter()
        .map(|ch| format!("Ch{} {}", ch, ANGLE_HEADER))
        .collect::<Vec<_>>();
    let vm_headers = use_signal_ch
        .iter()
        .map(|ch| format!("Ch{} {}", ch, VM_HEADER))
        .collect::<Vec<_>>();
    build_analysis_headers(cfg, angle_headers.into_iter().chain(vm_headers))
}

#[allow(clippy::too_many_arguments)]
pub fn write_moke_results<P: AsRef<Path>>(
    fname: P,
    headers: &[String],
    t: &[f64],
    s_rate: &[Vec<f64>],
    s_integral: &[Vec<f64>],
    angle_results: &[Vec<f64>],
    vm_results: &[Vec<f64>],
    save_npy: bool,
) -> Result<()> {
    write_analysis_results(
        fname,
        headers,
        t,
        s_rate,
        s_integral,
        &angle_results
            .iter()
            .chain(vm_results.iter())
            .map(Vec::as_slice)
            .collect::<Vec<_>>(),
        save_npy,
    )
}

#[cfg(test)]
mod tests {
    use super::get_moke_headers;
    use crate::test_support::test_config;

    #[test]
    fn moke_headers_use_rate_integral_angle_vm_order() {
        let cfg = test_config(vec![1, 2], vec![3, 4]);

        let headers = get_moke_headers(&cfg).unwrap();

        assert_eq!(
            headers,
            vec![
                "time (s)".to_string(),
                "ch1 rate (T/s)".to_string(),
                "ch2 rate (T/s)".to_string(),
                "ch1 integral (T)".to_string(),
                "ch2 integral (T)".to_string(),
                "Ch3 angle (rad)".to_string(),
                "Ch4 angle (rad)".to_string(),
                "Ch3 Vm (V)".to_string(),
                "Ch4 Vm (V)".to_string(),
            ]
        );
    }
}
