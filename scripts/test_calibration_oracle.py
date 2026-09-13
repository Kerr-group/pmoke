"""Self-tests for the calibration oracle (analytic anchors only)."""

import math
import unittest

import numpy as np

from calibration_oracle import (
    assemble_samples,
    correlation,
    nuisance_fit,
    phase_variance,
    scs_adequacy,
)

TAU = 2.0 * math.pi


class NuisanceTests(unittest.TestCase):
    def test_pure_dc_trend_harmonic_recovers_coefficients(self):
        f_ref, phase = 1000.0, 0.7
        times = (np.arange(512) * 1e-5).tolist()
        phi = TAU * f_ref * np.array(times) - phase
        trend = (np.array(times) - times[0]) / (times[-1] - times[0]) * 2.0 - 1.0
        signal = 0.5 + 0.25 * trend + 1.5 * np.cos(phi) - 0.5 * np.sin(phi)
        fit = nuisance_fit(times, signal.tolist(), f_ref, phase)
        self.assertEqual(fit["rank"], 26)
        self.assertAlmostEqual(fit["coefficients"][0], 0.5, places=9)
        self.assertAlmostEqual(fit["coefficients"][1], 0.25, places=9)
        self.assertAlmostEqual(fit["coefficients"][2], 1.5, places=9)
        self.assertAlmostEqual(fit["coefficients"][3], -0.5, places=9)
        for coefficient in fit["coefficients"][4:]:
            self.assertAlmostEqual(coefficient, 0.0, places=9)
        np.testing.assert_allclose(fit["residual"], np.zeros(512), atol=1e-9)


class PhaseVarianceTests(unittest.TestCase):
    def test_pooled_count_minus_one_anchor(self):
        # Three residuals in one of two bins: variance is exactly 1.0.
        samples = [
            {"phase": 0.1, "cycle": i, "residual": value, "block": 0}
            for i, value in enumerate([1.0, 2.0, 3.0])
        ] + [
            {"phase": TAU - 0.1, "cycle": 100 + i,
             "residual": 1.0 if i % 2 == 0 else -1.0, "block": 1}
            for i in range(300)
        ]
        out = phase_variance(samples, 2, min_samples_per_bin=3,
                             min_cycles_per_bin=3, min_contributing_blocks=2,
                             shrinkage_alpha=0.0, floor_ratio=1e-9)
        self.assertAlmostEqual(out["raw"][0], 1.0, places=12)
        self.assertAlmostEqual(out["raw"][1], 300.0 / 299.0, places=9)

    def test_smoothing_shrink_floor_order(self):
        out = phase_variance(
            [{"phase": (b + 0.5) / 4 * TAU, "cycle": 1000 + b * 300 + i,
              "residual": math.sqrt(v) * (1.0 if (i % 2 == 0) else -1.0), "block": b % 3}
             for b, v in enumerate([1.0, 4.0, 9.0, 16.0]) for i in range(300)],
            4, min_samples_per_bin=300, min_cycles_per_bin=300,
            min_contributing_blocks=3, shrinkage_alpha=0.0, floor_ratio=1e-9)
        # Cyclic smoothing: (16 + 2*1 + 4)/4 for bin 0, times the
        # count/(count-1) pooled-variance factor of the 300-sample bins.
        self.assertAlmostEqual(out["smoothed"][0], 5.5 * 300.0 / 299.0, places=9)
        self.assertAlmostEqual(out["variances"][0], out["smoothed"][0], places=12)

    def test_missing_bin_is_an_error(self):
        samples = [{"phase": 0.1, "cycle": i, "residual": 1.0, "block": 0}
                   for i in range(300)]
        with self.assertRaises(ValueError):
            phase_variance(samples, 4, min_samples_per_bin=3,
                           min_cycles_per_bin=3, min_contributing_blocks=1)


class CorrelationTests(unittest.TestCase):
    def test_biased_denominator_anchor(self):
        # Zero-mean x = [1,0,-1,0]: biased auto = [0.5,0,-0.25,0].
        out = correlation([[1.0, 0.0, -1.0, 0.0]], None, 1e-5, 3, eta=0.0)
        self.assertAlmostEqual(out["lags"][0], 1.0, places=12)
        self.assertAlmostEqual(out["lags"][1], 0.0, places=12)
        self.assertAlmostEqual(out["lags"][2], -0.5 * (1.0 - 2.0 / 4.0), places=12)

    def test_taper_shrink_order(self):
        # Constant block has no lag-zero power after DC removal.
        with self.assertRaises(ValueError):
            correlation([[1.0, 1.0, 1.0, 1.0]], None, 1e-5, 2)
        out = correlation([[1.0, -1.0, 1.0, -1.0, 1.0, -1.0]], None, 1e-5, 2, eta=0.1)
        # Biased normalization on 6 samples: lag1 = -5/6, lag2 = 4/6.
        # Taper (1-l/3); shrink toward delta with eta = 0.1.
        self.assertAlmostEqual(out["lags"][0], 1.0, places=12)
        self.assertAlmostEqual(out["lags"][1], 0.9 * (-5.0 / 6.0) * (2.0 / 3.0), places=12)
        self.assertAlmostEqual(out["lags"][2], 0.9 * (4.0 / 6.0) * (1.0 / 3.0), places=12)


class AdequacyTests(unittest.TestCase):
    def test_identical_groups_are_adequate(self):
        rng = np.random.RandomState(7)
        groups = [{"standardized": rng.normal(0, 1, 2048).tolist(),
                   "phases": (rng.uniform(0, TAU, 2048)).tolist()} for _ in range(3)]
        out = scs_adequacy(groups[:2], groups[2:], 4, 10, 0.2, 0.2)
        self.assertTrue(out["adequate"])
        self.assertLess(out["reserved_shift"], 0.2)


if __name__ == "__main__":
    unittest.main()
