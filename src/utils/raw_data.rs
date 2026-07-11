#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawTimeAxis {
    pub sample_count: usize,
    pub x_increment: f64,
    pub x_origin: f64,
    pub x_reference: f64,
}

impl RawTimeAxis {
    pub fn value_at(self, index: usize) -> f64 {
        self.x_origin + (index as f64 - self.x_reference) * self.x_increment
    }

    pub fn build(self) -> Vec<f64> {
        (0..self.sample_count)
            .map(|index| self.value_at(index))
            .collect()
    }

    pub fn validate_geometry(self) -> Result<(), TimeAxisError> {
        if self.sample_count == 0 {
            return Err(TimeAxisError::Empty);
        }
        if self.x_increment <= 0.0 {
            return Err(TimeAxisError::NonPositiveIncrement(self.x_increment));
        }
        for index in [0, self.sample_count - 1] {
            let value = self.value_at(index);
            if !value.is_finite() {
                return Err(TimeAxisError::NonFiniteTime { index, value });
            }
        }
        if self.sample_count > 1 {
            for (left, right) in [(0, 1), (self.sample_count - 2, self.sample_count - 1)] {
                if self.value_at(right) <= self.value_at(left) {
                    return Err(TimeAxisError::NonIncreasing { left, right });
                }
            }
        }
        Ok(())
    }

    pub fn compare(self, actual: Self) -> Result<(), TimeAxisMismatch> {
        if self.sample_count != actual.sample_count {
            return Err(TimeAxisMismatch::SampleCount {
                expected: self.sample_count,
                actual: actual.sample_count,
            });
        }
        for (name, expected, actual) in [
            ("x_increment", self.x_increment, actual.x_increment),
            ("x_origin", self.x_origin, actual.x_origin),
            ("x_reference", self.x_reference, actual.x_reference),
        ] {
            if !expected.is_finite() || !actual.is_finite() {
                return Err(TimeAxisMismatch::NonFinite { name });
            }
            let scale = expected.abs().max(actual.abs());
            let tolerance = (scale * 1.0e-12).max(1.0e-18);
            if (expected - actual).abs() > tolerance {
                return Err(TimeAxisMismatch::Value {
                    name,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeAxisError {
    Empty,
    NonPositiveIncrement(f64),
    NonFiniteTime { index: usize, value: f64 },
    NonIncreasing { left: usize, right: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeAxisMismatch {
    SampleCount {
        expected: usize,
        actual: usize,
    },
    NonFinite {
        name: &'static str,
    },
    Value {
        name: &'static str,
        expected: f64,
        actual: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawVoltageScale {
    pub y_increment: f64,
    pub y_origin: f64,
    pub y_reference: f64,
}

impl RawVoltageScale {
    pub fn value_at(self, word: u16) -> f64 {
        (word as f64 - self.y_origin - self.y_reference) * self.y_increment
    }

    pub fn validate_geometry(self) -> Result<(), VoltageScaleError> {
        if self.y_increment <= 0.0 {
            return Err(VoltageScaleError::InvalidIncrement(self.y_increment));
        }
        for word in [u16::MIN, u16::MAX] {
            let value = self.value_at(word);
            if !value.is_finite() {
                return Err(VoltageScaleError::NonFinite { word, value });
            }
        }
        for (left, right) in [(u16::MIN, 1), (u16::MAX - 1, u16::MAX)] {
            if self.value_at(right) <= self.value_at(left) {
                return Err(VoltageScaleError::Indistinguishable { left, right });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoltageScaleError {
    InvalidIncrement(f64),
    NonFinite { word: u16, value: f64 },
    Indistinguishable { left: u16, right: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_time_axis_endpoints_and_resolution() {
        let valid = RawTimeAxis {
            sample_count: 3,
            x_increment: 0.5,
            x_origin: -1.0,
            x_reference: 1.0,
        };
        assert_eq!(valid.build(), vec![-1.5, -1.0, -0.5]);
        assert_eq!(valid.validate_geometry(), Ok(()));

        let rounded = RawTimeAxis {
            x_origin: f64::MAX,
            ..valid
        };
        assert!(matches!(
            rounded.validate_geometry(),
            Err(TimeAxisError::NonIncreasing { .. })
        ));
    }

    #[test]
    fn compares_time_axes_with_scale_aware_tolerance() {
        let expected = RawTimeAxis {
            sample_count: 2,
            x_increment: 1.0e-9,
            x_origin: 1.0,
            x_reference: 0.0,
        };
        assert_eq!(
            expected.compare(RawTimeAxis {
                x_increment: 1.0e-9 + 1.0e-21,
                ..expected
            }),
            Ok(())
        );
        assert!(matches!(
            expected.compare(RawTimeAxis {
                x_increment: 2.0e-9,
                ..expected
            }),
            Err(TimeAxisMismatch::Value {
                name: "x_increment",
                ..
            })
        ));
    }

    #[test]
    fn validates_voltage_range_without_inspecting_captured_samples() {
        let valid = RawVoltageScale {
            y_increment: 0.5,
            y_origin: 1.0,
            y_reference: 2.0,
        };
        assert_eq!(valid.value_at(0), -1.5);
        assert_eq!(valid.validate_geometry(), Ok(()));

        let rounded = RawVoltageScale {
            y_increment: 1.0,
            y_origin: -1.0e30,
            y_reference: 0.0,
        };
        assert!(matches!(
            rounded.validate_geometry(),
            Err(VoltageScaleError::Indistinguishable { .. })
        ));
    }
}
