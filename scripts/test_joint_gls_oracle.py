"""Self-consistency tests for the independent SciPy oracle (AT-010 input)."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
import scipy.linalg

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
        for name in ("mixed_phases_nonzero_t0", "large_even_weak_odd",
                     "negative_start_phase"):
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
        case = convention_case("mixed_phases_nonzero_t0")
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
        case = convention_case("negative_start_phase")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        base = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                          case["fit_harmonics"], case["output_harmonics"])
        scaled = solve_case(times, (3.0 * np.array(case["signal"])).tolist(),
                            case["f_ref"], case["phase_rad"],
                            case["fit_harmonics"], case["output_harmonics"])
        for got, want in zip(scaled["beta"], base["beta"]):
            self.assertAlmostEqual(got, 3.0 * want, delta=1e-9)

    def test_diagonal_whitening_matches_weighted_form(self):
        case = convention_case("mixed_phases_nonzero_t0")
        times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
        design = design_matrix(times, case["f_ref"], case["phase_rad"],
                               case["fit_harmonics"])
        variances = 1.0 + 0.5 * np.sin(np.linspace(0, 4 * np.pi, case["samples"])) ** 2
        dw, yw, _ = whiten_diagonal(design, case["signal"], variances)
        got = np.array(solve_whitened(dw, yw)["beta"])
        # Independent closed form via SVD pseudoinverse of the whitened design.
        want, _, _, _ = scipy.linalg.lstsq(dw, yw, lapack_driver="gelsd")
        self.assertLess(float(np.max(np.abs(got - want))), 1e-12)

    def test_rectangular_overdetermined_multiple_rhs(self):
        rng = np.random.RandomState(11)
        design = rng.normal(size=(512, 9))
        truth = rng.normal(size=(9, 3))
        response = design @ truth + 1e-3 * rng.normal(size=(512, 3))
        for column in range(3):
            result = solve_whitened(*whiten_identity(design, response[:, column])[:2])
            self.assertEqual(result["rank"], 9)
            self.assertLess(float(np.max(np.abs(result["beta"] - truth[:, column]))), 0.05)

    def test_covariance_reference_is_symmetric(self):
        case = convention_case("mixed_phases_nonzero_t0")
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
        xy = map_to_xy(beta, [1, 2])
        self.assertEqual(xy["1"], {"x": 2.0, "y": 0.0})
        self.assertEqual(xy["2"], {"x": 0.0, "y": -3.0})


if __name__ == "__main__":
    unittest.main()
