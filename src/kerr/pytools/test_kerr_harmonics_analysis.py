import sys
import types
import unittest

import numpy as np
from scipy.special import jv

sys.modules.setdefault("gsplot", types.ModuleType("gsplot"))

from kerr_harmonics_analysis import KerrHarmonicsAnalyser


class KerrHarmonicsAnalyserTests(unittest.TestCase):
    def test_recovers_folded_angle_and_ignores_common_signal_sign(self):
        modulation_depth = 1.84

        for theta in (0.01, -0.01, 0.7, 1.0, -1.0):
            expected = 0.5 * np.arctan(np.tan(2 * theta))
            results = []
            for gain in (1.0, -1.0):
                even = gain * np.cos(2 * theta)
                a2 = np.array([even * jv(2, modulation_depth)])
                a3 = np.array(
                    [gain * np.sin(2 * theta) * jv(3, modulation_depth)]
                )
                a4 = np.array([even * jv(4, modulation_depth)])
                a6 = np.array([even * jv(6, modulation_depth)])

                x0 = KerrHarmonicsAnalyser.get_modulation_depth(a2, a4, a6)
                results.append(
                    KerrHarmonicsAnalyser.get_kerr(x0, a2, a3, a4)[0]
                )

            self.assertAlmostEqual(results[0], expected)
            self.assertAlmostEqual(results[1], expected)

    def test_representative_modulation_depth_ignores_invalid_values_and_outlier(self):
        x0 = np.array([np.nan, np.inf, -1.0, 1.83, 1.84, 1.85, 100.0])

        actual = KerrHarmonicsAnalyser.get_representative_modulation_depth(x0)

        self.assertAlmostEqual(actual, 1.845)

    def test_representative_modulation_depth_rejects_missing_valid_value(self):
        with self.assertRaisesRegex(ValueError, "finite positive modulation depth"):
            KerrHarmonicsAnalyser.get_representative_modulation_depth(
                np.array([np.nan, np.inf, -1.0, 0.0])
            )

    def test_invalid_modulation_depth_is_nan_without_warning(self):
        with np.errstate(all="raise"):
            actual = KerrHarmonicsAnalyser.get_modulation_depth(
                np.array([0.0, 1.0]),
                np.array([0.0, -0.1]),
                np.array([0.0, 0.0]),
            )

        self.assertTrue(np.isnan(actual).all())

    def test_zero_over_zero_is_nan_without_warning(self):
        with np.errstate(all="raise"):
            actual = KerrHarmonicsAnalyser.get_kerr(
                np.array([1.84]),
                np.array([0.0]),
                np.array([0.0]),
                np.array([0.0]),
            )

        self.assertTrue(np.isnan(actual[0]))

    def test_nonzero_over_zero_reaches_fold_boundary_without_warning(self):
        with np.errstate(all="raise"):
            actual = KerrHarmonicsAnalyser.get_kerr(
                np.array([1.84, 1.84]),
                np.array([0.0, 0.0]),
                np.array([1.0, -1.0]),
                np.array([0.0, 0.0]),
            )

        np.testing.assert_allclose(actual, [np.pi / 4, -np.pi / 4])

    def test_negative_denominator_preserves_ratio_sign(self):
        actual = KerrHarmonicsAnalyser.get_kerr(
            np.array([1.84]),
            np.array([-2.0]),
            np.array([0.5]),
            np.array([-1.0]),
        )
        expected = 0.5 * np.arctan(0.5 / ((-2.0 - 1.0) * 1.84 / 6))

        self.assertAlmostEqual(actual[0], expected)


if __name__ == "__main__":
    unittest.main()
