use anyhow::{Result, bail};

pub struct PulseBgAverage {}

impl PulseBgAverage {
    pub fn calculate<I>(&self, values: I) -> Result<f64>
    where
        I: IntoIterator<Item = f64>,
    {
        let mut sum = 0.0;
        let mut compensation = 0.0;
        let mut count = 0usize;

        for value in values {
            if !value.is_finite() {
                bail!("background window contains a non-finite value");
            }

            let next = sum + value;
            if sum.abs() >= value.abs() {
                compensation += (sum - next) + value;
            } else {
                compensation += (value - next) + sum;
            }
            sum = next;
            count += 1;
        }

        if count == 0 {
            bail!("cannot calculate a background average from an empty window");
        }

        let average = (sum + compensation) / count as f64;
        if !average.is_finite() {
            bail!("background average is not finite");
        }
        Ok(average)
    }
}

#[derive(Debug, Clone)]
pub struct PulseIntegralCalculator {
    dt: f64,
}

impl PulseIntegralCalculator {
    pub fn new(dt: f64) -> Self {
        Self { dt }
    }

    pub fn integrate(&self, data: &[f64], c_bg: f64, coeff: f64) -> Vec<f64> {
        let n = data.len();
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![(data[0] - c_bg) * coeff];
        }

        let h = self.dt;
        let mut out = Vec::with_capacity(n);
        out.push(0.0);

        let mut acc = 0.0;
        for i in 1..n {
            let s0 = data[i - 1] - c_bg;
            let s1 = data[i] - c_bg;
            let incr = h * (s0 + s1) * 0.5;
            acc += incr;
            out.push(acc);
        }

        if coeff != 1.0 {
            for v in &mut out {
                *v *= coeff;
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::{PulseBgAverage, PulseIntegralCalculator};

    #[test]
    fn background_average_is_the_constant_least_squares_solution() {
        let average = PulseBgAverage {}.calculate([1.0, 2.0, 6.0, 11.0]).unwrap();
        assert_eq!(average, 5.0);
    }

    #[test]
    fn background_average_preserves_small_terms_during_cancellation() {
        let average = PulseBgAverage {}.calculate([1.0e16, 1.0, -1.0e16]).unwrap();
        assert_eq!(average, 1.0 / 3.0);
    }

    #[test]
    fn background_average_rejects_empty_and_non_finite_inputs() {
        assert!(PulseBgAverage {}.calculate([]).is_err());
        assert!(PulseBgAverage {}.calculate([1.0, f64::NAN]).is_err());
        assert!(PulseBgAverage {}.calculate([f64::INFINITY]).is_err());
    }

    #[test]
    fn pulse_integral_preserves_sign_after_factor_application() {
        let integral = PulseIntegralCalculator::new(1.0).integrate(&[1.0, 1.0, 1.0], 0.0, -2.0);

        assert_eq!(integral, vec![0.0, -2.0, -4.0]);
    }
}
