import tempfile
import unittest
from pathlib import Path

from configure_pyo3_ci import environment_signature, write_github_environment


class ConfigurePyo3CiTests(unittest.TestCase):
    def test_signature_changes_when_python_link_environment_changes(self):
        base = {
            "executable": "/opt/python/bin/python",
            "version": "3.13.15",
            "libdir": "/opt/python/lib",
            "ldlibrary": "libpython3.13.so",
        }

        self.assertEqual(environment_signature(base), environment_signature(dict(base)))
        changed = dict(base, ldlibrary="libpython3.13.so.1.0")
        self.assertNotEqual(environment_signature(base), environment_signature(changed))

    def test_writes_only_expected_environment_file_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "github-env"

            write_github_environment(
                path,
                {
                    "PYO3_PYTHON": "/opt/python/bin/python",
                    "PYO3_ENVIRONMENT_SIGNATURE": "abc123",
                },
            )

            self.assertEqual(
                path.read_text(encoding="utf-8"),
                "PYO3_PYTHON=/opt/python/bin/python\n"
                "PYO3_ENVIRONMENT_SIGNATURE=abc123\n",
            )

    def test_rejects_multiline_environment_values(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "github-env"

            with self.assertRaises(ValueError):
                write_github_environment(path, {"PYO3_PYTHON": "bad\nvalue"})


if __name__ == "__main__":
    unittest.main()
