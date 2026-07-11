import unittest

from benchmark_plot import benchmark, summarize


class PlotBenchmarkTests(unittest.TestCase):
    def test_summary_records_median_mad_and_p95(self):
        result = summarize("case", 10, [1.0, 2.0, 10.0])

        self.assertEqual(result["median_seconds"], 2.0)
        self.assertEqual(result["mad_seconds"], 1.0)
        self.assertEqual(result["p95_seconds"], 10.0)

    def test_smoke_benchmark_uses_agg_and_reports_all_draw_paths(self):
        report = benchmark([100], 1)
        names = {result["name"] for result in report["results"]}

        self.assertEqual(report["backend"].lower(), "agg")
        self.assertIn("matplotlib_figure_init", names)
        self.assertIn("matplotlib_full_draw_100", names)
        self.assertIn("matplotlib_set_data_draw_idle_100", names)
        self.assertIn("matplotlib_blit_100", names)


if __name__ == "__main__":
    unittest.main()
