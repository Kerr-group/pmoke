use crate::utils::raw_data::RawTimeAxis;
use std::ops::Range;

#[derive(Debug, Clone)]
pub enum WaveformTime {
    Uniform(RawTimeAxis),
    Explicit(Vec<f64>),
}

impl WaveformTime {
    pub fn as_ref(&self) -> TimeAxisRef<'_> {
        match self {
            Self::Uniform(axis) => TimeAxisRef::Uniform(*axis),
            Self::Explicit(values) => TimeAxisRef::Explicit(values),
        }
    }

    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn value_at(&self, index: usize) -> f64 {
        self.as_ref().value_at(index)
    }

    pub fn to_vec(&self) -> Vec<f64> {
        self.as_ref().values(0..self.len())
    }
}

impl From<Vec<f64>> for WaveformTime {
    fn from(values: Vec<f64>) -> Self {
        Self::Explicit(values)
    }
}

impl From<RawTimeAxis> for WaveformTime {
    fn from(axis: RawTimeAxis) -> Self {
        Self::Uniform(axis)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TimeAxisRef<'a> {
    Uniform(RawTimeAxis),
    Explicit(&'a [f64]),
}

impl<'a> TimeAxisRef<'a> {
    pub fn len(self) -> usize {
        match self {
            Self::Uniform(axis) => axis.sample_count,
            Self::Explicit(values) => values.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn value_at(self, index: usize) -> f64 {
        assert!(index < self.len(), "time index out of bounds");
        match self {
            Self::Uniform(axis) => axis.value_at(index),
            Self::Explicit(values) => values[index],
        }
    }

    pub fn first(self) -> Option<f64> {
        (!self.is_empty()).then(|| self.value_at(0))
    }

    pub fn dt(self) -> Option<f64> {
        (self.len() >= 2).then(|| self.value_at(1) - self.value_at(0))
    }

    pub fn iter(self) -> TimeAxisIter<'a> {
        TimeAxisIter {
            axis: self,
            range: 0..self.len(),
        }
    }

    pub fn values(self, range: Range<usize>) -> Vec<f64> {
        assert!(range.start <= range.end && range.end <= self.len());
        match self {
            Self::Explicit(values) => values[range].to_vec(),
            Self::Uniform(_) => range.map(|index| self.value_at(index)).collect(),
        }
    }

    pub fn partition_point(self, mut predicate: impl FnMut(f64) -> bool) -> usize {
        let mut left = 0;
        let mut right = self.len();
        while left < right {
            let middle = left + (right - left) / 2;
            if predicate(self.value_at(middle)) {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }
}

impl<'a> From<&'a WaveformTime> for TimeAxisRef<'a> {
    fn from(time: &'a WaveformTime) -> Self {
        time.as_ref()
    }
}

impl<'a> From<&'a [f64]> for TimeAxisRef<'a> {
    fn from(values: &'a [f64]) -> Self {
        Self::Explicit(values)
    }
}

impl<'a> From<&'a Vec<f64>> for TimeAxisRef<'a> {
    fn from(values: &'a Vec<f64>) -> Self {
        Self::Explicit(values)
    }
}

impl<'a, const N: usize> From<&'a [f64; N]> for TimeAxisRef<'a> {
    fn from(values: &'a [f64; N]) -> Self {
        Self::Explicit(values)
    }
}

pub struct TimeAxisIter<'a> {
    axis: TimeAxisRef<'a>,
    range: Range<usize>,
}

impl Iterator for TimeAxisIter<'_> {
    type Item = f64;

    fn next(&mut self) -> Option<Self::Item> {
        self.range.next().map(|index| self.axis.value_at(index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for TimeAxisIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_and_explicit_axes_have_matching_operations() {
        let uniform = WaveformTime::Uniform(RawTimeAxis {
            sample_count: 4,
            x_increment: 0.5,
            x_origin: 1.0,
            x_reference: 1.0,
        });
        let explicit = WaveformTime::Explicit(vec![0.5, 1.0, 1.5, 2.0]);

        for time in [&uniform, &explicit] {
            assert_eq!(time.len(), 4);
            assert_eq!(time.value_at(2), 1.5);
            assert_eq!(time.as_ref().dt(), Some(0.5));
            assert_eq!(time.as_ref().values(1..3), vec![1.0, 1.5]);
            assert_eq!(time.as_ref().partition_point(|value| value < 1.5), 2);
        }
    }
}
