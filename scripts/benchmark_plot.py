#!/usr/bin/env python3
"""Record matplotlib rendering costs without using them as CI pass/fail gates."""

from __future__ import annotations

import argparse
import json
import math
import platform
import statistics
import time
from pathlib import Path

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
import numpy as np


def summarize(name: str, samples: int, durations: list[float]) -> dict:
    median = statistics.median(durations)
    deviations = [abs(value - median) for value in durations]
    ordered = sorted(durations)
    p95_index = max(0, math.ceil(len(ordered) * 0.95) - 1)
    return {
        "name": name,
        "samples": samples,
        "iterations": len(durations),
        "median_seconds": median,
        "mad_seconds": statistics.median(deviations),
        "p95_seconds": ordered[p95_index],
    }


def measure(operation, iterations: int) -> list[float]:
    operation()
    durations = []
    for _ in range(iterations):
        start = time.perf_counter_ns()
        operation()
        durations.append((time.perf_counter_ns() - start) / 1e9)
    return durations


class DrawFixture:
    def __init__(self, samples: int):
        self.samples = samples
        self.x = np.linspace(0.0, 1.0, samples, dtype=np.float64)
        self.y = np.sin(2.0 * np.pi * 17.0 * self.x)
        self.figure, self.axes = plt.subplots()
        (self.line,) = self.axes.plot(self.x, self.y)
        self.figure.canvas.draw()
        self.background = self.figure.canvas.copy_from_bbox(self.axes.bbox)
        self.phase = 0.0

    def close(self):
        plt.close(self.figure)

    def full_draw(self):
        self.figure.canvas.draw()

    def set_data_draw_idle(self):
        self.phase += 1e-4
        self.line.set_ydata(self.y + self.phase)
        self.figure.canvas.draw_idle()

    def blit(self):
        self.phase += 1e-4
        self.line.set_ydata(self.y + self.phase)
        self.figure.canvas.restore_region(self.background)
        self.axes.draw_artist(self.line)
        self.figure.canvas.blit(self.axes.bbox)


def benchmark(sample_counts: list[int], iterations: int) -> dict:
    results = []

    def figure_init():
        figure, _ = plt.subplots()
        figure.canvas.draw()
        plt.close(figure)

    results.append(
        summarize("matplotlib_figure_init", 0, measure(figure_init, iterations))
    )
    for samples in sample_counts:
        fixture = DrawFixture(samples)
        try:
            if samples == 1_000_000:
                suffix = "1m"
            elif samples >= 1_000 and samples % 1_000 == 0:
                suffix = f"{samples // 1_000}k"
            else:
                suffix = str(samples)
            results.append(
                summarize(
                    f"matplotlib_full_draw_{suffix}",
                    samples,
                    measure(fixture.full_draw, iterations),
                )
            )
            results.append(
                summarize(
                    f"matplotlib_set_data_draw_idle_{suffix}",
                    samples,
                    measure(fixture.set_data_draw_idle, iterations),
                )
            )
            results.append(
                summarize(
                    f"matplotlib_blit_{suffix}",
                    samples,
                    measure(fixture.blit, iterations),
                )
            )
        finally:
            fixture.close()

    return {
        "schema_version": 1,
        "backend": matplotlib.get_backend(),
        "matplotlib": matplotlib.__version__,
        "numpy": np.__version__,
        "python": platform.python_version(),
        "os": platform.system().lower(),
        "arch": platform.machine(),
        "results": results,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--samples",
        default="100000,1000000",
        help="comma-separated sample counts",
    )
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    sample_counts = [int(value) for value in args.samples.split(",")]
    if not sample_counts or any(value <= 0 for value in sample_counts):
        parser.error("--samples values must be positive")
    if args.iterations <= 0:
        parser.error("--iterations must be positive")

    report = benchmark(sample_counts, args.iterations)
    encoded = json.dumps(report, indent=2) + "\n"
    print(encoded, end="")
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")


if __name__ == "__main__":
    main()
