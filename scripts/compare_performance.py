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
    samples: int
    seconds: float
    samples_per_second: float
    max_rss_kib: int | None

    @property
    def key(self) -> tuple[str, int, int]:
        return self.name, self.channels, self.samples


def load_max_rss_kib(path: Path) -> int | None:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        print(f"::warning title=Performance resources skipped::{path}: {error}")
        return None

    prefix = "Maximum resident set size (kbytes):"
    for line in lines:
        stripped = line.strip()
        if stripped.startswith(prefix):
            try:
                value = int(stripped.removeprefix(prefix).strip())
            except ValueError:
                break
            if value >= 0:
                return value
            break

    print(f"::warning title=Performance resources skipped::{path}: maximum RSS not found")
    return None


def resource_path_for_report(path: Path, report: dict, result_count: int) -> Path | None:
    if result_count != 1:
        return None

    case = report.get("case")
    if isinstance(case, str) and case and case != "all":
        candidate = path.with_name(f"resources-{case}.txt")
        if candidate.exists():
            return candidate

    prefix = "results-"
    if path.stem.startswith(prefix):
        candidate = path.with_name(f"resources-{path.stem.removeprefix(prefix)}.txt")
        if candidate.exists():
            return candidate

    return None


def load_measurements(root: Path) -> dict[tuple[str, int, int], Measurement]:
    measurements: dict[tuple[str, int, int], Measurement] = {}
    if not root.exists():
        return measurements

    for path in sorted(root.rglob("results*.json")):
        try:
            report = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            print(f"::warning title=Performance report skipped::{path}: {error}")
            continue

        results = report.get("results", [])
        if not isinstance(results, list):
            print(f"::warning title=Performance report skipped::{path}: results must be a list")
            continue
        resource_path = resource_path_for_report(path, report, len(results))
        max_rss_kib = load_max_rss_kib(resource_path) if resource_path else None

        for result in results:
            try:
                name = str(result["name"])
                samples = int(report["samples"])
                channels = (
                    int(report.get("raw_csv_channels", 0))
                    if name in {"raw_waveform_read", "raw_to_csv"}
                    else 0
                )
                measurement = Measurement(
                    name=name,
                    channels=channels,
                    samples=samples,
                    seconds=float(result["median_seconds"]),
                    samples_per_second=float(result["samples_per_second"]),
                    max_rss_kib=max_rss_kib,
                )
            except (KeyError, TypeError, ValueError) as error:
                print(f"::warning title=Performance result skipped::{path}: {error}")
                continue
            if measurement.samples <= 0:
                print(
                    "::warning title=Performance result skipped::"
                    f"{path}: samples must be positive for {measurement.name}"
                )
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
    current: dict[tuple[str, int, int], Measurement],
    previous: dict[tuple[str, int, int], Measurement],
    warning_threshold: float,
    memory_warning_threshold: float,
) -> str:
    lines = [
        "## Performance comparison",
        "",
        "Shared-runner measurements are informational and do not gate the workflow.",
        "",
    ]
    if not current:
        lines.append("No completed performance reports were produced in this job.")
        return "\n".join(lines) + "\n"

    lines.extend(
        [
            "| Case | Samples | RAW channels | Current | Previous | Change "
            "| Current RSS | Previous RSS | RSS change |",
            "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
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
        if value.max_rss_kib is None:
            current_rss_text = "—"
            previous_rss_text = "—"
            rss_change_text = "—"
        else:
            current_rss_text = f"{value.max_rss_kib / 1024.0:.1f} MiB"
            if baseline is None or baseline.max_rss_kib in {None, 0}:
                previous_rss_text = "—"
                rss_change_text = "new"
            else:
                rss_change = (value.max_rss_kib / baseline.max_rss_kib - 1.0) * 100.0
                previous_rss_text = f"{baseline.max_rss_kib / 1024.0:.1f} MiB"
                rss_change_text = f"{rss_change:+.1f}%"
                if rss_change > memory_warning_threshold:
                    channel_label = f" ({value.channels}ch)" if value.channels else ""
                    print(
                        f"::warning title=Performance memory regression::{value.name}{channel_label} "
                        f"uses {rss_change:.1f}% more peak RSS than the previous successful run"
                    )
        channels_text = str(value.channels) if value.channels else "—"
        lines.append(
            f"| `{value.name}` | {value.samples:,} | {channels_text} | "
            f"{value.seconds:.6f} s | {previous_text} | {change_text} | "
            f"{current_rss_text} | {previous_rss_text} | {rss_change_text} |"
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
    parser.add_argument("--memory-warning-threshold", type=float, default=30.0)
    args = parser.parse_args()

    current = load_measurements(args.current)
    previous = load_measurements(args.previous)
    summary = render_summary(
        current,
        previous,
        args.warning_threshold,
        args.memory_warning_threshold,
    )
    with args.summary.open("a", encoding="utf-8") as output:
        output.write(summary)


if __name__ == "__main__":
    main()
