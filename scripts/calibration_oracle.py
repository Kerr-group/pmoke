"""Independent NumPy reference for the NUMERICS section 6 calibration recipe.

Mirrors the fixed v1 recipe (nuisance regression, phase variance, tapered
correlation, SCS adequacy) with straightforward NumPy code written
independently of the Rust implementation. Committed outputs ground the Rust
differential tests; agreement is evidence, not proof.
"""

import math

import numpy as np

TAU = 2.0 * math.pi
NUISANCE_HARMONICS = 12


def nuisance_design(times, f_ref, phase_rad):
    times = np.asarray(times, dtype=float)
    first, last = times[0], times[-1]
    mid = 0.5 * (first + last)
    half = 0.5 * (last - first)
    columns = [np.ones_like(times), (times - mid) / half]
    phi = TAU * f_ref * times - phase_rad
    for k in range(1, NUISANCE_HARMONICS + 1):
        columns.append(np.cos(k * phi))
        columns.append(np.sin(k * phi))
    return np.column_stack(columns)


def nuisance_fit(times, signal, f_ref, phase_rad):
    design = nuisance_design(times, f_ref, phase_rad)
    beta, _, rank, _ = np.linalg.lstsq(design, np.asarray(signal, dtype=float), rcond=None)
    residual = np.asarray(signal, dtype=float) - design @ beta
    singular = np.linalg.svd(design, compute_uv=False)
    condition = float(singular[0] / singular[-1]) if singular[-1] > 0 else math.inf
    return {"coefficients": beta.tolist(), "rank": int(rank),
            "condition": condition, "residual": residual.tolist()}


def assemble_samples(block_times, block_residuals, f_ref, phase_rad):
    samples = []
    for block, (times, residuals) in enumerate(zip(block_times, block_residuals)):
        for t, r in zip(times, residuals):
            phase = (TAU * f_ref * t - phase_rad) % TAU
            samples.append({"phase": phase, "cycle": math.floor(f_ref * t),
                            "residual": r, "block": block})
    return samples


def phase_variance(samples, bins, min_samples_per_bin=256, min_cycles_per_bin=128,
                   min_contributing_blocks=8, shrinkage_alpha=0.1, floor_ratio=0.05):
    counts = [0] * bins
    sums = [0.0] * bins
    squares = [0.0] * bins
    cycles = [set() for _ in range(bins)]
    blocks = set()
    for sample in samples:
        b = int(math.floor(sample["phase"] / TAU * bins)) % bins
        counts[b] += 1
        sums[b] += sample["residual"]
        squares[b] += sample["residual"] ** 2
        cycles[b].add(sample["cycle"])
        blocks.add(sample["block"])
    if len(blocks) < min_contributing_blocks:
        raise ValueError("insufficient contributing blocks")
    raw = []
    for b in range(bins):
        if counts[b] < min_samples_per_bin or len(cycles[b]) < min_cycles_per_bin:
            raise ValueError(f"insufficient coverage in bin {b}")
        n = counts[b]
        var = (squares[b] - sums[b] ** 2 / n) / (n - 1)
        if not var > 0.0 or not math.isfinite(var):
            raise ValueError(f"nonpositive raw variance in bin {b}")
        raw.append(var)
    total = sum(counts)
    v0 = sum(c * v for c, v in zip(counts, raw)) / total
    smoothed = [(raw[(b - 1) % bins] + 2.0 * raw[b] + raw[(b + 1) % bins]) / 4.0
                for b in range(bins)]
    floor = floor_ratio * v0
    variances = []
    floor_hits = []
    for b in range(bins):
        shrunk = (1.0 - shrinkage_alpha) * smoothed[b] + shrinkage_alpha * v0
        if shrunk < floor:
            variances.append(floor)
            floor_hits.append(b)
        else:
            variances.append(shrunk)
    if len(floor_hits) * 4 > bins:
        raise ValueError("excessive floor activation")
    return {"variances": variances, "v0": v0, "counts": counts,
            "distinct_cycles": [len(c) for c in cycles],
            "contributing_blocks": len(blocks), "raw": raw,
            "smoothed": smoothed, "floor_activations": floor_hits}


