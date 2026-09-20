"""Liveness regression tests for the phase omega_t0 step.

Covers the recorded-scale hang report (phase analysis stuck with no
progress): the least-squares fit carries an explicit iteration cap and a
convergence guard, and the full analyse (fit + scatter render) must
complete within a wall-time budget instead of hanging silently.

The tests load the real embedded module
(`src/phase/pytools/omega_t0_analysis.py`) so they cover the shipped
code, not a copy.
"""

import concurrent.futures
import os
import sys
import tempfile
import unittest
from pathlib import Path

os.environ.setdefault("MPLBACKEND", "Agg")

import numpy as np

PYTOOLS = (
    Path(__file__).resolve().parent.parent
    / "src"
    / "phase"
    / "pytools"
)
sys.path.insert(0, str(PYTOOLS))

import omega_t0_analysis as ot0
from omega_t0_analysis import FIT_MAX_NFEV, OT0Analyser, ensure_converged


def synthetic_harmonics(samples, slope, seed=7):
    rng = np.random.default_rng(seed)
    return [
        (-slope * harmonic + rng.normal(0.0, 0.01, samples)).astype(float)
        for harmonic in (1, 2, 3, 4, 5, 6)
    ]


class FitCapTests(unittest.TestCase):
    def test_fit_cap_is_honored_and_slope_stays_accurate(self):
        slope = 0.001234
        arrays = synthetic_harmonics(2_000, slope)
        with tempfile.TemporaryDirectory(prefix="ot0live_") as tmp:
            result = OT0Analyser().analyse(
                arrays[0],
                arrays[1],
                arrays[2],
                arrays[3],
                arrays[4],
                arrays[5],
                True,
                False,
                os.path.join(tmp, "phase_offset.png"),
                100_000,
                "stride",
            )
        # Fails on the pre-fix module: no evaluation count is reported.
        self.assertIn("nfev", result)
        self.assertLessEqual(result["nfev"], FIT_MAX_NFEV)
        self.assertGreater(result["nfev"], 0)
        # No accuracy change from the cap on valid data.
        # (fit slope ~= -slope, and omega_t0 = -slope_fit.)
        self.assertAlmostEqual(result["omega_t0"], slope, delta=1e-3)
        self.assertIsNone(result["plot_error"])

    def test_convergence_guard_fails_closed(self):
        class FailedFit:
            success = False
            nfev = FIT_MAX_NFEV
            message = "mocked non-convergence"

        with self.assertRaisesRegex(RuntimeError, "failed to converge"):
            ensure_converged(FailedFit())


class AnalyseLivenessTests(unittest.TestCase):
    def test_analyse_completes_within_timeout(self):
        # Timeout-bounded: a hung fit or render fails here instead of
        # blocking the suite forever. The budget is generous (minutes)
        # against a seconds-scale step so runner variance cannot flake it.
        slope = -0.000987
        arrays = synthetic_harmonics(30_000, slope)
        with tempfile.TemporaryDirectory(prefix="ot0live_") as tmp:
            output = os.path.join(tmp, "phase_offset.png")

            def run():
                return OT0Analyser().analyse(
                    arrays[0],
                    arrays[1],
                    arrays[2],
                    arrays[3],
                    arrays[4],
                    arrays[5],
                    True,
                    False,
                    output,
                    100_000,
                    "stride",
                )

            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                result = pool.submit(run).result(timeout=240)
            self.assertAlmostEqual(result["omega_t0"], slope, delta=1e-3)
            self.assertTrue(Path(output).is_file())


class DecimationBudgetTests(unittest.TestCase):
    def test_decimation_indices_respect_the_point_budget(self):
        rng = np.random.default_rng(11)
        series = [rng.normal(0.0, 1.0, 50_000) for _ in range(6)]
        for method in ("stride", "min_max"):
            indices = ot0.decimation_indices(series, 10_000, method)
            self.assertLessEqual(len(indices), 10_000)
            self.assertGreater(len(indices), 0)


if __name__ == "__main__":
    unittest.main()
