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

# Float regenerations cross platform libm boundaries (macOS-committed vs
# Linux-regenerated sin/cos agree to the last ulp, not to the last repr
# digit), so regenerated-vs-committed comparison is numeric with a tight
# tolerance, not byte-exact. Same-machine determinism is pinned separately
# by test_regeneration_is_byte_identical. A 1% legacy-output mutation
# (≈1e-2 relative) is orders of magnitude above these floors.
FLOAT_REL_TOL = 1.0e-12
# Absolute floor: gelsd solution noise is ~eps*cond*||beta|| ≈ 1e-15, and
# cross-version LAPACK rounding flips signs at that level. 1e-14 keeps 10x
# headroom while catching 1% mutations on every entry above 1e-12.
FLOAT_ABS_FLOOR = 1.0e-14
# Recorded generating-environment metadata, not numerical ground truth:
# regenerating under a newer SciPy must not fail the binding.
EXEMPT_VALUE_KEYS = {"numpy_version", "scipy_version"}


def assert_fixture_close(testcase, fresh, committed, path="root"):
    """Recursive fixture comparison: exact for structure and non-floats,
    tolerance-bound for floats."""
    if isinstance(fresh, float) and isinstance(committed, float):
        if fresh == committed:
            return
        testcase.assertTrue(
            math.isclose(fresh, committed, rel_tol=FLOAT_REL_TOL)
            or abs(fresh - committed) <= FLOAT_ABS_FLOOR,
            f"{path}: {fresh!r} vs {committed!r}",
        )
    elif isinstance(fresh, dict) and isinstance(committed, dict):
        testcase.assertEqual(sorted(fresh), sorted(committed), path)
        for key in fresh:
            if key in EXEMPT_VALUE_KEYS:
                continue
            assert_fixture_close(testcase, fresh[key], committed[key], f"{path}.{key}")
    elif isinstance(fresh, list) and isinstance(committed, list):
        testcase.assertEqual(len(fresh), len(committed), path)
        for index, (a, b) in enumerate(zip(fresh, committed)):
            assert_fixture_close(testcase, a, b, f"{path}[{index}]")
    else:
        testcase.assertEqual(fresh, committed, path)


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

    def test_regeneration_matches_committed_fixtures(self):
        # Binds the current generator to the committed payloads: a stale
        # golden survives neither a generator change nor a manual fixture edit.
        manifest = json.loads((FIXTURE_DIR / "manifest.json").read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as directory:
            proc = subprocess.run(
                ["python3", str(SCRIPTS / "generate_joint_gls_fixtures.py"),
                 "--output", directory],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            fresh = sorted(p.name for p in Path(directory).iterdir())
            self.assertEqual(fresh, sorted(["manifest.json", *manifest["files"]]))
            for name, entry in manifest["files"].items():
                regenerated = json.loads((Path(directory) / name).read_text(encoding="utf-8"))
                committed = json.loads((FIXTURE_DIR / name).read_text(encoding="utf-8"))
                assert_fixture_close(self, regenerated, committed, name)


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

    def test_adjacent_boundary_lengths(self):
        # L=260 is invalid; L=261 emits exactly one row centered at 140.
        self.assertEqual(legacy_geometry(260, 1e-5, 20, 1000.0, 1.0)["expected_error"],
                         "signal_too_short")
        grid = legacy_geometry(261, 1e-5, 20, 1000.0, 1.0)
        self.assertEqual(grid["i_start"], grid["i_end"])
        self.assertEqual(grid["first_center"], 140)
        self.assertEqual(grid["last_center"], 140)

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

    def test_each_family_reproduces_from_its_declared_seed(self):
        # Declared seeds are independent stream owners, not labels on one
        # advancing stream: regenerating one family reproduces its arrays.
        from generate_joint_gls_fixtures import build_noise_families
        payload = json.loads((FIXTURE_DIR / "noise.json").read_text(encoding="utf-8"))
        fresh = build_noise_families()
        for name, family in payload["families"].items():
            # Exact for seeds/kinds; tolerance-bound for libm-derived arrays.
            self.assertEqual(fresh[name]["seed"], family["seed"], name)
            self.assertEqual(fresh[name]["kind"], family["kind"], name)
            for key in ("values", "variances", "phases", "variance_profile"):
                if key in family:
                    np.testing.assert_allclose(
                        np.array(fresh[name][key]), np.array(family[key]),
                        rtol=FLOAT_REL_TOL, atol=FLOAT_ABS_FLOOR, err_msg=f"{name}.{key}",
                    )
            for key in ("sample_mean", "sample_var", "variance_mean"):
                if key in family:
                    self.assertAlmostEqual(fresh[name][key], family[key],
                                           places=12, msg=f"{name}.{key}")

    def test_phase_family_carries_samplewise_truth(self):
        from generate_joint_gls_fixtures import periodic_interp
        payload = json.loads((FIXTURE_DIR / "noise.json").read_text(encoding="utf-8"))
        family = payload["families"]["phase_heteroscedastic"]
        self.assertEqual(family["profile_kind"], "dimensionless_multiplier")
        phases = np.array(family["phases"])
        variances = np.array(family["variances"])
        self.assertEqual(len(phases), family["samples"])
        self.assertTrue(bool(np.all((phases >= 0.0) & (phases < 2.0 * math.pi))))
        self.assertTrue(bool(np.all(variances > 0.0)))
        # Knots sit at declared bin centers: interpolation there is exact.
        bins = family["bins"]
        centers = 2.0 * math.pi * (np.arange(bins) + 0.5) / bins
        profile = np.array(family["variance_profile"])
        at_centers = periodic_interp(centers, centers, profile)
        np.testing.assert_allclose(at_centers, profile, rtol=0, atol=1e-12)
        # Stored variances equal v0 times the center-based interpolation.
        np.testing.assert_allclose(
            variances, family["v0"] * periodic_interp(phases, centers, profile),
            rtol=0, atol=0)
        self.assertAlmostEqual(float(np.mean(variances)), family["variance_mean"],
                               places=12)


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

    def test_impulse_matches_closed_form_endpoint_weights(self):
        # Unit impulse at the inner-negative edge sample isolates the
        # half-weight plus fractional-edge correction. A mutant dropping the
        # edge integrals changes this row by ~19% relative.
        f_ref, dt, t0, phase = 999.0, 1e-5, 0.0, 0.3
        n, stride, harmonic = 3000, 50, 1
        omega = 2.0 * math.pi * f_ref
        half_window_s = 1.0 / f_ref
        n_half = 100
        edge_dt = half_window_s - n_half * dt
        self.assertGreater(edge_dt, 0.0)
        center = 200
        impulse = center - n_half
        signal = [0.0] * n
        signal[impulse] = 1.0
        _, xs, ys, _ = legacy_boxcar(signal, t0, dt, f_ref, phase, 1.0,
                                     stride, harmonic)
        step = -harmonic * omega * dt
        phase_zero = -harmonic * (omega * t0 - phase)
        mix_re = math.cos(phase_zero + impulse * step)
        mix_im = math.sin(phase_zero + impulse * step)
        scale = 1.0 / (2.0 * half_window_s)

        def edge(y0):
            interp = y0 * (dt - edge_dt) / dt
            return edge_dt * 0.5 * (y0 + interp)

        want_y = (0.5 * mix_re * dt + edge(mix_re)) * scale
        want_x = -(0.5 * mix_im * dt + edge(mix_im)) * scale
        self.assertAlmostEqual(ys[0], want_y, delta=1e-9 * (1.0 + abs(want_y)))
        self.assertAlmostEqual(xs[0], want_x, delta=1e-9 * (1.0 + abs(want_x)))
        # The edge correction is load-bearing, not rounding noise.
        self.assertGreater(abs(edge(mix_re) * scale), 1e-6)


if __name__ == "__main__":
    unittest.main()