def correlation(blocks, weights, lag_step_s, max_lag, eta=0.05):
    blocks = [np.asarray(b, dtype=float) for b in blocks]
    if weights is None:
        total = sum(len(b) for b in blocks)
        weights = [len(b) / total for b in blocks]
    pooled = np.zeros(max_lag + 1)
    for block, weight in zip(blocks, weights):
        centered = block - block.mean()
        auto = np.array([float(np.dot(centered[:len(block) - lag], centered[lag:]) / len(block))
                         for lag in range(max_lag + 1)])
        if not auto[0] > 0.0:
            raise ValueError("correlation block has no lag-zero power")
        pooled += weight * auto / auto[0]
    tapered = np.array([(1.0 - eta) * pooled[lag] * (1.0 - lag / (max_lag + 1))
                        + eta * (1.0 if lag == 0 else 0.0)
                        for lag in range(max_lag + 1)])
    tail = None
    max_tail = max_lag
    shortest = min(len(b) for b in blocks)
    for lag in range(max_lag + 1, min(2 * max_lag, shortest - 1) + 1):
        pooled_tail = 0.0
        for block, weight in zip(blocks, weights):
            centered = block - block.mean()
            num = float(np.dot(centered[:len(block) - lag], centered[lag:]) / len(block))
            den = float(np.dot(centered, centered) / len(block))
            if den > 0.0:
                pooled_tail += weight * num / den
        tail = max(tail or 0.0, abs(pooled_tail))
        max_tail = lag
    return {"lags": tapered.tolist(), "lag_step_s": lag_step_s,
            "support_samples": max_lag + 1,
            "physical_duration_s": max_lag * lag_step_s,
            "taper_id": "bartlett_v1", "eta": eta,
            "tail_energy": tail, "max_tail_lag": max_tail}


def scs_adequacy(train_groups, reserved_groups, lags, min_pairs_per_cell,
                 max_phase_spread, max_reserved_shift, octants=8):
    def pairs(groups, lag):
        out = []
        for group in groups:
            x = np.asarray(group["standardized"], dtype=float)
            p = np.asarray(group["phases"], dtype=float)
            for i in range(len(x) - lag):
                out.append((x[i], x[i + lag], p[i] % TAU))
        return out

    train_pooled = np.zeros(lags)
    reserved_pooled = np.zeros(lags)
    train_n = np.zeros(lags, dtype=int)
    reserved_n = np.zeros(lags, dtype=int)
    oct_num = np.zeros((lags, octants))
    oct_den = np.zeros((lags, octants), dtype=int)
    for role, groups in ((0, train_groups), (1, reserved_groups)):
        for lag in range(1, lags + 1):
            for first, second, phase in pairs(groups, lag):
                octant = min(int(math.floor(phase / TAU * octants)), octants - 1)
                if role == 0:
                    train_pooled[lag - 1] += first * second
                    train_n[lag - 1] += 1
                    oct_num[lag - 1, octant] += first * second
                    oct_den[lag - 1, octant] += 1
                else:
                    reserved_pooled[lag - 1] += first * second
                    reserved_n[lag - 1] += 1
    train_pooled /= train_n
    reserved_pooled /= reserved_n
    spread = 0.0
    for lag in range(lags):
        if np.any(oct_den[lag] < min_pairs_per_cell):
            raise ValueError("insufficient octant pairs")
        cell = oct_num[lag] / oct_den[lag]
        spread = max(spread, float(cell.max() - cell.min()))
    shift = float(np.max(np.abs(train_pooled - reserved_pooled)))
    if spread > max_phase_spread:
        return {"adequate": False, "phase_spread": spread,
                "reserved_shift": shift, "reason": "phase_spread"}
    if shift > max_reserved_shift:
        return {"adequate": False, "phase_spread": spread,
                "reserved_shift": shift, "reason": "reserved_shift"}
    return {"adequate": True, "phase_spread": spread,
            "reserved_shift": shift, "reason": "within_policy"}
