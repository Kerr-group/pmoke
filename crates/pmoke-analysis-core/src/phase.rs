pub fn rotate_phase(lix: &[f64], liy: &[f64], delta: f64) -> (Vec<f64>, Vec<f64>) {
    let cos_delta = delta.cos();
    let sin_delta = delta.sin();
    lix.iter()
        .zip(liy)
        .map(|(&x, &y)| {
            (
                x * cos_delta + y * sin_delta,
                -x * sin_delta + y * cos_delta,
            )
        })
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_preserves_magnitude() {
        let (rotated_x, rotated_y) = rotate_phase(&[3.0, -1.0], &[4.0, 2.0], 0.73);
        for ((x, y), expected) in rotated_x
            .iter()
            .zip(&rotated_y)
            .zip([5.0_f64, 5.0_f64.sqrt()])
        {
            assert!((x.hypot(*y) - expected).abs() < 1.0e-12);
        }
    }
}
