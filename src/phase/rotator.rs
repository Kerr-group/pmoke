pub fn rotate_phase(lix: &[f64], liy: &[f64], delta: f64) -> (Vec<f64>, Vec<f64>) {
    let cos_d = delta.cos();
    let sin_d = delta.sin();

    let li_in_out = lix.iter().zip(liy.iter()).map(|(x, y)| {
        // x and y are &f64
        let li_in = x * cos_d + y * sin_d;
        let li_out = -x * sin_d + y * cos_d;
        (li_in, li_out)
    });

    let (li_in, li_out): (Vec<f64>, Vec<f64>) = li_in_out.unzip();

    (li_in, li_out)
}
