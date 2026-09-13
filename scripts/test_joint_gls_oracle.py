"""Self-consistency tests for the independent SciPy oracle (AT-010 input)."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from joint_gls_oracle import (
    covariance_pinv_reference,
    design_matrix,
    map_to_xy,
    solve_case,
    solve_whitened,
    whiten_diagonal,
    whiten_identity,
)

SCRIPTS = Path(__file__).resolve().parent
FIXTURE_DIR = (
    SCRIPTS.parent / "crates" / "pmoke-analysis-core" / "tests"
    / "fixtures" / "joint-gls"
)


def convention_case(name):
    payload = json.loads((FIXTURE_DIR / "conventions.json").read_text(encoding="utf-8"))
    return next(c for c in payload["cases"] if c["name"] == name)


class OracleRecoveryTests(unittest.TestCase):
    def test_noiseless_conventions_recover_beta(self):
        for name in ("mixed_phases_fractional_t0", "large_even_weak_odd",
                     "negative_fractional_t0"):
            case = convention_case(name)
            times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
            result = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                                case["fit_harmonics"], case["output_harmonics"])
            self.assertEqual(result["rank"], 2 * len(case["fit_harmonics"]) + 1)
            for got, want in zip(result["beta"], case["expected_beta"]):
                self.assertAlmostEqual(got, want, delta=1e-8, msg=name)
            for key, pair in result["xy"].items():
                want = case["expected_xy"][key]
                self.assertAlmostEqual(pair["x"], want["x"], delta=1e-9, msg=name)
                self.assertAlmostEqual(pair["y"], want["y"], delta=1e-9, msg=name)

    def test_residuals_orthogonal_to_design(self):
        case = convention_case("mixed_phases_fractional_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        design = design_matrix(times, case["f_ref"], case["phase_rad"],
                               case["fit_harmonics"])
        rng = np.random.RandomState(7)
        noisy = np.array(case["signal"]) + rng.normal(0.0, 0.01, case["samples"])
        dw, yw, _ = whiten_identity(design, noisy)
        result = solve_whitened(dw, yw)
        residual = yw - dw @ np.array(result["beta"])
        self.assertLess(float(np.max(np.abs(dw.T @ residual))), 1e-8)

    def test_scaling_and_additivity(self):
        case = convention_case("negative_fractional_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        base = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                          case["fit_harmonics"], case["output_harmonics"])
        scaled = solve_case(times, (3.0 * np.array(case["signal"])).tolist(),
                            case["f_ref"], case["phase_rad"],
                            case["fit_harmonics"], case["output_harmonics"])
        for got, want in zip(scaled["beta"], base["beta"]):
            self.assertAlmostEqual(got, 3.0 * want, delta=1e-9)

    def test_diagonal_whitening_matches_analytic_anchor(self):
        # D=[[1],[1]], y=[0,1], variances=[1,4]: weighted coefficient is
        # (0/1 + 1/4)/(1 + 1/4) = 0.2, known-noise covariance 1/1.25 = 0.8.
        # Identity whitening gives 0.5 instead, so a whitening bypass fails.
        design = [[1.0], [1.0]]
        dw, yw, _ = whiten_diagonal(design, [0.0, 1.0], [1.0, 4.0])
        result = solve_whitened(dw, yw)
        self.assertAlmostEqual(result["beta"][0], 0.2, delta=1e-12)
        cov = covariance_pinv_reference(np.array(dw))
        self.assertAlmostEqual(float(cov[0, 0]), 0.8, delta=1e-12)
        self.assertAlmostEqual(
            float(covariance_pinv_reference(np.array(dw), sigma2=4.0)[0, 0]),
            3.2, delta=1e-12)
        identity = solve_whitened(*whiten_identity(design, [0.0, 1.0])[:2])
        self.assertAlmostEqual(identity["beta"][0], 0.5, delta=1e-12)

    def test_weighted_form_matches_closed_form(self):
        # Independent inline weighted average, not the same driver call.
        # DC-only design: the weighted mean is the exact closed form.
        case = convention_case("mixed_phases_fractional_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        design = design_matrix(times, case["f_ref"], case["phase_rad"], [])
        assert design.shape == (case["samples"], 1)
        variances = 1.0 + 0.5 * np.sin(np.linspace(0, 4 * np.pi, case["samples"])) ** 2
        dw, yw, _ = whiten_diagonal(design, case["signal"], variances)
        got = float(solve_whitened(dw, yw)["beta"][0])
        w = 1.0 / variances
        want = float(np.sum(w * np.array(case["signal"])) / np.sum(w))
        self.assertAlmostEqual(got, want, delta=1e-9)

    def test_rectangular_overdetermined_batched(self):
        rng = np.random.RandomState(11)
        design = rng.normal(size=(512, 9))
        truth = rng.normal(size=(9, 3))
        response = design @ truth + 1e-3 * rng.normal(size=(512, 3))
        dw, yw, _ = whiten_identity(design, response)
        batched = solve_whitened(dw, yw)
        self.assertEqual(batched["rank"], 9)
        self.assertEqual(len(batched["beta"]), 3)
        for column in range(3):
            single = solve_whitened(*whiten_identity(design, response[:, column])[:2])
            for got, want in zip(batched["beta"][column], single["beta"]):
                self.assertAlmostEqual(got, want, delta=1e-12)
            for got, want in zip(batched["beta"][column], truth[:, column]):
                self.assertLess(abs(got - want), 0.05)

    def test_qr_driver_solves_without_singular_values(self):
        case = convention_case("mixed_phases_fractional_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        qr = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                        case["fit_harmonics"], case["output_harmonics"], driver="gelsy")
        svd = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                         case["fit_harmonics"], case["output_harmonics"], driver="gelsd")
        self.assertIsNone(qr["singular_values"])
        self.assertIsNotNone(svd["singular_values"])
        for got, want in zip(qr["beta"], svd["beta"]):
            self.assertAlmostEqual(got, want, delta=1e-9)

    def test_fractional_start_offset_is_phase_significant(self):
        # Dropping t0 from the time coordinates must materially change the
        # recovered coefficients on fractional-period-offset fixtures.
        case = convention_case("mixed_phases_fractional_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        good = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                          case["fit_harmonics"], case["output_harmonics"])
        for got, want in zip(good["beta"], case["expected_beta"]):
            self.assertAlmostEqual(got, want, delta=1e-8)
        zeroed = [t - case["t_start"] for t in times]
        bad = solve_case(zeroed, case["signal"], case["f_ref"], case["phase_rad"],
                         case["fit_harmonics"], case["output_harmonics"])
        error = max(abs(g - w) for g, w in zip(bad["beta"], case["expected_beta"]))
        self.assertGreater(error, 1e-3)

    def test_solve_case_rejects_matrix_response(self):
        with self.assertRaises(ValueError):
            solve_case([0.0, 0.1], [[0.0, 1.0], [2.0, 3.0]], 1000.0, 0.0, [1], [1])

    def test_covariance_reference_is_symmetric(self):
        case = convention_case("mixed_phases_fractional_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        design = design_matrix(times, case["f_ref"], case["phase_rad"],
                               case["fit_harmonics"])
        cov = covariance_pinv_reference(design, sigma2=4.0)
        self.assertLess(float(np.max(np.abs(cov - cov.T))), 1e-12)
        self.assertTrue(bool(np.all(np.diag(cov) > 0)))

    def test_cli_writes_result_file(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "oracle.json"
            import subprocess
            proc = subprocess.run(
                ["python3", str(SCRIPTS / "joint_gls_oracle.py"),
                 "--fixture", str(FIXTURE_DIR / "conventions.json"),
                 "--case", "large_even_weak_odd",
                 "--output", str(output)],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            result = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(result["case"], "large_even_weak_odd")
            self.assertIn("1", result["xy"])


class MapToXyTests(unittest.TestCase):
    def test_half_amplitude_mapping(self):
        beta = [10.0, 0.0, 4.0, -6.0, 0.0]
        xy = map_to_xy(beta, [1, 2], [1, 2])
        self.assertEqual(xy["1"], {"x": 2.0, "y": 0.0})
        self.assertEqual(xy["2"], {"x": 0.0, "y": -3.0})

    def test_sparse_fitting_list_selects_by_position(self):
        # fit=[1,3,5] with true (a3,b3)=(4,6): X3/Y3 must be 3/2, not A5.
        rng = np.random.RandomState(23)
        n = 512
        times = rng.uniform(0.0, 0.01, n).tolist()
        phi = 2.0 * np.pi * 1000.0 * np.array(times) - 0.4
        signal = (4.0 * np.cos(3 * phi) + 6.0 * np.sin(3 * phi)).tolist()
        result = solve_case(times, signal, 1000.0, 0.4, [1, 3, 5], [3])
        self.assertAlmostEqual(result["xy"]["3"]["x"], 3.0, delta=1e-8)
        self.assertAlmostEqual(result["xy"]["3"]["y"], 2.0, delta=1e-8)

    def test_output_outside_fitting_list_is_rejected(self):
        with self.assertRaises(ValueError):
            map_to_xy([1.0, 2.0, 3.0], [1, 3], [5])


if __name__ == "__main__":
    unittest.main()
