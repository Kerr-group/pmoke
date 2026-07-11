import sys
import types
import unittest

import numpy as np
from scipy.special import jn

sys.modules.setdefault("gsplot", types.ModuleType("gsplot"))

from kerr_standard_analysis import KerrStandardAnalyser, decimation_indices


class KerrStandardAnalyserTests(unittest.TestCase):
    def test_min_max_decimation_keeps_narrow_extrema(self):
        values = np.zeros(100)
        values[47] = 10.0
        values[48] = -8.0

        indices = decimation_indices(values, 10, "min_max")

        self.assertIn(47, indices)
        self.assertIn(48, indices)
        self.assertLessEqual(len(indices), 10)

    def test_one_point_min_max_keeps_largest_absolute_extreme(self):
        indices = decimation_indices(np.array([0.0, -9.0, 4.0]), 1, "min_max")

        np.testing.assert_array_equal(indices, [1])

    def test_common_signal_sign_cancels_in_folded_angle(self):
        theta = 0.01
        phim = 0.92
        a1 = np.sin(2 * theta) / jn(2, 2 * phim)
        a2 = np.cos(2 * theta) / jn(1, 2 * phim)

        positive = KerrStandardAnalyser.calculate(np.array([a1]), np.array([a2]))
        negative = KerrStandardAnalyser.calculate(np.array([-a1]), np.array([-a2]))

        self.assertAlmostEqual(positive[0], theta)
        self.assertAlmostEqual(negative[0], theta)

    def test_zero_over_zero_is_nan_without_warning(self):
        with np.errstate(all="raise"):
            actual = KerrStandardAnalyser.calculate(
                np.array([0.0]), np.array([0.0])
            )

        self.assertTrue(np.isnan(actual[0]))

    def test_nonzero_over_zero_reaches_fold_boundary_without_warning(self):
        with np.errstate(all="raise"):
            actual = KerrStandardAnalyser.calculate(
                np.array([1.0, -1.0]), np.array([0.0, 0.0])
            )

        np.testing.assert_allclose(actual, [np.pi / 4, -np.pi / 4])


if __name__ == "__main__":
    unittest.main()
