#!/usr/bin/env python3
"""Configure PyO3 for the Python selected by GitHub Actions.

The Rust cache can preserve PyO3's absolute ``libpython`` path across Python
patch-version updates.  Exporting a signature of the selected interpreter and
its link configuration makes PyO3 rerun its build script when that happens.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import sys
import sysconfig
from pathlib import Path


def python_environment_metadata() -> dict[str, str]:
    """Return Python details which affect PyO3's native link configuration."""

    config = sysconfig.get_config_vars()
    return {
        "executable": os.path.realpath(sys.executable),
        "implementation": platform.python_implementation(),
        "version": platform.python_version(),
        "prefix": os.path.realpath(sys.prefix),
        "base_prefix": os.path.realpath(sys.base_prefix),
        "libdir": str(config.get("LIBDIR") or ""),
        "ldlibrary": str(config.get("LDLIBRARY") or ""),
        "soabi": str(config.get("SOABI") or ""),
        "multiarch": str(config.get("MULTIARCH") or ""),
        "abiflags": str(config.get("ABIFLAGS") or ""),
    }


def environment_signature(metadata: dict[str, str]) -> str:
    """Create a stable, opaque signature for the selected Python environment."""

    payload = json.dumps(metadata, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def write_github_environment(path: Path, variables: dict[str, str]) -> None:
    """Append environment variables in GitHub Actions' environment-file format."""

    if any("\n" in value or "\r" in value for value in variables.values()):
        raise ValueError("GitHub environment values must not contain newlines")

    with path.open("a", encoding="utf-8", newline="\n") as environment_file:
        for name, value in variables.items():
            environment_file.write(f"{name}={value}\n")


def main() -> None:
    github_environment = os.environ.get("GITHUB_ENV")
    if not github_environment:
        raise SystemExit("GITHUB_ENV must be set by GitHub Actions")

    metadata = python_environment_metadata()
    signature = environment_signature(metadata)
    write_github_environment(
        Path(github_environment),
        {
            "PYO3_PYTHON": metadata["executable"],
            "PYO3_ENVIRONMENT_SIGNATURE": signature,
        },
    )

    print(
        "Configured PyO3 for "
        f"{metadata['executable']} ({metadata['implementation']} {metadata['version']}); "
        f"library={metadata['libdir']}/{metadata['ldlibrary']}"
    )


if __name__ == "__main__":
    main()
