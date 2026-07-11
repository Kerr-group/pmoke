import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from compare_performance import (
    Measurement,
    load_max_rss_kib,
    load_measurements,
    render_summary,
)


class ComparePerformanceTests(unittest.TestCase):
    def test_load_distinguishes_only_raw_channel_sensitive_cases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = {
                "case": "raw_waveform_read",
                "samples": 10,
                "raw_csv_channels": 4,
                "results": [
                    {
                        "name": "raw_waveform_read",
                        "median_seconds": 2.0,
                        "samples_per_second": 5.0,
                    }
                ],
            }
            (root / "results-raw_waveform_read.json").write_text(
                json.dumps(report), encoding="utf-8"
            )
            (root / "resources-raw_waveform_read.txt").write_text(
                "Maximum resident set size (kbytes): 2048\n", encoding="utf-8"
            )
            (root / "results-lockin_w1.json").write_text(
                json.dumps(
                    {
                        "case": "lockin_w1",
                        "samples": 10,
                        "raw_csv_channels": 4,
                        "results": [
                            {
                                "name": "lockin_w1",
                                "median_seconds": 1.0,
                                "samples_per_second": 10.0,
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            measurements = load_measurements(root)

            self.assertIn(("raw_waveform_read", 4, 10), measurements)
            self.assertIn(("lockin_w1", 0, 10), measurements)
            self.assertEqual(
                measurements[("raw_waveform_read", 4, 10)].max_rss_kib, 2048
            )

    def test_summary_warns_without_failing_on_large_regression(self):
        current = {
            ("lockin_w1", 0, 100): Measurement(
                "lockin_w1", 0, 100, 1.5, 10.0, 1536
            ),
        }
        previous = {
            ("lockin_w1", 0, 100): Measurement(
                "lockin_w1", 0, 100, 1.0, 15.0, 1024
            ),
        }
        warnings = io.StringIO()

        with contextlib.redirect_stdout(warnings):
            summary = render_summary(current, previous, 30.0, 30.0)

        self.assertIn("+50.0%", summary)
        self.assertIn("::warning title=Performance regression::", warnings.getvalue())
        self.assertIn(
            "::warning title=Performance memory regression::", warnings.getvalue()
        )

    def test_different_sample_counts_are_not_compared(self):
        current = {
            ("raw_word_decode", 0, 50_000_000): Measurement(
                "raw_word_decode", 0, 50_000_000, 50.0, 1_000_000.0, None
            ),
        }
        previous = {
            ("raw_word_decode", 0, 1_000_000): Measurement(
                "raw_word_decode", 0, 1_000_000, 1.0, 1_000_000.0, None
            ),
        }

        summary = render_summary(current, previous, 30.0, 30.0)

        self.assertIn("| new |", summary)
        self.assertNotIn("+4900.0%", summary)

    def test_loads_gnu_time_maximum_rss(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "resources.txt"
            path.write_text(
                "Command being timed: benchmark\n"
                "Maximum resident set size (kbytes): 12345\n",
                encoding="utf-8",
            )

            self.assertEqual(load_max_rss_kib(path), 12345)


if __name__ == "__main__":
    unittest.main()
