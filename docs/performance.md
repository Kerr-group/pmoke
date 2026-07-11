# Performance

pmoke tracks elapsed time and peak memory separately. Shared CI runners are
useful for trends, but their timing is not a release gate.

Run the deterministic synthetic workload locally:

```console
cargo bench --locked --no-default-features --bench performance
```

Select a workload size and save the machine-readable report:

```console
cargo bench --locked --no-default-features --bench performance -- \
  --case raw_to_csv --samples 10000000 --channels 4 --iterations 5 \
  --output results.json
```

Available cases are `raw_word_decode`, `raw_waveform_read`, `sensor_integral`,
`lockin_w1`, `lockin_w2`, `python_copy`, `raw_to_csv`, and
`analysis_pipeline`. Omitting `--case` runs all microbenchmarks except
`analysis_pipeline`.

Each case performs one untimed validation run before measurement. RAW read
timings therefore use a warmed filesystem page cache and are intended to track
metadata validation, decoding, allocation, and cached-I/O regressions rather
than physical cold-storage throughput.

For peak resident memory on Linux, run one case per process and wrap the
command with `/usr/bin/time -v`.
On macOS, use `/usr/bin/time -l`. The weekly `Performance` workflow stores the
JSON report, resource usage, commit, toolchain, CPU, OS, and architecture as an
artifact. The 50-million-sample workload is reserved for a manual or
self-hosted run.

The workflow compares median time and peak RSS with the previous successful
run when the case, sample count, and RAW channel count match. Changes above 30%
are reported as informational warnings; shared-runner measurements do not gate
the workflow.

The benchmark contains deterministic RAW WORD decoding, sensor integration,
`boxcar_legacy` lock-in with one and two workers, and Rust-to-NumPy copy
workloads. `--channels` controls the RAW-to-CSV workload; weekly CI runs both
two- and four-channel cases. The JSON report also records the bytes and
cumulative time copied across the Python boundary.
The weekly workflow additionally runs the in-memory analysis pipeline from
reference fitting through Kerr with plotting disabled. It intentionally
excludes CLI startup, config parsing, RAW file I/O, and WORD decoding; those
boundaries are measured separately. This case requires NumPy, SciPy, lmfit,
and gsplot.
Expected numerical behavior remains in the regular golden and unit tests;
benchmark timing alone never defines correctness.
