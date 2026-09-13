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
    return (np.asarray(design, dtype=float), np.asarray(signal, dtype=float).copy(),
            {"mode": "identity"})


def whiten_diagonal(design, signal, variances):
    w = 1.0 / np.sqrt(np.asarray(variances, dtype=float))
    dw = design * w[:, None]
    y = np.asarray(signal, dtype=float)
    if y.ndim == 1:
        return dw, y * w, {"mode": "phase_diagonal"}
    if y.ndim == 2 and y.shape[0] == design.shape[0]:
        return dw, y * w[:, None], {"mode": "phase_diagonal"}
    raise ValueError(f"response shape {y.shape} mismatches design rows {design.shape[0]}")


def whiten_known_cholesky(design, signal, chol):
    """Whiten with lower Cholesky factor L of R: solve L X = D, L yv = y."""
    lower = np.asarray(chol, dtype=float)
    dw = scipy.linalg.solve_triangular(lower, design, lower=True)
    yw = scipy.linalg.solve_triangular(lower, np.asarray(signal, dtype=float), lower=True)
    return dw, yw, {"mode": "known_cholesky"}


def solve_whitened(design_w, signal_w, driver="gelsd"):
    """Least squares via LAPACK driver (gelsd SVD-based; gelsy QR-based).

    Returns beta as a flat list for vector responses, or a list of rows for
    matrix (batched) responses. gelsy reports no singular values; they are
    serialized as None so callers cannot mistake absence for zeros.
    """
    y = np.asarray(signal_w, dtype=float)
    if y.ndim not in (1, 2):
        raise ValueError(f"response must be a vector or matrix, got shape {y.shape}")
    beta, residuals, rank, singular = scipy.linalg.lstsq(
        design_w, y, lapack_driver=driver
    )
    if y.ndim == 1:
        beta_out: object = [float(v) for v in beta]
        resid = float(np.linalg.norm(y - design_w @ beta))
    else:
        # Outer index is the response column: beta_out[k] holds the
        # parameter vector for response column k.
        beta_out = [[float(v) for v in col] for col in beta.T]
        resid = float(np.linalg.norm(y - design_w @ beta))
    return {
        "beta": beta_out,
        "residual_norm": resid,
        "rank": int(rank),
        "singular_values": None if singular is None else [float(v) for v in singular],
        "driver": driver,
    }


def map_to_xy(beta, fit_harmonics, output_harmonics):
    """Legacy half-amplitude mapping: Xk = b_k / 2, Yk = a_k / 2.

    Coefficients are selected by fitting-list position, not harmonic number,
    so sparse lists like [1, 3, 5] resolve correctly. Every output must be a
    member of the fitting list.
    """
    beta = list(beta)
    xy = {}
    for k in output_harmonics:
        try:
            position = list(fit_harmonics).index(k)
        except ValueError:
            raise ValueError(f"output harmonic {k} is not in the fitting list") from None
        a, b = beta[1 + 2 * position], beta[2 + 2 * position]
        xy[str(k)] = {"x": float(b / 2.0), "y": float(a / 2.0)}
    return xy


def covariance_pinv_reference(design_w, sigma2=1.0):
    """Reference covariance sigma2 * pinv(Dw) pinv(Dw)^T via SVD. Oracle only.

    Estimation (gelsd/gelsy truncation) and this pinv helper may apply
    different rank policies on ill-conditioned designs; reconcile one policy
    before using the oracle on M2 rank/conditioning fixtures. A truncated
    zero covariance entry is not precise knowledge of an unidentifiable
    coefficient.
    """
    pinv = np.linalg.pinv(design_w)
    return sigma2 * pinv @ pinv.T


def solve_case(times, signal, f_ref, phase_rad, fit_harmonics, output_harmonics,
               noise=None, driver="gelsd"):
    """Vector-response convenience wrapper. Matrix responses are rejected:
    use whiten_* plus solve_whitened directly for batched solves."""
    if np.ndim(signal) != 1:
        raise ValueError("solve_case accepts a single response vector; use solve_whitened for batches")
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
    result["xy"] = map_to_xy(result["beta"], fit_harmonics, output_harmonics)
    result["whitening"] = info
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
