import sys
import types
import unittest

import numpy as np
from scipy.special import jn

sys.modules.setdefault("gsplot", types.ModuleType("gsplot"))

from kerr_standard_analysis import KerrStandardAnalyser


class KerrStandardAnalyserTests(unittest.TestCase):
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
