"""Independent least-squares oracle for joint harmonic estimation fixtures.

Builds the NUMERICS section 2 design matrix, whitens y and D together, and
solves the whitened problem with SciPy QR/SVD drivers. Never forms normal
equations. Used to cross-check the Rust direct solver (M2, AT-010) and to
validate the public convention fixtures produced by
generate_joint_gls_fixtures.py.
"""

import argparse
import json
import math
from pathlib import Path

import numpy as np
import scipy.linalg


def design_matrix(times, f_ref, phase_rad, fit_harmonics):
    """Columns [DC, cos(k phi), sin(k phi), ...]; phi = 2 pi f t - phase."""
    phi = 2.0 * math.pi * f_ref * np.asarray(times, dtype=float) - phase_rad
    columns = [np.ones_like(phi)]
    for k in fit_harmonics:
        columns.append(np.cos(k * phi))
        columns.append(np.sin(k * phi))
    return np.column_stack(columns)


def whiten_identity(design, signal):
    return design.copy(), np.asarray(signal, dtype=float).copy(), {"mode": "identity"}


def whiten_diagonal(design, signal, variances):
    w = 1.0 / np.sqrt(np.asarray(variances, dtype=float))
    return design * w[:, None], np.asarray(signal, dtype=float) * w, {"mode": "phase_diagonal"}


def whiten_known_cholesky(design, signal, chol):
    """Whiten with lower Cholesky factor L of R: solve L X = D, L yv = y."""
    lower = np.asarray(chol, dtype=float)
    dw = scipy.linalg.solve_triangular(lower, design, lower=True)
    yw = scipy.linalg.solve_triangular(lower, np.asarray(signal, dtype=float), lower=True)
    return dw, yw, {"mode": "known_cholesky"}


def solve_whitened(design_w, signal_w, driver="gelsd"):
    """Least squares via LAPACK driver (default gelsd: SVD-based)."""
    beta, residuals, rank, singular = scipy.linalg.lstsq(
        design_w, signal_w, lapack_driver=driver
    )
    return {
        "beta": beta,
        "residual_norm": float(np.linalg.norm(signal_w - design_w @ beta)),
        "rank": int(rank),
        "singular_values": singular,
        "driver": driver,
    }


def map_to_xy(beta, output_harmonics):
    """Legacy half-amplitude mapping: Xk = b_k / 2, Yk = a_k / 2."""
    xy = {}
    for k in output_harmonics:
        a, b = beta[2 * k - 1], beta[2 * k]
        xy[str(k)] = {"x": float(b / 2.0), "y": float(a / 2.0)}
    return xy


def covariance_pinv_reference(design_w, sigma2=1.0):
    """Reference covariance sigma2 * pinv(Dw) pinv(Dw)^T via SVD. Oracle only."""
    pinv = np.linalg.pinv(design_w)
    return sigma2 * pinv @ pinv.T


def solve_case(times, signal, f_ref, phase_rad, fit_harmonics, output_harmonics,
               noise=None, driver="gelsd"):
    design = design_matrix(times, f_ref, phase_rad, fit_harmonics)
    if noise is None:
        dw, yw, info = whiten_identity(design, signal)
    elif noise["mode"] == "phase_diagonal":
        dw, yw, info = whiten_diagonal(design, signal, noise["variances"])
    elif noise["mode"] == "known_cholesky":
        dw, yw, info = whiten_known_cholesky(design, signal, noise["chol"])
    else:
        raise ValueError(f"unknown noise mode: {noise['mode']}")
    result = solve_whitened(dw, yw, driver=driver)
    result["xy"] = map_to_xy(result["beta"], output_harmonics)
    result["whitening"] = info
    result["beta"] = [float(v) for v in result["beta"]]
    result["singular_values"] = [float(v) for v in result["singular_values"]]
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--case", required=True)
    parser.add_argument("--output", type=Path, default=None)
    args: argparse.Namespace = parser.parse_args()
    payload = json.loads(args.fixture.read_text(encoding="utf-8"))
    case = next(c for c in payload["cases"] if c["name"] == args.case)
    times = [case["t_start"] + i * case["dt"] for i in range(case["samples"])]
    result = solve_case(times, case["signal"], case["f_ref"], case["phase_rad"],
                        case["fit_harmonics"], case["output_harmonics"])
    result["case"] = case["name"]
    text = json.dumps(result, indent=1) + "\n"
    if args.output is not None:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
