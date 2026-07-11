import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from compare_performance import Measurement, load_measurements, render_summary


class ComparePerformanceTests(unittest.TestCase):
    def test_load_distinguishes_only_raw_channel_sensitive_cases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = {
                "raw_csv_channels": 4,
                "results": [
                    {
                        "name": "raw_waveform_read",
                        "median_seconds": 2.0,
                        "samples_per_second": 5.0,
                    },
                    {
                        "name": "lockin_w1",
                        "median_seconds": 1.0,
                        "samples_per_second": 10.0,
                    },
                ],
            }
            (root / "results.json").write_text(json.dumps(report), encoding="utf-8")

            measurements = load_measurements(root)

            self.assertIn(("raw_waveform_read", 4), measurements)
            self.assertIn(("lockin_w1", 0), measurements)

    def test_summary_warns_without_failing_on_large_regression(self):
        current = {
            ("lockin_w1", 0): Measurement("lockin_w1", 0, 1.5, 10.0),
        }
        previous = {
            ("lockin_w1", 0): Measurement("lockin_w1", 0, 1.0, 15.0),
        }
        warnings = io.StringIO()

        with contextlib.redirect_stdout(warnings):
            summary = render_summary(current, previous, 30.0)

        self.assertIn("+50.0%", summary)
        self.assertIn("::warning title=Performance regression::", warnings.getvalue())


if __name__ == "__main__":
    unittest.main()
