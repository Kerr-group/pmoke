"""Deterministic public fixtures for fixed-support joint harmonic estimation.

Generates synthetic waveforms, geometry goldens, known-noise families, and
legacy boxcar goldens under crates/pmoke-analysis-core/tests/fixtures/joint-gls/
with a sha256 manifest. Uses only NumPy with fixed RandomState seeds, fully
independent of the Rust implementation and of private measurement data.

The legacy boxcar reimplementation below mirrors
crates/pmoke-analysis-core/src/lockin.rs (rolling mixed window, trapezoidal
edges, oscillator resync) so the frozen goldens are independent evidence for
FR-052, not copies of Rust outputs.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUTPUT = REPO_ROOT / "crates" / "pmoke-analysis-core" / "tests" / "fixtures" / "joint-gls"
RESYNC_INTERVAL = 4096


def periodic_interp(phases, centers, values):
    """Periodic piecewise-linear interpolation at declared bin centers.

    NUMERICS section 4.1: uniform centers phi_b = 2 pi (b + 0.5) / B with
    last-to-first wrap. Interpolates variance, never standard deviation.
    """
    period = 2.0 * math.pi
    count = len(centers)
    step = period / count
    u = (np.asarray(phases, dtype=float) % period) / step - 0.5
    below = np.floor(u).astype(int) % count
    frac = u - np.floor(u)
    table = np.asarray(values, dtype=float)
    return (1.0 - frac) * table[below] + frac * table[(below + 1) % count]


def legacy_geometry(length, dt, stride, f_ref, cycles):
    """NUMERICS section 1 grid. Returns dict or {'expected_error': code}.

    Error codes match AnalysisError codes in
    crates/pmoke-analysis-core/src/lockin.rs.
    """
    half_window_s = cycles / f_ref
    if length < 2:
        return {"expected_error": "signal_too_short"}
    if stride == 0:
        return {"expected_error": "invalid_stride"}
    n_half = max(int(math.floor(half_window_s / dt)), 1)
    if half_window_s < dt:
        return {"expected_error": "window_too_short"}
    integration_points = (length - 1) // stride + 1
    i_start = 2 + (n_half + 1) // stride
    i_end = integration_points - i_start
    if i_end < i_start:
        return {"expected_error": "signal_too_short"}
    first_center = i_start * stride
    last_center = i_end * stride
    if first_center <= n_half or last_center + n_half + 1 >= length:
        return {"expected_error": "window_out_of_range"}
    return {
        "half_window_s": half_window_s,
        "n_half": n_half,
        "integration_points": integration_points,
        "i_start": i_start,
        "i_end": i_end,
        "row_count": i_end - i_start + 1,
        "first_center": first_center,
        "last_center": last_center,
    }


def legacy_boxcar(signal, t0, dt, f_ref, phase_rad, cycles, stride, harmonic):
    """Independent legacy boxcar for harmonic k. Returns (times, x, y, meta)."""
    n = len(signal)
    omega = 2.0 * math.pi * f_ref
    half_window_s = cycles / f_ref
    n_half = max(int(math.floor(half_window_s / dt)), 1)
    integration_points = (n - 1) // stride + 1
    i_start = 2 + (n_half + 1) // stride
    i_end = integration_points - i_start
    scale = 1.0 / (2.0 * half_window_s)
    edge_dt = half_window_s - n_half * dt
    step_phase = -harmonic * omega * dt
    step_sin = math.sin(step_phase)
    step_cos = math.cos(step_phase)
    phase_zero = -harmonic * (omega * t0 - phase_rad)
    window_len = 2 * n_half + 3
    first_center = i_start * stride
    last_center = i_end * stride
    raw_start = first_center - n_half - 1
    raw_end = last_center + n_half + 2
    anchor = raw_start - raw_start % RESYNC_INTERVAL
    osc_sin, osc_cos = math.sin(phase_zero + anchor * step_phase), math.cos(
        phase_zero + anchor * step_phase
    )
    for _ in range(anchor, raw_start):
        osc_cos, osc_sin = (
            osc_cos * step_cos - osc_sin * step_sin,
            osc_cos * step_sin + osc_sin * step_cos,
        )
    window = [(0.0, 0.0)] * window_len
    times, xs, ys = [], [], []
    next_emit = first_center + n_half + 1
    for idx in range(raw_start, raw_end):
        if idx > 0 and idx % RESYNC_INTERVAL == 0:
            osc_sin, osc_cos = math.sin(phase_zero + idx * step_phase), math.cos(
                phase_zero + idx * step_phase
            )
        mixed = (signal[idx] * osc_cos, signal[idx] * osc_sin)
        window.append(mixed)
        window.pop(0)
        osc_cos, osc_sin = (
            osc_cos * step_cos - osc_sin * step_sin,
            osc_cos * step_sin + osc_sin * step_cos,
        )
        if idx != next_emit:
            continue
        outer_neg, inner_neg = window[0], window[1]
        inner_pos, outer_pos = window[-2], window[-1]
        sum_re = sum(v[0] for v in window)
        sum_im = sum(v[1] for v in window)

        def edge(y0, y1):
            interp = (y1 * edge_dt + y0 * (dt - edge_dt)) / dt
            return edge_dt * 0.5 * (y0 + interp)

        int_re = (
            sum_re - outer_neg[0] - outer_pos[0] - 0.5 * inner_neg[0] - 0.5 * inner_pos[0]
        ) * dt
        int_im = (
            sum_im - outer_neg[1] - outer_pos[1] - 0.5 * inner_neg[1] - 0.5 * inner_pos[1]
        ) * dt
        xs.append(
            -(
                int_im
                + edge(inner_neg[1], outer_neg[1])
                + edge(inner_pos[1], outer_pos[1])
            )
            * scale
        )
        ys.append(
            (
                int_re
                + edge(inner_neg[0], outer_neg[0])
                + edge(inner_pos[0], outer_pos[0])
            )
            * scale
        )
        times.append(t0 + (next_emit - n_half - 1) * dt)
        next_emit += stride
    meta = {
        "input_samples": n,
        "output_samples": len(xs),
        "first_input_index": first_center,
        "last_input_index": first_center + (len(xs) - 1) * stride,
    }
    return times, xs, ys, meta


def mixture_value(t, f_ref, phase_rad, dc, coeffs):
    """y = dc + sum a_k cos(k phi) + b_k sin(k phi), phi = 2 pi f t - phase."""
    phi = 2.0 * math.pi * f_ref * t - phase_rad
    value = dc
    for k, (a, b) in coeffs.items():
        value += a * math.cos(k * phi) + b * math.sin(k * phi)
    return value


def build_geometry_cases():
    return [
        {"name": "integral_window_stride20", "length": 20000, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "fractional_window_stride100", "length": 20000, "dt": 1e-5,
         "stride": 100, "f_ref": 999.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "stride1_fine", "length": 3000, "dt": 1e-5,
         "stride": 1, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "nonzero_t_start", "length": 20000, "dt": 1e-5,
         "stride": 100, "f_ref": 1000.0, "cycles": 1.0, "t_start": -0.028},
        {"name": "wide_window", "length": 20000, "dt": 1e-5,
         "stride": 50, "f_ref": 1000.0, "cycles": 2.5, "t_start": 0.0},
        {"name": "nine_rows_421", "length": 421, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "short_two_rows", "length": 300, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "boundary_invalid_260", "length": 260, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "boundary_single_row_261", "length": 261, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "incomplete_window", "length": 200, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "single_sample", "length": 1, "dt": 1e-5,
         "stride": 20, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "zero_stride", "length": 20000, "dt": 1e-5,
         "stride": 0, "f_ref": 1000.0, "cycles": 1.0, "t_start": 0.0},
        {"name": "subsample_window", "length": 20000, "dt": 1e-5,
         "stride": 20, "f_ref": 2000000.0, "cycles": 1.0, "t_start": 0.0},
    ]


def build_convention_cases():
    base = {"f_ref": 100000.0, "dt": 1e-7, "samples": 1024,
            "fit_harmonics": list(range(1, 13)), "output_harmonics": list(range(1, 7))}
    cases = []
    cases.append(dict(base, name="mixed_phases_fractional_t0", t_start=3.7e-6,
                      phase_rad=0.7, dc=0.05,
                      coeffs={1: (0.0, 1.0), 2: (0.5, 0.0), 3: (0.0, 0.02),
                              4: (-2.0, 1.5), 7: (0.3, -0.2), 12: (0.0, 0.1)}))
    cases.append(dict(base, name="large_even_weak_odd", t_start=0.0,
                      phase_rad=-1.3, dc=-3.0,
                      coeffs={1: (0.0, 0.001), 2: (4.0, -3.0), 5: (0.002, 0.0),
                              6: (1.0, 1.0), 8: (-0.5, 0.4), 11: (0.05, -0.05)}))
    cases.append(dict(base, name="negative_fractional_t0", t_start=-1.3e-6,
                      phase_rad=2.6, dc=0.0,
                      coeffs={1: (0.7, -0.7), 3: (-0.1, 0.0), 6: (0.0, 0.4),
                              9: (0.2, 0.2), 12: (-0.15, 0.0)}))
    return cases


def expected_beta(dc, coeffs, fit_harmonics):
    beta = [dc]
    for k in fit_harmonics:
        a, b = coeffs.get(k, (0.0, 0.0))
        beta += [a, b]
    return beta


def build_noise_families():
    """Each family owns its declared RandomState seed: regenerating one
    family from its seed reproduces its arrays independent of the others."""
    families = {}
    n = 2048
    rng = np.random.RandomState(20260913)
    g = rng.normal(0.0, 2.0, n)
    families["iid_gaussian"] = {
        "kind": "iid_gaussian", "seed": 20260913, "samples": n,
        "sigma": 2.0, "values": g.tolist(),
        "sample_mean": float(np.mean(g)), "sample_var": float(np.var(g)),
    }
    rng = np.random.RandomState(20260914)
    u = rng.uniform(-1.0, 1.0, n) * math.sqrt(3.0)
    families["iid_uniform"] = {
        "kind": "iid_uniform", "seed": 20260914, "samples": n,
        "values": u.tolist(),
        "sample_mean": float(np.mean(u)), "sample_var": float(np.var(u)),
    }
    rng = np.random.RandomState(20260915)
    rho = 0.7
    ar = np.empty(n)
    ar[0] = rng.normal()
    for i in range(1, n):
        ar[i] = rho * ar[i - 1] + math.sqrt(1.0 - rho * rho) * rng.normal()
    families["ar1_correlated"] = {
        "kind": "ar1", "seed": 20260915, "samples": n, "rho": rho,
        "values": ar.tolist(), "sample_mean": float(np.mean(ar)),
        "sample_var": float(np.var(ar)), "lag1_corr": float(np.corrcoef(ar[:-1], ar[1:])[0, 1]),
    }
    rng = np.random.RandomState(20260916)
    bins = 64
    centers = 2.0 * math.pi * (np.arange(bins) + 0.5) / bins
    # Dimensionless multiplier profile; absolute variance is v0 * profile.
    profile = 1.0 + 0.5 * np.sin(2.0 * math.pi * (np.arange(bins) + 0.5) / bins)
    v0 = 2.25
    phases = rng.uniform(0.0, 2.0 * math.pi, n)
    variances = v0 * periodic_interp(phases, centers, profile)
    hetero = np.sqrt(variances) * rng.normal(0.0, 1.0, n)
    families["phase_heteroscedastic"] = {
        "kind": "phase_diagonal", "seed": 20260916, "samples": n,
        "bins": bins, "v0": v0,
        "profile_kind": "dimensionless_multiplier",
        "variance_profile": profile.tolist(),
        "phases": phases.tolist(), "variances": variances.tolist(),
        "values": hetero.tolist(), "sample_mean": float(np.mean(hetero)),
        "sample_var": float(np.var(hetero)),
        "variance_mean": float(np.mean(variances)),
    }
    return families


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    out: Path = args.output
    out.mkdir(parents=True, exist_ok=True)
    files = {}

    geometry = []
    for case in build_geometry_cases():
        grid = legacy_geometry(case["length"], case["dt"], case["stride"],
                               case["f_ref"], case["cycles"])
        entry = dict(case)
        entry.update(grid)
        if "expected_error" not in grid:
            dt, t0 = case["dt"], case["t_start"]
            entry["first_time"] = t0 + grid["first_center"] * dt
            entry["last_time"] = t0 + grid["last_center"] * dt
            entry["first_support"] = [grid["first_center"] - grid["n_half"] - 1,
                                      grid["first_center"] + grid["n_half"] + 1]
            entry["last_support"] = [grid["last_center"] - grid["n_half"] - 1,
                                     grid["last_center"] + grid["n_half"] + 1]
        geometry.append(entry)
    files["geometry.json"] = {"schema_version": 1, "cases": geometry}

    conventions = []
    for case in build_convention_cases():
        t = case["t_start"] + np.arange(case["samples"]) * case["dt"]
        y = np.array([mixture_value(ti, case["f_ref"], case["phase_rad"],
                                    case["dc"], case["coeffs"]) for ti in t])
        beta = expected_beta(case["dc"], case["coeffs"], case["fit_harmonics"])
        xy = {}
        for k in case["output_harmonics"]:
            a, b = case["coeffs"].get(k, (0.0, 0.0))
            xy[str(k)] = {"x": b / 2.0, "y": a / 2.0}
        conventions.append({
            "name": case["name"], "f_ref": case["f_ref"], "dt": case["dt"],
            "t_start": case["t_start"], "samples": case["samples"],
            "phase_rad": case["phase_rad"], "dc": case["dc"],
            "fit_harmonics": case["fit_harmonics"],
            "output_harmonics": case["output_harmonics"],
            "coeffs": {str(k): list(v) for k, v in case["coeffs"].items()},
            "signal": y.tolist(), "expected_beta": beta, "expected_xy": xy,
        })
    files["conventions.json"] = {"schema_version": 1, "cases": conventions}

    files["noise.json"] = {"schema_version": 1, "families": build_noise_families()}

    golden = []
    for name, params in [
        ("harmonic_mixture_h1", {"f_ref": 1000.0, "dt": 1e-5, "t0": 0.0,
                                 "phase": 0.2, "cycles": 1.0, "stride": 50,
                                 "harmonic": 1, "samples": 3000}),
        ("harmonic_mixture_h2_fractional_t0", {"f_ref": 1000.0, "dt": 1e-5,
                                               "t0": 0.00137, "phase": -0.9,
                                               "cycles": 1.0, "stride": 50,
                                               "harmonic": 2, "samples": 3000}),
        ("fractional_edge_window", {"f_ref": 999.0, "dt": 1e-5, "t0": 0.00137,
                                    "phase": 0.5, "cycles": 1.0, "stride": 50,
                                    "harmonic": 1, "samples": 6000}),
    ]:
        t = params["t0"] + np.arange(params["samples"]) * params["dt"]
        phi = 2.0 * math.pi * params["f_ref"] * t - params["phase"]
        y = (1.5 * np.sin(phi) + 0.4 * np.cos(2 * phi)
             - 0.8 * np.sin(2 * phi) + 0.1).tolist()
        times, xs, ys, meta = legacy_boxcar(
            y, params["t0"], params["dt"], params["f_ref"], params["phase"],
            params["cycles"], params["stride"], params["harmonic"])
        golden.append({"name": name, "params": params, "signal": y,
                       "times": times, "x": xs, "y": ys, "meta": meta})
    files["legacy-golden.json"] = {"schema_version": 1, "cases": golden}

    manifest = {
        "schema_version": 1,
        "license": "CC0-1.0",
        "generator": "scripts/generate_joint_gls_fixtures.py",
        "numpy_version": np.__version__,
        "files": {},
    }
    for name, payload in files.items():
        path = out / name
        path.write_text(json.dumps(payload, indent=1) + "\n", encoding="utf-8")
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest["files"][name] = {"sha256": digest, "bytes": path.stat().st_size}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n",
                                        encoding="utf-8")
    print(f"wrote {len(files)} fixture files + manifest to {out}")


if __name__ == "__main__":
    main()
