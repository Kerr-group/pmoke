#!/usr/bin/env python3
"""Compare pmoke performance JSON reports and write a Markdown summary."""

from __future__ import annotations

import argparse
import json
import math
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Measurement:
    name: str
    channels: int
    seconds: float
    samples_per_second: float

    @property
    def key(self) -> tuple[str, int]:
        return self.name, self.channels


def load_measurements(root: Path) -> dict[tuple[str, int], Measurement]:
    measurements: dict[tuple[str, int], Measurement] = {}
    if not root.exists():
        return measurements

    for path in sorted(root.rglob("results*.json")):
        try:
            report = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            print(f"::warning title=Performance report skipped::{path}: {error}")
            continue

        for result in report.get("results", []):
            try:
                name = str(result["name"])
                channels = (
                    int(report.get("raw_csv_channels", 0))
                    if name in {"raw_waveform_read", "raw_to_csv"}
                    else 0
                )
                measurement = Measurement(
                    name=name,
                    channels=channels,
                    seconds=float(result["median_seconds"]),
                    samples_per_second=float(result["samples_per_second"]),
                )
            except (KeyError, TypeError, ValueError) as error:
                print(f"::warning title=Performance result skipped::{path}: {error}")
                continue
            if not math.isfinite(measurement.seconds) or measurement.seconds < 0.0:
                print(
                    "::warning title=Performance result skipped::"
                    f"{path}: invalid median_seconds for {measurement.name}"
                )
                continue
            # Older matrix runs repeated channel-independent cases in both
            # artifacts. Keep the first deterministic result in that case.
            measurements.setdefault(measurement.key, measurement)

    return measurements


def render_summary(
    current: dict[tuple[str, int], Measurement],
    previous: dict[tuple[str, int], Measurement],
    warning_threshold: float,
) -> str:
    lines = [
        "## Performance comparison",
        "",
        "Shared-runner timings are informational and do not gate the workflow.",
        "",
    ]
    if not current:
        lines.append("No completed performance reports were produced in this job.")
        return "\n".join(lines) + "\n"

    lines.extend(
        [
        "| Case | RAW channels | Current | Previous | Change |",
            "|---|---:|---:|---:|---:|",
        ]
    )
    for key in sorted(current):
        value = current[key]
        baseline = previous.get(key)
        if baseline is None or baseline.seconds == 0.0:
            previous_text = "—"
            change_text = "new"
        else:
            change = (value.seconds / baseline.seconds - 1.0) * 100.0
            previous_text = f"{baseline.seconds:.6f} s"
            change_text = f"{change:+.1f}%"
            if change > warning_threshold:
                channel_label = f" ({value.channels}ch)" if value.channels else ""
                print(
                    f"::warning title=Performance regression::{value.name}{channel_label} "
                    f"is {change:.1f}% slower than the previous successful run"
                )
        channels_text = str(value.channels) if value.channels else "—"
        lines.append(
            f"| `{value.name}` | {channels_text} | {value.seconds:.6f} s | "
            f"{previous_text} | {change_text} |"
        )

    if not previous:
        lines.extend(["", "No previous successful-run artifact was available."])
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--current", type=Path, required=True)
    parser.add_argument("--previous", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--warning-threshold", type=float, default=30.0)
    args = parser.parse_args()

    current = load_measurements(args.current)
    previous = load_measurements(args.previous)
    summary = render_summary(current, previous, args.warning_threshold)
    with args.summary.open("a", encoding="utf-8") as output:
        output.write(summary)


if __name__ == "__main__":
    main()
