"""Tests for the joint-GLS fixture generator (WP-1 fixture evidence)."""

import hashlib
import json
import math
import subprocess
import tempfile
import unittest
from pathlib import Path

import numpy as np

from generate_joint_gls_fixtures import legacy_boxcar, legacy_geometry, mixture_value

SCRIPTS = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS.parent
FIXTURE_DIR = REPO_ROOT / "crates" / "pmoke-analysis-core" / "tests" / "fixtures" / "joint-gls"


class GeneratorDeterminismTests(unittest.TestCase):
    def test_regeneration_is_byte_identical(self):
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            for directory in (first, second):
                proc = subprocess.run(
                    ["python3", str(SCRIPTS / "generate_joint_gls_fixtures.py"),
                     "--output", directory],
                    capture_output=True, text=True, check=False,
                )
                self.assertEqual(proc.returncode, 0, proc.stderr)
            first_files = sorted(Path(first).iterdir())
            second_files = sorted(Path(second).iterdir())
            self.assertEqual([p.name for p in first_files], [p.name for p in second_files])
            for a, b in zip(first_files, second_files):
                self.assertEqual(a.read_bytes(), b.read_bytes())

    def test_manifest_hashes_match_files(self):
        manifest = json.loads((FIXTURE_DIR / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["schema_version"], 1)
        self.assertEqual(manifest["license"], "CC0-1.0")
        for name, entry in manifest["files"].items():
            digest = hashlib.sha256((FIXTURE_DIR / name).read_bytes()).hexdigest()
            self.assertEqual(digest, entry["sha256"], name)


class GeometryFormulaTests(unittest.TestCase):
    def test_hand_computed_shortest_valid_case(self):
        # L=421, dt=1e-5, s=20, f=1000, c=1: half=1e-3, n_half=100,
        # integration=(420)//20+1=22, i_start=2+101//20=7, i_end=15.
        grid = legacy_geometry(421, 1e-5, 20, 1000.0, 1.0)
        self.assertEqual(grid["n_half"], 100)
        self.assertEqual(grid["i_start"], 7)
        self.assertEqual(grid["i_end"], 15)
        self.assertEqual(grid["row_count"], 9)
        self.assertEqual(grid["first_center"], 140)
        self.assertEqual(grid["last_center"], 300)

    def test_error_codes(self):
        self.assertEqual(legacy_geometry(200, 1e-5, 20, 1000.0, 1.0)["expected_error"],
                         "signal_too_short")
        self.assertEqual(legacy_geometry(1, 1e-5, 20, 1000.0, 1.0)["expected_error"],
                         "signal_too_short")
        self.assertEqual(legacy_geometry(20000, 1e-5, 0, 1000.0, 1.0)["expected_error"],
                         "invalid_stride")
        self.assertEqual(legacy_geometry(20000, 1e-5, 20, 2000000.0, 1.0)["expected_error"],
                         "window_too_short")


class MixtureReconstructionTests(unittest.TestCase):
    def test_committed_signals_match_closed_form(self):
        payload = json.loads((FIXTURE_DIR / "conventions.json").read_text(encoding="utf-8"))
        for case in payload["cases"]:
            t0, dt = case["t_start"], case["dt"]
            coeffs = {int(k): tuple(v) for k, v in case["coeffs"].items()}
            for i, stored in enumerate(case["signal"]):
                expected = mixture_value(t0 + i * dt, case["f_ref"], case["phase_rad"],
                                         case["dc"], coeffs)
                self.assertAlmostEqual(stored, expected, places=12, msg=case["name"])


class NoiseFamilyTests(unittest.TestCase):
    def test_recorded_moments_match_arrays(self):
        payload = json.loads((FIXTURE_DIR / "noise.json").read_text(encoding="utf-8"))
        for name, family in payload["families"].items():
            values = np.array(family["values"])
            self.assertEqual(len(values), family["samples"], name)
            self.assertTrue(np.all(np.isfinite(values)), name)
            self.assertAlmostEqual(float(np.mean(values)), family["sample_mean"],
                                   places=12, msg=name)
            self.assertAlmostEqual(float(np.var(values)), family["sample_var"],
                                   places=12, msg=name)

    def test_ar1_lag_correlation_near_theory(self):
        payload = json.loads((FIXTURE_DIR / "noise.json").read_text(encoding="utf-8"))
        family = payload["families"]["ar1_correlated"]
        # N=2048 gives lag-1 standard error ~1/sqrt(N); 5 sigma is generous.
        self.assertAlmostEqual(family["lag1_corr"], family["rho"], delta=0.12)


class LegacyBoxcarSmokeTests(unittest.TestCase):
    def test_pure_sine_recovers_half_amplitude(self):
        # y = 2 sin(phi): legacy X1 must equal 1 within window ripple.
        f_ref, dt, t0, phase = 1000.0, 1e-5, 0.0, 0.0
        n = 3000
        signal = [2.0 * math.sin(2.0 * math.pi * f_ref * (t0 + i * dt) - phase)
                  for i in range(n)]
        _, xs, _, _ = legacy_boxcar(signal, t0, dt, f_ref, phase, 1.0, 50, 1)
        self.assertTrue(xs)
        for value in xs:
            self.assertAlmostEqual(value, 1.0, delta=0.02)


if __name__ == "__main__":
    unittest.main()
