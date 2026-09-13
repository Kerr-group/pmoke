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

    def test_pool_then_normalize_order(self):
        # Unequal-power blocks: pool raw autocovariances first, normalize
        # once by pooled lag zero. A=[1,1,-1,-1], B=[3,-3,3,-3], J=1.
        out = correlation([[1.0, 1.0, -1.0, -1.0], [3.0, -3.0, 3.0, -3.0]],
                          None, 1e-5, 1, eta=0.0)
        # Pooled auto0 = (1+9)/2 = 5; pooled auto1 = (0.25-6.75)/2 = -3.25;
        # normalized -0.65; taper (1-1/2) gives -0.325.
        self.assertAlmostEqual(out["lags"][0], 1.0, places=12)
        self.assertAlmostEqual(out["lags"][1], -0.325, places=12)

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
                   "phases": (rng.uniform(0, TAU, 2048)).tolist()} for _ in range(4)]
        out = scs_adequacy(groups[:2], groups[2:], 4, 10, 0.2, 0.2)
        self.assertTrue(out["adequate"])
        self.assertLess(out["reserved_shift"], 0.2)

    def test_reserved_periodic_structure_is_rejected(self):
        # White training, sign-flipping pair correlation on reserved data:
        # pooled averages cancel, per-octant structure must still reject.
        rng = np.random.RandomState(11)
        phases = (np.arange(512) / 512 * TAU).tolist()
        train = [{"standardized": rng.normal(0, 1, 512).tolist(), "phases": phases}]
        reserved = []
        for _ in range(2):
            series = np.empty(512)
            for pair in range(256):
                sign = 1.0 if phases[2 * pair] < math.pi else -1.0
                first = rng.normal()
                series[2 * pair] = first
                series[2 * pair + 1] = sign * 0.9 * first + math.sqrt(0.19) * rng.normal()
            reserved.append({"standardized": (series / series.std()).tolist(),
                             "phases": phases})
        out = scs_adequacy(train, reserved, 4, 10, 0.5, 0.5)
        self.assertFalse(out["adequate"])
        self.assertIn(out["reason"], ("reserved_spread", "pattern_shift"))

    def test_zero_power_single_phase_nan_policy_are_rejected(self):
        phases = (np.arange(256) / 256 * TAU).tolist()
        white = {"standardized": np.random.RandomState(3).normal(0, 1, 256).tolist(),
                 "phases": phases}
        zero = {"standardized": [0.0] * 256, "phases": phases}
        with self.assertRaises(ValueError):
            scs_adequacy([zero], [white], 4, 10, 0.5, 0.5)
        single_phase = {"standardized": white["standardized"],
                        "phases": [0.0] * 256}
        with self.assertRaises(ValueError):
            scs_adequacy([white], [single_phase], 4, 10, 0.5, 0.5)
        with self.assertRaises(ValueError):
            scs_adequacy([white], [white], 4, 10, float("nan"), 0.5)


if __name__ == "__main__":
    unittest.main()
